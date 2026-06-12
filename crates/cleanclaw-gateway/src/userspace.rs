//! Per-user space — the gateway's unit of user isolation.
//!
//! # Role in the Architecture
//!
//! In the Go daemon, each user gets a `UserSpace` lazily constructed the
//! first time a message routes to them. The space caches the user's agent
//! manager, channel manager, plugin manager, workspace store handle, and
//! resolved provider scope. Admin writes invalidate the cached entry so a
//! subsequent inbound rebuilds from the store.
//!
//! This Rust port provides a smaller, offline-friendly equivalent:
//!
//! - A `UserSpace` struct holding `Arc`'d subsystem handles — the
//!   constructors of those subsystems live in their own crates and are
//!   plugged in by the higher-level `cleanclaw` binary.
//! - A `UserSpaceCache` keyed by `user_id` with `get_or_create(user)`,
//!   `invalidate(user)`, and `invalidate_all()` access patterns that
//!   mirror the Go cache semantics.
//! - An LRU-ish capacity backstop to prevent unbounded memory growth in
//!   long-lived processes.
//!
//! # Relationship with `userspace_loader`
//!
//! The module `userspace_loader` handles *construction* of a fully
//! populated `UserSpace` from the database (listing agents, building
//! bindings, attaching sandbox pools). This module handles *caching*
//! and *lifecycle* — it owns the cache data structure, the factory
//! wiring, and the invalidation hooks. The separation keeps
//! construction complexity out of the cache layer.
//!
//! # Concurrency Model
//!
//! The cache is guarded by a `tokio::sync::Mutex` so async handlers
//! (channel callbacks, webhook endpoints) can call into it without
//! blocking the runtime. The mutex is held for short durations —
//! HashMap operations plus an optional factory call. The factory
//! itself may perform I/O (database reads, file ops), so holding
//! the lock across the factory call is intentional: it serializes
//! construction for the same user_id, preventing duplicate loads.
//!
//! # Key Design Decisions
//!
//! 1. **Factory is caller-supplied** — The cache doesn't know how to
//!    build a `UserSpace`; it delegates to a `SpaceFactory` injected
//!    at construction time. This lets tests use lightweight factories
//!    while production uses database-backed ones.
//! 2. **Capacity is a backstop, not a policy** — The cache has a
//!    configurable maximum entry count, but the primary eviction
//!    mechanism is admin-driven invalidation. The capacity cap
//!    (default 1024) prevents runaway memory in edge cases.
//! 3. **Subsystem handles are `Arc<dyn Any>`** — Rather than depending
//!    on concrete types from other crates, the space stores typed
//!    erased handles. The binary that wires everything together knows
//!    the concrete types and downcasts them at the integration point.
//! 4. **Stats are monotonic counters** — Hits, misses, and evictions
//!    are cumulative. They don't reset on `invalidate_all` so
//!    observability dashboards can show long-term cache efficiency.

use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// UserSpace — per-user runtime environment
// ---------------------------------------------------------------------------

/// Per-user owned subsystem handles.
///
/// This struct is the "box" that holds everything a single user needs at
/// runtime: their LLM provider scope, agent manager, channel adapters,
/// plugin subprocesses, and workspace handle. The fields are all `Option`
/// so the struct can represent users at different levels of configuration
/// (e.g., a user with no workspace, or a user who only uses the web
/// channel).
///
/// # Why `Arc<dyn Any>`?
///
/// The subsystem types live in separate crates (`cleanclaw-agent`,
/// `cleanclaw-channels`, `cleanclaw-plugin`, etc.). To avoid a circular
/// dependency chain, the gateway crate stores them as type-erased
/// `Arc<dyn Any>` handles. The higher-level binary that wires everything
/// together knows the concrete types and performs the downcast at the
/// integration boundary. This pattern is common in plugin architectures
/// and service locators where the registry layer shouldn't depend on
/// every implementation.
///
/// # Field Overview
///
/// | Field | Populated by | When `None` |
/// |-------|-------------|-------------|
/// | `provider_scope` | Boot path (system→user→agent resolution) | User not configured for LLM access |
/// | `agent_manager` | `load_user_space` or `ensure_agent` | No agents created yet |
/// | `channel_manager` | Channel adapter factory | No IM channels configured |
/// | `plugin_manager` | Plugin launcher | No plugins installed |
/// | `workspace` | Workspace store factory | No workspace configured |
///
/// # Comparison with `userspace_loader::UserSpace`
///
/// This `UserSpace` (the cache entry) is a *superset* of the loader's
/// `UserSpace`. The loader's version carries only what the dispatch
/// path needs (agent manager, bindings, sandbox pool). This version
/// carries everything the gateway might hand to a user-facing handler
/// (channels, plugins, workspace, provider scope). In production,
/// the binary constructs both and stores the fuller one in the cache.
#[derive(Default, Clone)]
pub struct UserSpace {
    /// The user's internal ID (e.g., `"u_abc123"`). Set at construction
    /// time by the factory and never changes. Used as the cache key.
    pub user_id: String,

    /// Resolved provider scope (system → user → agent) for this user.
    ///
    /// This represents the final LLM provider configuration after all
    /// resolution layers have been applied:
    /// 1. System defaults (from global config)
    /// 2. User overrides (from the user's config rows)
    /// 3. Agent overrides (from the agent's config rows)
    ///
    /// Populated by the boot path during `UserSpace` construction.
    /// Left as `None` for callers that only need agent or channel
    /// access (e.g., a user who only manages channel bots without
    /// running LLM agents).
    pub provider_scope: Option<Arc<dyn std::any::Any + Send + Sync>>,

    /// Per-user agent manager.
    ///
    /// Owns the in-memory agent state for this user: loaded agents,
    /// active conversations, pending turns, and per-chat task queues.
    /// The runtime calls into it on every inbound message to resolve
    /// the target agent and enqueue a turn.
    ///
    /// Populated by `load_user_space` from the database (listing the
    /// user's agents) or by `ensure_agent` (lazy-attaching a foreign
    /// agent for cross-user access).
    pub agent_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,

    /// Per-user channel manager.
    ///
    /// Owns the live `Channel` adapters for the user's IM bots
    /// (Telegram, Discord, Slack, Feishu, WeChat, LINE, Web).
    /// Each adapter holds a connection to the IM platform's API
    /// and sends/receives messages on behalf of the user's bot.
    ///
    /// Populated by the channel factory in the boot path, using the
    /// channel configuration rows from the database.
    pub channel_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,

    /// Per-user plugin manager.
    ///
    /// Owns the running `Subprocess` set for the user's installed
    /// plugins. Plugins extend agent capabilities with custom tools,
    /// hooks, or provider integrations.
    ///
    /// Populated by the plugin launcher during boot. Each plugin
    /// runs as a separate subprocess with stdin/stdout JSON-RPC
    /// communication.
    pub plugin_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,

    /// Per-user workspace store handle.
    ///
    /// Provides file storage for the user's workspace: uploaded
    /// documents, generated artifacts, session transcripts, and
    /// sandbox state. Backed by the filesystem (local or networked).
    ///
    /// `None` when the user has no workspace configured — they can
    /// still chat with agents but can't persist files.
    pub workspace: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

impl UserSpace {
    /// Create a blank `UserSpace` with only a `user_id` set.
    ///
    /// This is the minimal valid state. In production, the factory
    /// calls this and then populates the optional fields by calling
    /// the subsystem constructors. In tests, this is often used
    /// directly to verify cache mechanics without heavy subsystems.
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            ..Default::default()
        }
    }

    /// Whether the space is the empty default — no subsystems wired.
    ///
    /// Returns `true` when every optional field is `None`. This is
    /// useful in two contexts:
    ///
    /// 1. **Tests** — Verifying that a factory-produced space actually
    ///    has subsystems attached (the `factory_with_subsystems_round_trip`
    ///    test uses `!is_empty()` as the success assertion).
    ///
    /// 2. **Observability** — `cache.stats()` callers can check whether
    ///    cached entries are fully populated or still in their default
    ///    state, which indicates "this user has never had a message
    ///    routed to them yet".
    pub fn is_empty(&self) -> bool {
        self.provider_scope.is_none()
            && self.agent_manager.is_none()
            && self.channel_manager.is_none()
            && self.plugin_manager.is_none()
            && self.workspace.is_none()
    }
}

// ---------------------------------------------------------------------------
// SpaceFactory — dependency injection for cache population
// ---------------------------------------------------------------------------

/// Construct a fresh `UserSpace` for a given `user_id`.
///
/// # Why a Factory?
///
/// The cache doesn't know how to build a `UserSpace`. It only knows
/// how to store and retrieve them. The factory pattern decouples
/// cache logic from construction logic:
///
/// - **Production**: The factory calls into `userspace_loader` (or
///   the equivalent boot path) to build a fully populated space from
///   the database.
/// - **Tests**: The factory returns a lightweight `UserSpace::new(uid)`
///   with no subsystems, keeping tests fast and focused on cache
///   behavior.
///
/// The factory is an `Arc<dyn Fn>` so it can be shared across
/// threads/tasks without lifetime issues. The `Send + Sync` bound
/// ensures it's safe to call from any async context.
///
/// # Factory Call Semantics
///
/// The factory is called **while holding the cache's mutex lock**.
/// This means:
/// - Concurrent `get_or_create` calls for *different* users are
///   serialized — only one factory runs at a time.
/// - Concurrent calls for the *same* user are guaranteed to see the
///   first caller's result (the second caller is a cache hit).
/// - The factory may perform I/O (database queries, file operations)
///   because tokio's Mutex doesn't block the runtime — it yields.
pub type SpaceFactory = Arc<dyn Fn(&str) -> UserSpace + Send + Sync>;

// ---------------------------------------------------------------------------
// UserSpaceCache — the central cache data structure
// ---------------------------------------------------------------------------

/// In-process cache of `UserSpace` per `user_id`.
///
/// # Design
///
/// The cache is a `HashMap<String, Arc<UserSpace>>` wrapped in a
/// `tokio::sync::Mutex`. The mutex protects the entire inner state
/// (entries map + stats counters), ensuring atomic updates across
/// the cache-hit, cache-miss, and eviction paths.
///
/// # Thread Safety
///
/// Using `tokio::sync::Mutex` instead of `std::sync::Mutex` means
/// the lock is async-friendly: if the factory performs I/O (database
/// reads, file operations), the lock holder yields to the tokio
/// runtime rather than blocking a thread. This is critical because
/// cache misses happen on the request path and a blocking mutex
/// would stall all concurrent requests.
///
/// If the caller needs `&mut self` ergonomics, `parking_lot::Mutex`
/// is a valid alternative — the boot path can lock once, populate,
/// then drop.
///
/// # Capacity Management
///
/// The cache has a configurable `max_entries` (default 1024).
/// When the limit is reached, the next insert evicts an arbitrary
/// entry — this is LRU-*ish*, not true LRU. Real callers should
/// rely on admin-driven `invalidate` and `invalidate_all` for
/// cache management; the capacity cap is a safety backstop.
///
/// # Stats
///
/// The cache tracks cumulative counters:
/// - `hits`: Successfully served from cache
/// - `misses`: Required factory call (counts even if factory fails)
/// - `evictions`: Entries removed by capacity enforcement
///
/// These counters are monotonic — they never reset, even across
/// `invalidate_all()`. This makes them useful for long-running
/// observability (cache hit rate over process lifetime).
pub struct UserSpaceCache {
    /// The mutable inner state (entries + stats). Protected by a
    /// tokio Mutex so async callers don't block the runtime.
    inner: Mutex<CacheState>,

    /// The factory function used to build new `UserSpace` instances
    /// on cache misses. Injected at construction time.
    factory: SpaceFactory,

    /// Maximum number of cached entries before eviction kicks in.
    /// Default: 1024. Minimum effective value: 1.
    max_entries: usize,
}

/// Internal mutable state of the cache.
///
/// Bundled into a single struct so the mutex lock covers both the
/// entries map and the stats counters atomically. This prevents
/// races between concurrent `get_or_create` calls that would
/// otherwise see stale stats.
struct CacheState {
    /// The cached entries, keyed by user_id. Each value is an
    /// `Arc<UserSpace>` so it can be cheaply cloned for callers.
    entries: std::collections::HashMap<String, Arc<UserSpace>>,

    /// Cumulative count of successful cache lookups.
    hits: u64,

    /// Cumulative count of cache misses (factory calls).
    misses: u64,

    /// Cumulative count of entries evicted due to capacity limits.
    evictions: u64,
}

impl std::fmt::Debug for UserSpaceCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only expose config fields in Debug — the entries map and
        // stats require a lock to read, and Debug shouldn't acquire
        // locks (it's often called from panic handlers). Use
        // `stats()` for programmatic access to cache state.
        f.debug_struct("UserSpaceCache")
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

/// A point-in-time snapshot of cache statistics.
///
/// Returned by `UserSpaceCache::stats()`. All fields are cumulative
/// counters, not rates — callers that want rates (e.g., hit rate
/// over the last minute) should sample `stats()` periodically and
/// compute deltas.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    /// Current number of cached entries.
    pub entries: usize,
    /// Total cache hits since process start.
    pub hits: u64,
    /// Total cache misses since process start.
    pub misses: u64,
    /// Total capacity-triggered evictions since process start.
    pub evictions: u64,
}

impl UserSpaceCache {
    // -------------------------------------------------------------------
    // Construction
    // -------------------------------------------------------------------

    /// Create a new cache with the default capacity of 1024 entries.
    ///
    /// This is the common path — 1024 users is well beyond typical
    /// self-hosted deployments, and the capacity backstop prevents
    /// memory growth in edge cases (e.g., a botnet of fake users).
    pub fn new(factory: SpaceFactory) -> Self {
        Self::with_capacity(factory, 1024)
    }

    /// Create a new cache with a custom capacity.
    ///
    /// The `max_entries` parameter is clamped to a minimum of 1 —
    /// a cache with capacity 0 would reject all writes, defeating
    /// its purpose. The minimum of 1 ensures at least one entry
    /// can be stored.
    ///
    /// # Test Usage
    ///
    /// Tests use small capacities (e.g., 2) to trigger eviction
    /// behavior with only a few inserts. Production would typically
    /// use the default or a larger value.
    pub fn with_capacity(factory: SpaceFactory, max_entries: usize) -> Self {
        Self {
            inner: Mutex::new(CacheState {
                entries: std::collections::HashMap::new(),
                hits: 0,
                misses: 0,
                evictions: 0,
            }),
            factory,
            max_entries: max_entries.max(1),
        }
    }

    // -------------------------------------------------------------------
    // Core operations
    // -------------------------------------------------------------------

    /// Get an existing `UserSpace` for `user_id`, or build and insert
    /// a fresh one via the factory.
    ///
    /// # Behavior
    ///
    /// 1. **Cache hit** — Returns the cached `Arc<UserSpace>` directly.
    ///    Increments `hits`. The returned `Arc` can be cloned cheaply.
    /// 2. **Cache miss** — Increments `misses`. If the cache is at
    ///    capacity, evicts an arbitrary entry (increments `evictions`).
    ///    Calls the factory to build a fresh `UserSpace`, inserts it,
    ///    and returns it.
    ///
    /// # Serialization
    ///
    /// The factory call happens *while holding the mutex lock*. This
    /// ensures that two concurrent `get_or_create("u1")` calls don't
    /// both run the factory — the second one will see the first one's
    /// cached result. The trade-off is that a slow factory (e.g.,
    /// database query for user `X`) blocks all other cache operations
    /// for its duration. In practice, factory calls are fast (a few
    /// milliseconds) and the benefit of deduplication outweighs the
    /// serialization cost.
    ///
    /// # Return Value
    ///
    /// Always returns an `Arc<UserSpace>`. On a cache hit it's the
    /// existing entry; on a miss it's the freshly constructed one.
    /// The caller does not need to check for `None`.
    pub async fn get_or_create(&self, user_id: &str) -> Arc<UserSpace> {
        let mut g = self.inner.lock().await;

        // Fast path: check the map. We clone the Arc out of the
        // immutable borrow first, then update stats afterwards.
        // This avoids a mutable borrow conflicting with the
        // immutable `get` — the clone is cheap (Arc ref-count bump).
        let existing = g.entries.get(user_id).cloned();
        if let Some(existing) = existing {
            g.hits += 1;
            return existing;
        }

        // Slow path: cache miss. Run the capacity gate, evict if
        // needed, then delegate to the factory.
        g.misses += 1;
        if g.entries.len() >= self.max_entries {
            // LRU-ish eviction: drop an arbitrary entry. A true LRU
            // would maintain a linked list or use an `IndexMap`, but
            // this cache is not the primary eviction mechanism —
            // admin-driven invalidation is. The capacity cap is a
            // safety backstop for edge cases (botnets, config errors,
            // long-running processes with unbounded user sets).
            //
            // We use `keys().next()` to pick a victim. HashMap
            // iteration order is non-deterministic, so this is
            // effectively random eviction, not true LRU.
            if let Some(victim) = g.entries.keys().next().cloned() {
                g.entries.remove(&victim);
                g.evictions += 1;
            }
        }

        // Build the space via the injected factory. This may perform
        // I/O (database queries, file reads). The factory is
        // caller-supplied so the cache itself has no knowledge of
        // how construction works — it just delegates.
        let space = Arc::new((self.factory)(user_id));
        g.entries.insert(user_id.to_string(), space.clone());
        space
    }

    // -------------------------------------------------------------------
    // Invalidation
    // -------------------------------------------------------------------

    /// Drop the cached entry for `user_id` if present.
    ///
    /// Called on admin writes that affect the user (reset password,
    /// role change, agent list update, channel config change) so the
    /// next `get_or_create` rebuilds from the database with fresh
    /// configuration.
    ///
    /// # Return Value
    ///
    /// - `true` if an entry existed and was removed
    /// - `false` if the user was not in the cache (caller may choose
    ///   to skip any follow-up invalidation work)
    ///
    /// # Idempotency
    ///
    /// Safe to call repeatedly — removing a non-existent key is a
    /// no-op on the cache (the HashMap handles it gracefully).
    /// Multiple admin writes for the same user may each trigger an
    /// invalidate; only the first one does actual work.
    pub async fn invalidate(&self, user_id: &str) -> bool {
        let mut g = self.inner.lock().await;
        g.entries.remove(user_id).is_some()
    }

    /// Drop every cached entry.
    ///
    /// Used by global configuration changes that affect all users:
    /// - Scope table reload (system prompt update)
    /// - Plugin update (all users get new plugin versions)
    /// - Global rate limit change
    /// - SIGHUP handler (operator signals config reload)
    ///
    /// # Return Value
    ///
    /// Returns the number of entries removed, which callers can log
    /// for observability (e.g., "reload: dropped 47 cached spaces").
    ///
    /// # Performance
    ///
    /// O(1) — `HashMap::clear` drops the internal table and allocates
    /// a fresh one. The old entries' `Arc` ref-counts are decremented
    /// as they drop; if no other references exist, the `UserSpace`
    /// and its subsystems are freed.
    pub async fn invalidate_all(&self) -> usize {
        let mut g = self.inner.lock().await;
        let n = g.entries.len();
        g.entries.clear();
        n
    }

    // -------------------------------------------------------------------
    // Introspection and stats
    // -------------------------------------------------------------------

    /// Return a point-in-time snapshot of cache statistics.
    ///
    /// Used by observability endpoints (health checks, metrics
    /// exporters, admin dashboards). The returned `CacheStats` is
    /// a cheap `Copy` type — callers can sample it periodically
    /// and compute deltas for rate calculations.
    ///
    /// # Example Rate Calculation
    ///
    /// ```text
    /// let before = cache.stats().await;
    /// sleep(Duration::from_secs(60)).await;
    /// let after = cache.stats().await;
    /// let hit_rate = (after.hits - before.hits) as f64
    ///     / ((after.hits - before.hits) + (after.misses - before.misses)) as f64;
    /// ```
    pub async fn stats(&self) -> CacheStats {
        let g = self.inner.lock().await;
        CacheStats {
            entries: g.entries.len(),
            hits: g.hits,
            misses: g.misses,
            evictions: g.evictions,
        }
    }

    /// Check whether `user_id` currently has a cached entry.
    ///
    /// This is a cheap check that acquires the lock, probes the
    /// HashMap, and releases. It does not affect stats counters
    /// (not a "hit" in the caching sense — it's an introspection
    /// operation, not a data-access operation).
    pub async fn contains(&self, user_id: &str) -> bool {
        self.inner.lock().await.entries.contains_key(user_id)
    }

    /// Return the current number of cached entries.
    ///
    /// Equivalent to `stats().entries` but cheaper — it only
    /// reads `entries.len()` without copying the counter fields.
    pub async fn len(&self) -> usize {
        self.inner.lock().await.entries.len()
    }

    /// Return `true` when the cache has no entries.
    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Helper: create a `SpaceFactory` that builds blank `UserSpace`
    /// instances and an `AtomicUsize` counter to track how many
    /// times the factory was called.
    ///
    /// Used by tests to verify cache-hit/miss behavior without
    /// heavyweight subsystem construction.
    fn factory_blank() -> (SpaceFactory, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        let f: SpaceFactory = Arc::new(move |uid: &str| {
            c2.fetch_add(1, Ordering::SeqCst);
            UserSpace::new(uid)
        });
        (f, counter)
    }

    /// Verify the basic get-or-create contract:
    ///
    /// 1. First call for a user is a miss — factory runs, space is
    ///    created and cached.
    /// 2. Second call for the same user is a hit — factory does NOT
    ///    run, the same `Arc` is returned (verified via pointer
    ///    equality).
    /// 3. Stats reflect one miss and one hit.
    #[tokio::test]
    async fn get_or_create_inserts_then_returns() {
        let (f, ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);

        // First access: cache miss, factory runs.
        let s1 = cache.get_or_create("u1").await;
        assert_eq!(s1.user_id, "u1");
        assert_eq!(ctr.load(Ordering::SeqCst), 1);

        // Second access: cache hit, same Arc returned.
        let s2 = cache.get_or_create("u1").await;
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(
            ctr.load(Ordering::SeqCst),
            1,
            "second hit must not call factory"
        );

        // Verify stats.
        let stats = cache.stats().await;
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    /// Verify the invalidate-then-rebuild flow:
    ///
    /// 1. Create and cache a space.
    /// 2. Invalidate it — entry is removed, `contains` returns false.
    /// 3. Next `get_or_create` is a miss — factory runs again.
    ///
    /// This models the admin-write-then-next-inbound pattern:
    /// admin changes a user's config, cache is invalidated, the
    /// next message from that user triggers a fresh load from DB.
    #[tokio::test]
    async fn invalidate_drops_entry_and_next_miss_rebuilds() {
        let (f, ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);

        cache.get_or_create("u1").await;
        assert_eq!(ctr.load(Ordering::SeqCst), 1);

        // Invalidate — the entry is gone.
        assert!(cache.invalidate("u1").await);
        assert!(!cache.contains("u1").await);

        // Re-access triggers a fresh factory call.
        cache.get_or_create("u1").await;
        assert_eq!(ctr.load(Ordering::SeqCst), 2, "factory must run again");
    }

    /// Verify that invalidating a non-existent user returns `false`
    /// (not an error, not a panic). This ensures admin APIs can
    /// blindly invalidate without checking cache membership first.
    #[tokio::test]
    async fn invalidate_missing_returns_false() {
        let (f, _ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);
        assert!(!cache.invalidate("never-seen").await);
    }

    /// Verify that `invalidate_all` returns the count of removed
    /// entries and leaves the cache empty.
    ///
    /// The return value is important for observability — operators
    /// want to see "reload: dropped 47 cached spaces" in logs.
    #[tokio::test]
    async fn invalidate_all_returns_count() {
        let (f, _ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);
        cache.get_or_create("u1").await;
        cache.get_or_create("u2").await;
        cache.get_or_create("u3").await;
        assert_eq!(cache.invalidate_all().await, 3);
        assert!(cache.is_empty().await);
    }

    /// Verify that the capacity backstop triggers eviction when
    /// entries exceed `max_entries`.
    ///
    /// With capacity 2, inserting a third user forces an eviction.
    /// The cache stays at size 2 and `stats.evictions` increments.
    ///
    /// Note: the victim is arbitrary (HashMap iteration order), so
    /// this test only checks the count, not which entry was evicted.
    #[tokio::test]
    async fn capacity_cap_evicts_oldest() {
        let (f, _ctr) = factory_blank();
        let cache = UserSpaceCache::with_capacity(f, 2);
        cache.get_or_create("u1").await;
        cache.get_or_create("u2").await;
        cache.get_or_create("u3").await;
        assert_eq!(cache.len().await, 2);
        let stats = cache.stats().await;
        assert_eq!(stats.evictions, 1);
    }

    /// Verify that `UserSpace::new()` produces an empty space
    /// and that `is_empty()` correctly reports it.
    #[tokio::test]
    async fn user_space_is_empty_for_blank() {
        let s = UserSpace::new("u1");
        assert!(s.is_empty());
        assert_eq!(s.user_id, "u1");
    }

    /// Verify that a factory producing a populated `UserSpace`
    /// (with an agent_manager attached) correctly flows through
    /// the cache: `get_or_create` returns a non-empty space.
    ///
    /// This test uses a dummy `Arc<i32>` as the agent_manager —
    /// the concrete type doesn't matter for cache mechanics.
    #[tokio::test]
    async fn factory_with_subsystems_round_trip() {
        let f: SpaceFactory = Arc::new(|uid: &str| {
            let mut s = UserSpace::new(uid);
            // Attach a dummy subsystem — any Send+Sync type works.
            let dummy: Arc<dyn std::any::Any + Send + Sync> = Arc::new(42_i32);
            s.agent_manager = Some(dummy);
            s
        });
        let cache = UserSpaceCache::new(f);
        let s = cache.get_or_create("u1").await;
        assert!(!s.is_empty());
        assert!(s.agent_manager.is_some());
    }

    /// Verify that `CacheStats::default()` starts at zero for all
    /// counters. This is used by tests that construct stats from
    /// scratch and by the `CacheState` initializer.
    #[tokio::test]
    async fn stats_default_zero() {
        let stats = CacheStats::default();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.evictions, 0);
    }

    /// Verify that capacity 0 is clamped to 1 — the cache must
    /// accept at least one entry. This guards against a footgun
    /// where passing `max_entries = 0` would create a cache that
    /// rejects all writes.
    ///
    /// The capacity cap is meant to prevent *unbounded* growth,
    /// not to reject all writes. A minimum of 1 preserves the
    /// backstop semantics without breaking basic functionality.
    #[tokio::test]
    async fn max_entries_at_least_one() {
        let (f, _ctr) = factory_blank();
        let cache = UserSpaceCache::with_capacity(f, 0);
        cache.get_or_create("u1").await;
        assert_eq!(cache.len().await, 1);
    }
}
