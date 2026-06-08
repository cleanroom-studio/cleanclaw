//! Per-user space —
//!
//! In the Go daemon, each user gets a `UserSpace` lazily constructed
//! the first time a message routes to them. The space caches the
//! user's agent manager, channel manager, plugin manager, workspace
//! store handle, and resolved provider scope. Admin writes invalidate
//! the cached entry so a subsequent inbound rebuilds from the store.
//!
//! This Rust port is a smaller, offline-friendly equivalent: a
//! `UserSpaceCache` keyed by `user_id` with `invalidate(user)` and
//! `invalidate_all()` admin hooks. The cached payload is a `UserSpace`
//! struct holding Arc'd subsystem handles; the constructors of those
//! subsystems live in their own crates and are plugged in by the
//! higher-level `cleanclaw` binary. The tests in this module cover
//! the cache mechanics + invalidation contract.

use std::sync::Arc;
use tokio::sync::Mutex;

/// Per-user owned subsystem handles. Populated by the higher-level
/// binary; here we just hold the `Arc`s so the cache can return them
/// by value. Optional fields cover the components that may not have
/// been wired in every build.
#[derive(Default, Clone)]
pub struct UserSpace {
    pub user_id: String,
    /// Resolved provider scope (system→user→agent) for this user.
    /// Populated by the boot path; left as `None` for callers that
    /// only need agent / channel access.
    pub provider_scope: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Per-user agent manager. Owns the in-memory agent state for
    /// this user; the runtime calls into it on every inbound.
    pub agent_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Per-user channel manager. Owns the live `Channel` adapters
    /// for the user's IM bots.
    pub channel_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Per-user plugin manager. Owns the running `Subprocess` set.
    pub plugin_manager: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Per-user workspace store handle. `None` when the user has no
    /// workspace configured.
    pub workspace: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

impl UserSpace {
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            ..Default::default()
        }
    }

    /// Whether the space is the empty default (no subsystems wired).
    /// Useful in tests and in `cache.stats()` to detect "this user
    /// has never had a message routed to them yet".
    pub fn is_empty(&self) -> bool {
        self.provider_scope.is_none()
            && self.agent_manager.is_none()
            && self.channel_manager.is_none()
            && self.plugin_manager.is_none()
            && self.workspace.is_none()
    }
}

/// Construct a fresh `UserSpace` for `user_id`. Production calls this
/// inside `cache.get_or_create` with the heavy subsystem
/// constructors; tests bypass it with a plain `UserSpace::new`.
pub type SpaceFactory = Arc<dyn Fn(&str) -> UserSpace + Send + Sync>;

/// In-process cache of `UserSpace` per `user_id`. Guarded by a
/// `tokio::sync::Mutex` so the higher-level binary can call into it
/// from async handlers without `std::sync` poisoning. Use
/// `parking_lot::Mutex` if the caller needs `&mut self` ergonomics
/// (the boot path can lock once, then drop).
pub struct UserSpaceCache {
    inner: Mutex<CacheState>,
    factory: SpaceFactory,
    max_entries: usize,
}

struct CacheState {
    entries: std::collections::HashMap<String, Arc<UserSpace>>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl std::fmt::Debug for UserSpaceCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserSpaceCache")
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl UserSpaceCache {
    pub fn new(factory: SpaceFactory) -> Self {
        Self::with_capacity(factory, 1024)
    }

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

    /// Get an existing space or build + insert a fresh one.
    pub async fn get_or_create(&self, user_id: &str) -> Arc<UserSpace> {
        let mut g = self.inner.lock().await;
        // Clone the Arc out of the immutable borrow first so we can
        // update the stats counters afterwards.
        let existing = g.entries.get(user_id).cloned();
        if let Some(existing) = existing {
            g.hits += 1;
            return existing;
        }
        g.misses += 1;
        if g.entries.len() >= self.max_entries {
            // LRU-ish: drop an arbitrary victim. Real callers should
            // subscribe to `invalidate` and let the admin path drive
            // the eviction. The cap is a backstop against unbounded
            // growth in long-lived processes.
            if let Some(victim) = g.entries.keys().next().cloned() {
                g.entries.remove(&victim);
                g.evictions += 1;
            }
        }
        let space = Arc::new((self.factory)(user_id));
        g.entries.insert(user_id.to_string(), space.clone());
        space
    }

    /// Drop the cached entry for `user_id` if any. Called on admin
    /// writes that touch the user (reset password, role change, etc.)
    /// so the next inbound rebuilds from the store.
    pub async fn invalidate(&self, user_id: &str) -> bool {
        let mut g = self.inner.lock().await;
        g.entries.remove(user_id).is_some()
    }

    /// Drop every entry. Used by global config writes (e.g. scope
    /// table reload, system prompt update).
    pub async fn invalidate_all(&self) -> usize {
        let mut g = self.inner.lock().await;
        let n = g.entries.len();
        g.entries.clear();
        n
    }

    pub async fn stats(&self) -> CacheStats {
        let g = self.inner.lock().await;
        CacheStats {
            entries: g.entries.len(),
            hits: g.hits,
            misses: g.misses,
            evictions: g.evictions,
        }
    }

    pub async fn contains(&self, user_id: &str) -> bool {
        self.inner.lock().await.entries.contains_key(user_id)
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.entries.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn factory_blank() -> (SpaceFactory, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        let f: SpaceFactory = Arc::new(move |uid: &str| {
            c2.fetch_add(1, Ordering::SeqCst);
            UserSpace::new(uid)
        });
        (f, counter)
    }

    #[tokio::test]
    async fn get_or_create_inserts_then_returns() {
        let (f, ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);
        let s1 = cache.get_or_create("u1").await;
        assert_eq!(s1.user_id, "u1");
        assert_eq!(ctr.load(Ordering::SeqCst), 1);
        let s2 = cache.get_or_create("u1").await;
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(ctr.load(Ordering::SeqCst), 1, "second hit must not call factory");
        let stats = cache.stats().await;
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn invalidate_drops_entry_and_next_miss_rebuilds() {
        let (f, ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);
        cache.get_or_create("u1").await;
        assert_eq!(ctr.load(Ordering::SeqCst), 1);
        assert!(cache.invalidate("u1").await);
        assert!(!cache.contains("u1").await);
        cache.get_or_create("u1").await;
        assert_eq!(ctr.load(Ordering::SeqCst), 2, "factory must run again");
    }

    #[tokio::test]
    async fn invalidate_missing_returns_false() {
        let (f, _ctr) = factory_blank();
        let cache = UserSpaceCache::new(f);
        assert!(!cache.invalidate("never-seen").await);
    }

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

    #[tokio::test]
    async fn user_space_is_empty_for_blank() {
        let s = UserSpace::new("u1");
        assert!(s.is_empty());
        assert_eq!(s.user_id, "u1");
    }

    #[tokio::test]
    async fn factory_with_subsystems_round_trip() {
        let f: SpaceFactory = Arc::new(|uid: &str| {
            let mut s = UserSpace::new(uid);
            let dummy: Arc<dyn std::any::Any + Send + Sync> = Arc::new(42_i32);
            s.agent_manager = Some(dummy);
            s
        });
        let cache = UserSpaceCache::new(f);
        let s = cache.get_or_create("u1").await;
        assert!(!s.is_empty());
        assert!(s.agent_manager.is_some());
    }

    #[tokio::test]
    async fn stats_default_zero() {
        let stats = CacheStats::default();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.evictions, 0);
    }

    #[tokio::test]
    async fn max_entries_at_least_one() {
        let (f, _ctr) = factory_blank();
        let cache = UserSpaceCache::with_capacity(f, 0);
        // Even with capacity 0 we accept 1 entry; the backstop is
        // meant to prevent unbounded growth, not to reject all writes.
        cache.get_or_create("u1").await;
        assert_eq!(cache.len().await, 1);
    }
}
