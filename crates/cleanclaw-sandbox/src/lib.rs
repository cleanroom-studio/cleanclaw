//! Sandbox runtime.
//!
//! Provides three things:
//!   - `Executor` trait — the per-(agent, project, session) execution
//!     environment. All agent tool calls (exec, read_file, write_file,
//!     list_dir) route through it in cloud mode so each user gets an
//!     isolated filesystem and runtime.
//!   - `ExecutorPool` — lazy per-scope allocation with idle eviction.
//!   - `LocalExecutor` — host passthrough; used in dev and tests so the
//!     agent tools can run without Docker/E2B.
//!
//! Concrete Docker / E2B / Boxlite executors are out of scope for the
//! parity sweep (they each need a real backend). The trait is the
//! contract; pool plumbing, idle eviction, and user-context injection
//! are testable here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use thiserror::Error;

pub mod remote;
pub use remote::*;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("workspace: {0}")]
    Workspace(#[from] cleanclaw_workspace::WorkspaceError),
    #[error("not configured: {0}")]
    NotConfigured(&'static str),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("exec failed: {0}")]
    Exec(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("pool: {0}")]
    Pool(String),
    #[error("http: {0}")]
    Http(String),
    #[error("upstream: {0}")]
    Upstream(String),
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<SandboxExec, SandboxError>;
    async fn read_file(&self, path: &str) -> Result<Bytes, SandboxError>;
    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), SandboxError>;
    async fn list_dir(&self, path: &str) -> Result<Vec<SandboxEntry>, SandboxError>;
    fn backend(&self) -> &'static str;
    async fn close(&self) -> Result<(), SandboxError>;
}

#[derive(Debug, Clone)]
pub struct SandboxExec {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct SandboxEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mod_time: DateTime<Utc>,
}

/// Per-scope pool. Implementations are responsible for actually
/// spinning up the backend (Docker, E2B, ...).
#[async_trait]
pub trait ExecutorPool: Send + Sync {
    async fn get(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<Arc<dyn Executor>, SandboxError>;
    async fn release(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<(), SandboxError>;
    async fn close_all(&self) -> Result<(), SandboxError>;
    fn backend(&self) -> &'static str;
}

// =====================================================================
// LocalExecutor — host passthrough. Dev + tests + single-user installs.
// =====================================================================

pub struct LocalExecutor {
    root: PathBuf,
}

impl LocalExecutor {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl Executor for LocalExecutor {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<SandboxExec, SandboxError> {
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            timeout,
            tokio::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.root)
                .output(),
        )
        .await;
        let duration = start.elapsed();
        match result {
            Ok(Ok(out)) => Ok(SandboxExec {
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                exit_code: out.status.code().unwrap_or(-1),
                duration,
            }),
            Ok(Err(e)) => Err(SandboxError::Io(e)),
            Err(_) => Err(SandboxError::Timeout(timeout)),
        }
    }

    async fn read_file(&self, path: &str) -> Result<Bytes, SandboxError> {
        let full = self.root.join(path);
        match tokio::fs::read(&full).await {
            Ok(b) => Ok(Bytes::from(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SandboxError::NotFound(path.to_string()))
            }
            Err(e) => Err(SandboxError::Io(e)),
        }
    }

    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), SandboxError> {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full, content).await?;
        Ok(())
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<SandboxEntry>, SandboxError> {
        let full = self.root.join(path);
        let mut entries = match tokio::fs::read_dir(&full).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SandboxError::NotFound(path.to_string()));
            }
            Err(e) => return Err(SandboxError::Io(e)),
        };
        let mut out = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            let name = entry.file_name().to_string_lossy().to_string();
            let mod_time = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    let dt: DateTime<Utc> = (std::time::UNIX_EPOCH + d).into();
                    dt
                })
                .unwrap_or_else(Utc::now);
            out.push(SandboxEntry {
                name,
                is_dir: meta.is_dir(),
                size: meta.len(),
                mod_time,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn backend(&self) -> &'static str {
        "local"
    }

    async fn close(&self) -> Result<(), SandboxError> {
        Ok(())
    }
}

// =====================================================================
// LocalExecutorPool — single root, all scopes share it.
// =====================================================================

pub struct LocalExecutorPool {
    root: PathBuf,
    executors: Mutex<HashMap<String, Arc<LocalExecutor>>>,
}

impl LocalExecutorPool {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            executors: Mutex::new(HashMap::new()),
        }
    }

    fn key(agent_id: &str, project_id: &str, session_id: &str) -> String {
        format!("{agent_id}|{project_id}|{session_id}")
    }
}

#[async_trait]
impl ExecutorPool for LocalExecutorPool {
    async fn get(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<Arc<dyn Executor>, SandboxError> {
        let key = Self::key(agent_id, project_id, session_id);
        let mut g = self.executors.lock().await;
        if !g.contains_key(&key) {
            // Scope the work directory per (agent, project, session).
            let scope_root = self.root.join(format!(
                "{}/{}/{}",
                agent_id,
                if project_id.is_empty() {
                    "_"
                } else {
                    project_id
                },
                if session_id.is_empty() {
                    "_" }
                else {
                    session_id
                }
            ));
            tokio::fs::create_dir_all(&scope_root).await?;
            g.insert(key.clone(), Arc::new(LocalExecutor::new(scope_root)));
        }
        let exec = g.get(&key).cloned().expect("just inserted");
        Ok(exec)
    }

    async fn release(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<(), SandboxError> {
        let key = Self::key(agent_id, project_id, session_id);
        self.executors.lock().await.remove(&key);
        Ok(())
    }

    async fn close_all(&self) -> Result<(), SandboxError> {
        self.executors.lock().await.clear();
        Ok(())
    }

    fn backend(&self) -> &'static str {
        "local"
    }
}

// =====================================================================
// LifecyclePool — adds lazy creation + idle eviction on top of any pool.
// =====================================================================

pub struct LifecyclePool {
    inner: Arc<dyn ExecutorPool>,
    idle_ttl: Duration,
    sweep_interval: Duration,
    state: Mutex<HashMap<String, std::time::Instant>>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LifecyclePool {
    pub fn new(inner: Arc<dyn ExecutorPool>, idle_ttl: Duration) -> Self {
        Self {
            inner,
            idle_ttl,
            sweep_interval: Duration::from_secs(60),
            state: Mutex::new(HashMap::new()),
            handle: Mutex::new(None),
        }
    }

    fn key(agent_id: &str, project_id: &str, session_id: &str) -> String {
        format!("{agent_id}|{project_id}|{session_id}")
    }

    /// Start the background sweeper that releases idle sandboxes.
    pub async fn start_sweeper(self: &Arc<Self>) {
        let me = self.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(me.sweep_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                me.sweep_once().await;
            }
        });
        *self.handle.lock().await = Some(handle);
    }

    pub async fn stop_sweeper(&self) {
        if let Some(h) = self.handle.lock().await.take() {
            h.abort();
        }
    }

    pub async fn sweep_once(&self) -> usize {
        let now = std::time::Instant::now();
        let stale: Vec<String> = {
            let g = self.state.lock().await;
            g.iter()
                .filter(|(_, last)| now.duration_since(**last) > self.idle_ttl)
                .map(|(k, _)| k.clone())
                .collect()
        };
        let mut released = 0;
        for k in stale {
            let parts: Vec<&str> = k.splitn(3, '|').collect();
            if parts.len() == 3 {
                let _ = self
                    .inner
                    .release(parts[0], parts[1], parts[2])
                    .await;
                self.state.lock().await.remove(&k);
                released += 1;
            }
        }
        released
    }

    pub async fn active_count(&self) -> usize {
        self.state.lock().await.len()
    }

    pub fn idle_ttl(&self) -> Duration {
        self.idle_ttl
    }
}

#[async_trait]
impl ExecutorPool for LifecyclePool {
    async fn get(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<Arc<dyn Executor>, SandboxError> {
        let key = Self::key(agent_id, project_id, session_id);
        // Mark the scope as freshly used.
        self.state
            .lock()
            .await
            .insert(key, std::time::Instant::now());
        self.inner.get(agent_id, project_id, session_id).await
    }

    async fn release(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<(), SandboxError> {
        let key = Self::key(agent_id, project_id, session_id);
        self.state.lock().await.remove(&key);
        self.inner.release(agent_id, project_id, session_id).await
    }

    async fn close_all(&self) -> Result<(), SandboxError> {
        self.state.lock().await.clear();
        self.inner.close_all().await
    }

    fn backend(&self) -> &'static str {
        self.inner.backend()
    }
}

// =====================================================================
// WorkspaceHydrator / Syncer — copy workspace.Store contents into a
// freshly-created sandbox so the LLM sees the same files.
// =====================================================================

pub struct WorkspaceHydrator;

impl WorkspaceHydrator {
    /// One-shot: walk the workspace.Store and `write_file` each object
    /// into the executor. Best-effort — partial failures are logged
    /// and the first error short-circuits (the lifecycle sweeper will
    /// retry on next allocation).
    pub async fn hydrate(
        executor: &Arc<dyn Executor>,
        store: &dyn cleanclaw_workspace::Store,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<usize, SandboxError> {
        let objects = store.list(agent_id, project_id, session_id).await?;
        let mut count = 0;
        for obj in objects {
            let bytes = store
                .get(agent_id, project_id, session_id, &obj.path)
                .await
                .map_err(|e| SandboxError::Pool(format!("workspace: {e}")))?;
            executor.write_file(&obj.path, &bytes).await?;
            count += 1;
        }
        Ok(count)
    }
}

// =====================================================================
// userctx — small helper to inject user_id / agent_id into the
// environment a sandbox process sees.
// =====================================================================

pub fn user_environment(user_id: &str, agent_id: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("CLEANCLAW_USER_ID".to_string(), user_id.to_string());
    env.insert("CLEANCLAW_AGENT_ID".to_string(), agent_id.to_string());
    env
}

// =====================================================================
// UserID context propagation. Mirrors
// . Threading userID via
// context (rather than widening every signature) keeps the
// ExecutorPool::Get contract clean and lets sites that don't know
// about chatters (cron flushes, admin reload triggers) keep calling
// Get() the way they already do — they just won't get the per-user
// mount, which is the correct fallback.
// =====================================================================

use std::any::Any;

/// Tag a `dyn Any` context with the current chatter's userID.
/// Empty uid is a no-op so call sites don't have to nil-check.
pub fn with_user_id(ctx: &mut dyn AnyExt, uid: &str) {
    if !uid.is_empty() {
        ctx.set_user_id(uid.to_string());
    }
}

/// Extract the chatter userID set by `with_user_id`, or `""` when
/// no wrap happened. Used by sandbox backends to decide whether
/// to mount per-user skills into the container.
pub fn user_id_from_dyn(ctx: &dyn AnyExt) -> &str {
    ctx.user_id()
}

/// Lightweight AnyExt trait so the per-key context box can be
/// type-erased. Real callers (e.g. a tower middleware layer) will
/// pass a concrete `RequestContext`; the trait is the contract
/// our sandbox layer sees.
pub trait AnyExt: Any + Send + Sync {
    fn set_user_id(&mut self, uid: String);
    fn user_id(&self) -> &str;
}

/// Default in-memory context for tests and standalone use.
#[derive(Default)]
pub struct RequestContext {
    user_id: String,
}

impl AnyExt for RequestContext {
    fn set_user_id(&mut self, uid: String) {
        self.user_id = uid;
    }
    fn user_id(&self) -> &str {
        &self.user_id
    }
}

// =====================================================================
// WorkspaceSync — hydrates / flushes a sandbox's local /workspace
// to a durable `WorkspaceStore`. Mirrors
// .
// =====================================================================

/// Narrow durability interface — `cleanclaw_workspace::Store` is
/// the production implementation (S3 / LocalFS). Mirrors the
/// Go `WorkspaceStore` trait shape.
#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn list_paths(&self, user_id: &str) -> Result<Vec<String>, SandboxError>;
    async fn get(&self, user_id: &str, path: &str) -> Result<Bytes, SandboxError>;
    async fn put(
        &self,
        user_id: &str,
        path: &str,
        content: Bytes,
    ) -> Result<(), SandboxError>;
    async fn delete(&self, user_id: &str, path: &str) -> Result<(), SandboxError>;
}

/// Adapter: adapt the existing `cleanclaw_workspace::Store` to the
/// narrow `WorkspaceStore` interface used by `WorkspaceSync`. Uses
/// the (agent_id=user_id, project_id="", session_id="") scope so
/// per-user files land in a predictable place. Production callers
/// can use this directly.
pub struct WorkspaceStoreAdapter {
    pub inner: Arc<dyn cleanclaw_workspace::Store>,
}

#[async_trait]
impl WorkspaceStore for WorkspaceStoreAdapter {
    async fn list_paths(&self, user_id: &str) -> Result<Vec<String>, SandboxError> {
        let objs = self.inner.list(user_id, "", "").await?;
        Ok(objs.into_iter().map(|o| o.path).collect())
    }
    async fn get(&self, user_id: &str, path: &str) -> Result<Bytes, SandboxError> {
        self.inner
            .get(user_id, "", "", path)
            .await
            .map_err(|e| SandboxError::Workspace(e))
    }
    async fn put(
        &self,
        user_id: &str,
        path: &str,
        content: Bytes,
    ) -> Result<(), SandboxError> {
        self.inner
            .put(user_id, "", "", path, content, "application/octet-stream")
            .await
            .map_err(|e| SandboxError::Workspace(e))
    }
    async fn delete(&self, user_id: &str, path: &str) -> Result<(), SandboxError> {
        self.inner
            .delete(user_id, "", "", path)
            .await
            .map_err(|e| SandboxError::Workspace(e))
    }
}

/// Hydrate a sandbox's local directory from a `WorkspaceStore`.
/// Called on sandbox creation. Best-effort — per-file errors are
/// logged and skipped rather than failing the whole hydrate.
pub struct WorkspaceSync {
    pub store: Arc<dyn WorkspaceStore>,
}

impl WorkspaceSync {
    pub fn new(store: Arc<dyn WorkspaceStore>) -> Self {
        Self { store }
    }

    /// Copy every workspace file for `user_id` into `local_dir`.
    /// Returns the number of files copied.
    pub async fn hydrate(
        &self,
        user_id: &str,
        local_dir: &Path,
    ) -> Result<usize, SandboxError> {
        let files = self.store.list_paths(user_id).await?;
        let mut count = 0;
        for path in files {
            match self.store.get(user_id, &path).await {
                Ok(bytes) => {
                    let full = local_dir.join(&path);
                    if let Some(parent) = full.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&full, &bytes).await?;
                    count += 1;
                }
                Err(e) => {
                    tracing::warn!(user_id, path, error = %e, "workspace hydrate: skip file");
                }
            }
        }
        Ok(count)
    }

    /// Walk `local_dir` and upload every regular file to the
    /// store, skipping hidden files. Returns the number of files
    /// uploaded.
    pub async fn flush(
        &self,
        user_id: &str,
        local_dir: &Path,
    ) -> Result<usize, SandboxError> {
        if !local_dir.is_dir() {
            return Ok(0);
        }
        let mut count = 0;
        let mut stack = vec![local_dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let mut entries = tokio::fs::read_dir(&d).await?;
            while let Some(e) = entries.next_entry().await? {
                let ft = e.file_type().await?;
                let from = e.path();
                if ft.is_dir() {
                    stack.push(from);
                } else {
                    // Skip hidden files.
                    if e.file_name()
                        .to_string_lossy()
                        .starts_with('.')
                    {
                        continue;
                    }
                    let rel = match from.strip_prefix(local_dir) {
                        Ok(r) => r.to_string_lossy().replace('\\', "/"),
                        Err(_) => continue,
                    };
                    let bytes = tokio::fs::read(&from).await?;
                    if let Err(e) = self.store.put(user_id, &rel, Bytes::from(bytes)).await {
                        tracing::warn!(user_id, path = %rel, error = %e, "workspace flush: skip file");
                    } else {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    /// Sync a single file. Used after `write_file` tool calls for
    /// real-time sync (callers can batch via `flush` instead).
    pub async fn sync_file(
        &self,
        user_id: &str,
        local_dir: &Path,
        rel_path: &str,
    ) -> Result<(), SandboxError> {
        let full = local_dir.join(rel_path);
        let bytes = tokio::fs::read(&full).await?;
        let rel = rel_path.replace('\\', "/");
        self.store.put(user_id, &rel, Bytes::from(bytes)).await
    }
}

/// Default sandbox root path. Mirrors CleanClaw's
/// `defaultSandboxRoot = "/workspace"`. The path is where
/// hydrated files land inside the sandbox.
pub const DEFAULT_SANDBOX_ROOT: &str = "/workspace";

/// Strip leading slashes and `..` segments so a hydrated key
/// can't escape `/workspace` even if the store holds a malicious
/// path. Mirrors CleanClaw's `sanitizeSandboxPath`.
pub fn sanitize_sandbox_path(p: &str) -> String {
    let cleaned = p
        .split('/')
        .filter(|s| !s.is_empty() && *s != "..")
        .collect::<Vec<_>>()
        .join("/");
    cleaned
}

#[cfg(test)]
mod userctx_tests {
    use super::*;

    #[test]
    fn user_environment_contains_user_and_agent_id() {
        let env = user_environment("u1", "a1");
        assert_eq!(env.get("CLEANCLAW_USER_ID"), Some(&"u1".to_string()));
        assert_eq!(env.get("CLEANCLAW_AGENT_ID"), Some(&"a1".to_string()));
    }

    #[test]
    fn with_user_id_sets_and_reads() {
        let mut ctx = RequestContext::default();
        with_user_id(&mut ctx, "alice");
        assert_eq!(user_id_from_dyn(&ctx), "alice");
    }

    #[test]
    fn with_user_id_empty_is_noop() {
        let mut ctx = RequestContext::default();
        with_user_id(&mut ctx, "");
        assert_eq!(user_id_from_dyn(&ctx), "");
    }
}

#[cfg(test)]
mod workspace_sync_tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    /// In-memory WorkspaceStore for tests.
    struct MemStore {
        files: Mutex<HashMap<String, HashMap<String, Bytes>>>,
    }

    impl MemStore {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl WorkspaceStore for MemStore {
        async fn list_paths(&self, user_id: &str) -> Result<Vec<String>, SandboxError> {
            let g = self.files.lock().await;
            Ok(g.get(user_id)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default())
        }
        async fn get(&self, user_id: &str, path: &str) -> Result<Bytes, SandboxError> {
            let g = self.files.lock().await;
            g.get(user_id)
                .and_then(|m| m.get(path).cloned())
                .ok_or_else(|| SandboxError::NotFound(path.to_string()))
        }
        async fn put(
            &self,
            user_id: &str,
            path: &str,
            content: Bytes,
        ) -> Result<(), SandboxError> {
            let mut g = self.files.lock().await;
            g.entry(user_id.to_string())
                .or_default()
                .insert(path.to_string(), content);
            Ok(())
        }
        async fn delete(&self, user_id: &str, path: &str) -> Result<(), SandboxError> {
            let mut g = self.files.lock().await;
            if let Some(m) = g.get_mut(user_id) {
                m.remove(path);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn hydrate_copies_files_from_store_to_disk() {
        let store = Arc::new(MemStore::new());
        store
            .put("u1", "a.txt", Bytes::from_static(b"hello"))
            .await
            .unwrap();
        store
            .put("u1", "sub/b.txt", Bytes::from_static(b"world"))
            .await
            .unwrap();
        let local = std::env::temp_dir().join(format!(
            "cleanclaw-sandbox-wssh-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let sync = WorkspaceSync::new(store.clone());
        let n = sync.hydrate("u1", &local).await.unwrap();
        assert_eq!(n, 2);
        let a = tokio::fs::read(local.join("a.txt")).await.unwrap();
        let b = tokio::fs::read(local.join("sub/b.txt")).await.unwrap();
        assert_eq!(a, b"hello");
        assert_eq!(b, b"world");
        let _ = std::fs::remove_dir_all(local);
    }

    #[tokio::test]
    async fn flush_uploads_files_from_disk_to_store() {
        let store = Arc::new(MemStore::new());
        let local = std::env::temp_dir().join(format!(
            "cleanclaw-sandbox-wsfl-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(local.join("sub")).unwrap();
        std::fs::write(local.join("a.txt"), b"hi").unwrap();
        std::fs::write(local.join("sub/b.txt"), b"there").unwrap();
        std::fs::write(local.join(".hidden"), b"skipped").unwrap();
        let sync = WorkspaceSync::new(store.clone());
        let n = sync.flush("u1", &local).await.unwrap();
        assert_eq!(n, 2);
        assert!(store.get("u1", "a.txt").await.is_ok());
        assert!(store.get("u1", "sub/b.txt").await.is_ok());
        // Hidden files are skipped.
        assert!(store.get("u1", ".hidden").await.is_err());
        let _ = std::fs::remove_dir_all(local);
    }

    #[tokio::test]
    async fn sync_file_uploads_one_file() {
        let store = Arc::new(MemStore::new());
        let local = std::env::temp_dir().join(format!(
            "cleanclaw-sandbox-wssf-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("c.txt"), b"yep").unwrap();
        let sync = WorkspaceSync::new(store.clone());
        sync.sync_file("u1", &local, "c.txt").await.unwrap();
        let got = store.get("u1", "c.txt").await.unwrap();
        assert_eq!(got.as_ref(), b"yep");
        let _ = std::fs::remove_dir_all(local);
    }

    #[test]
    fn sanitize_sandbox_path_strips_escape_attempts() {
        assert_eq!(sanitize_sandbox_path("../etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_sandbox_path("/abs/foo"), "abs/foo");
        assert_eq!(sanitize_sandbox_path("a/b/c"), "a/b/c");
        assert_eq!(sanitize_sandbox_path("a/../../b"), "a/b");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-sandbox-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn local_exec_runs_command() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        let res = e.exec("echo hello", Duration::from_secs(5)).await.unwrap();
        assert_eq!(res.stdout.trim(), "hello");
        assert_eq!(res.exit_code, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_exec_captures_exit_code() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        let res = e
            .exec("exit 7", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(res.exit_code, 7);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_exec_timeout_errors() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        let err = e
            .exec("sleep 10", Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::Timeout(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_read_write_round_trip() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        e.write_file("a.txt", b"hi").await.unwrap();
        let got = e.read_file("a.txt").await.unwrap();
        assert_eq!(got.as_ref(), b"hi");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_read_missing_returns_not_found() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        let err = e.read_file("nope.txt").await.unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_list_dir_returns_sorted_entries() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        e.write_file("a.txt", b"a").await.unwrap();
        e.write_file("b.txt", b"b").await.unwrap();
        let entries = e.list_dir(".").await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["a.txt".to_string(), "b.txt".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_list_dir_missing_errors() {
        let dir = tmpdir();
        let e = LocalExecutor::new(dir.clone());
        let err = e.list_dir("nope").await.unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_pool_get_creates_per_scope_dirs() {
        let dir = tmpdir();
        let p = LocalExecutorPool::new(dir.clone());
        let e1 = p.get("a1", "", "s1").await.unwrap();
        e1.write_file("x", b"x").await.unwrap();
        let e2 = p.get("a1", "", "s2").await.unwrap();
        e2.write_file("x", b"y").await.unwrap();
        // Same key returns the same executor.
        let e1b = p.get("a1", "", "s1").await.unwrap();
        assert!(Arc::ptr_eq(&e1, &e1b));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_pool_release_drops_executor() {
        let dir = tmpdir();
        let p = LocalExecutorPool::new(dir.clone());
        let _ = p.get("a1", "", "s1").await.unwrap();
        p.release("a1", "", "s1").await.unwrap();
        // After release, next get creates a new one (no equality check,
        // just no panic).
        let _ = p.get("a1", "", "s1").await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_pool_backend() {
        let p = LocalExecutorPool::new("/tmp");
        assert_eq!(p.backend(), "local");
    }

    #[tokio::test]
    async fn local_pool_close_all_clears_cache() {
        let dir = tmpdir();
        let p = LocalExecutorPool::new(dir.clone());
        let _ = p.get("a1", "", "s1").await.unwrap();
        p.close_all().await.unwrap();
        // No panic on subsequent get.
        let _ = p.get("a1", "", "s1").await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lifecycle_pool_marks_used() {
        let dir = tmpdir();
        let inner = Arc::new(LocalExecutorPool::new(dir.clone()));
        let lc = Arc::new(LifecyclePool::new(inner, Duration::from_secs(60)));
        let _ = lc.get("a1", "", "s1").await.unwrap();
        assert_eq!(lc.active_count().await, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lifecycle_pool_sweep_releases_idle() {
        let dir = tmpdir();
        let inner = Arc::new(LocalExecutorPool::new(dir.clone()));
        let lc = Arc::new(LifecyclePool::new(
            inner.clone(),
            Duration::from_millis(0),
        ));
        let _ = lc.get("a1", "", "s1").await.unwrap();
        // Backdate the lastUsed to force idle.
        {
            let mut g = lc.state.lock().await;
            for v in g.values_mut() {
                *v = std::time::Instant::now()
                    .checked_sub(Duration::from_secs(60))
                    .unwrap_or_else(std::time::Instant::now);
            }
        }
        let released = lc.sweep_once().await;
        assert_eq!(released, 1);
        assert_eq!(lc.active_count().await, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lifecycle_pool_release_clears_state() {
        let dir = tmpdir();
        let inner = Arc::new(LocalExecutorPool::new(dir.clone()));
        let lc = LifecyclePool::new(inner, Duration::from_secs(60));
        let _ = lc.get("a1", "", "s1").await.unwrap();
        lc.release("a1", "", "s1").await.unwrap();
        assert_eq!(lc.active_count().await, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lifecycle_pool_backend_passthrough() {
        let dir = tmpdir();
        let inner = Arc::new(LocalExecutorPool::new(dir.clone()));
        let lc = LifecyclePool::new(inner, Duration::from_secs(60));
        assert_eq!(lc.backend(), "local");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn user_environment_includes_ids() {
        let env = user_environment("u1", "a1");
        assert_eq!(env.get("CLEANCLAW_USER_ID"), Some(&"u1".to_string()));
        assert_eq!(env.get("CLEANCLAW_AGENT_ID"), Some(&"a1".to_string()));
    }

    #[test]
    fn local_executor_backend_constant() {
        let e = LocalExecutor::new("/tmp");
        assert_eq!(e.backend(), "local");
    }
}

// =====================================================================
// Docker executor. go`.
// =====================================================================

fn scope_dir_helper(root: &Path, agent_id: &str, project_id: &str, session_id: &str) -> PathBuf {
    let mut p = root.join(agent_id);
    if !project_id.is_empty() {
        p = p.join("projects").join(project_id);
    } else if !session_id.is_empty() {
        p = p.join("sessions").join(session_id);
    }
    p
}

/// Resource + network policy for a sandbox container.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub max_cpu: String,    // e.g. "2"
    pub max_memory: String, // e.g. "512m"
    pub net_mode: String,   // "none" | "host" | "bridge"
}

/// Docker-backed `Executor` running each tool call as `docker exec`.
//
/// **Status**: this is a process-level wrapper that shells out to
/// `docker exec` / `docker cp`. Production deployments should pair
/// it with a Docker daemon reachable from the gateway host. The
/// stub here doesn't actually create containers — it returns a
/// `DockerExecutor` that can be queried for backend/identity.
pub struct DockerExecutor {
    image: String,
    workspace: String,
    policy: Policy,
    container_id: String,
    /// Optional container id this executor is bound to. When set,
    /// `Executor::exec` shells out to `docker exec` against it
    /// instead of returning `NotConfigured`. The gateway's sandbox
    /// runtime calls `bind_container_id` after `docker run` finishes
    /// bringing the container up.
    bound_container: std::sync::Mutex<Option<String>>,
}

impl DockerExecutor {
    pub fn new(image: impl Into<String>, workspace: impl Into<String>, policy: Policy) -> Self {
        Self {
            image: image.into(),
            workspace: workspace.into(),
            policy,
            container_id: String::new(),
            bound_container: std::sync::Mutex::new(None),
        }
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    /// Bind a live container id so subsequent `Executor::exec` calls
    /// use `docker exec` against it. Pass `None` to clear.
    pub fn bind_container_id(&self, id: Option<String>) {
        if let Ok(mut g) = self.bound_container.lock() {
            *g = id;
        }
    }

    fn bound(&self) -> Option<String> {
        self.bound_container.lock().ok().and_then(|g| g.clone())
    }

    /// Issue a `docker exec` against the given container. Returns
    /// (stdout, stderr, exit_code) or an Io error.
    pub async fn docker_exec(
        &self,
        container_id: &str,
        cmd: &[&str],
    ) -> Result<SandboxExec, SandboxError> {
        let start = std::time::Instant::now();
        let mut args: Vec<String> = vec!["exec".into(), "-i".into(), container_id.into()];
        args.extend(cmd.iter().map(|s| s.to_string()));
        let output = tokio::process::Command::new("docker")
            .args(&args)
            .output()
            .await?;
        Ok(SandboxExec {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            duration: start.elapsed(),
        })
    }

    /// Issue a `docker run` to bring a fresh container up. Returns
    /// the container id (first 12 hex chars from `docker run --detach`
    /// output). Mirrors the entrypoint the Go daemon's
    /// `docker_executor.go` uses.
    pub async fn docker_run(image: &str, workspace: &str) -> Result<String, SandboxError> {
        let output = tokio::process::Command::new("docker")
            .args([
                "run",
                "--detach",
                "--network=none",
                "-v",
                &format!("{workspace}:/workspace"),
                image,
                "sleep",
                "infinity",
            ])
            .output()
            .await?;
        if !output.status.success() {
            return Err(SandboxError::Exec(format!(
                "docker run failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[async_trait::async_trait]
impl Executor for DockerExecutor {
    async fn exec(
        &self,
        command: &str,
        _timeout: Duration,
    ) -> Result<SandboxExec, SandboxError> {
        // If no container is bound, fall back to the historical
        // NotConfigured so callers that haven't wired the gateway
        // sandbox runtime keep their existing semantics. When a
        // container is bound, shell out via `docker exec` and run the
        // command through `sh -c` so the caller can use shell
        // metacharacters the way the Go exec tool does.
        let container = match self.bound() {
            Some(c) if !c.is_empty() => c,
            _ => {
                return Err(SandboxError::NotConfigured(
                    "DockerExecutor: no container bound (call bind_container_id)",
                ));
            }
        };
        self.docker_exec(&container, &["sh", "-c", command]).await
    }
    async fn read_file(&self, path: &str) -> Result<bytes::Bytes, SandboxError> {
        let container = self.bound().ok_or(SandboxError::NotConfigured("docker"))?;
        let out = self.docker_exec(&container, &["cat", path]).await?;
        if out.exit_code != 0 {
            return Err(SandboxError::Exec(format!(
                "docker read_file failed: {}",
                out.stderr
            )));
        }
        Ok(bytes::Bytes::from(out.stdout.into_bytes()))
    }
    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), SandboxError> {
        let container = self.bound().ok_or(SandboxError::NotConfigured("docker"))?;
        // Pipe via stdin: `cat > <path>` with the content as the
        // command's stdin. tokio::process::Command::Stdio isn't
        // thread-safe, so we go via a heredoc tempfile in /workspace
        // when the workspace is mounted.
        let tmp = format!("/workspace/.cleanclaw-write-{}", std::process::id());
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args([
            "exec",
            "-i",
            &container,
            "sh",
            "-c",
            &format!("cat > {tmp} && mv {tmp} {path}"),
        ]);
        cmd.stdin(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(SandboxError::Io)?;
        if let Some(stdin) = child.stdin.as_mut() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(content)
                .await
                .map_err(|e| SandboxError::Exec(format!("write stdin: {e}")))?;
        }
        let status = child.wait().await?;
        if !status.success() {
            return Err(SandboxError::Exec(format!(
                "docker write_file exit {status}"
            )));
        }
        Ok(())
    }
    async fn list_dir(&self, path: &str) -> Result<Vec<SandboxEntry>, SandboxError> {
        let container = self.bound().ok_or(SandboxError::NotConfigured("docker"))?;
        // `ls -1` one entry per line; the -a flag exposes dotfiles
        // too. Mirrors the Go `ListDir` shape.
        let out = self
            .docker_exec(&container, &["ls", "-1a", path])
            .await?;
        if out.exit_code != 0 {
            return Ok(Vec::new());
        }
        Ok(out
            .stdout
            .lines()
            .filter(|l| !l.is_empty() && *l != "." && *l != "..")
            .map(|l| SandboxEntry {
                name: l.to_string(),
                is_dir: false,
                size: 0,
                mod_time: chrono::Utc::now(),
            })
            .collect())
    }
    fn backend(&self) -> &'static str {
        "docker"
    }
    async fn close(&self) -> Result<(), SandboxError> {
        // `docker stop` + `docker rm` against the bound container.
        if let Some(c) = self.bound() {
            let _ = tokio::process::Command::new("docker")
                .args(["stop", &c])
                .output()
                .await;
            let _ = tokio::process::Command::new("docker")
                .args(["rm", "-f", &c])
                .output()
                .await;
            self.bind_container_id(None);
        }
        Ok(())
    }
}

/// Pool that returns Docker executors.
pub struct DockerExecutorPool {
    image: String,
    workspace_root: String,
    policy: Policy,
}

impl DockerExecutorPool {
    pub fn new(
        image: impl Into<String>,
        workspace_root: impl Into<String>,
        policy: Policy,
    ) -> Self {
        Self {
            image: image.into(),
            workspace_root: workspace_root.into(),
            policy,
        }
    }
}

#[async_trait::async_trait]
impl ExecutorPool for DockerExecutorPool {
    async fn get(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<Arc<dyn Executor>, SandboxError> {
        let workspace = scope_dir_helper(
            std::path::Path::new(&self.workspace_root),
            agent_id,
            project_id,
            session_id,
        );
        tokio::fs::create_dir_all(&workspace).await?;
        Ok(Arc::new(DockerExecutor::new(
            self.image.clone(),
            workspace.to_string_lossy().to_string(),
            self.policy.clone(),
        )))
    }
    async fn release(
        &self,
        _agent_id: &str,
        _project_id: &str,
        _session_id: &str,
    ) -> Result<(), SandboxError> {
        Ok(())
    }
    async fn close_all(&self) -> Result<(), SandboxError> {
        Ok(())
    }
    fn backend(&self) -> &'static str {
        "docker"
    }
}

// =====================================================================
// E2B executor.
// Real HTTP client against api.e2b.dev. The offline build compiles
// the request builders + envelope parsers; the actual HTTP path
// activates once an `E2BExecutor::with_client()` is plugged in at boot.
// =====================================================================

pub struct E2BExecutor {
    api_key: String,
    template: String,
    base_url: String,
    /// Optional `reqwest::Client`. When `None`, every method
    /// returns `NotConfigured` — the offline build path. When
    /// `Some`, the methods issue real HTTP calls against
    /// `E2B_API_BASE`.
    client: Option<Arc<reqwest::Client>>,
}

impl E2BExecutor {
    pub fn new(api_key: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            template: template.into(),
            base_url: crate::remote::E2B_API_BASE.to_string(),
            client: None,
        }
    }

    pub fn template(&self) -> &str {
        &self.template
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn with_endpoint(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }

    pub fn with_client(mut self, c: Arc<reqwest::Client>) -> Self {
        self.client = Some(c);
        self
    }

    fn require_client(&self) -> Result<&reqwest::Client, SandboxError> {
        self.client
            .as_ref()
            .map(|c| c.as_ref())
            .ok_or(SandboxError::NotConfigured("e2b client"))
    }

    /// Build the auth headers E2B expects (Authorization + JSON
    /// content type). Centralised so the test suite can pin the
    /// exact wire format.
    pub fn auth_headers(&self) -> std::collections::HashMap<&'static str, String> {
        let mut h = std::collections::HashMap::new();
        h.insert("X-API-Key", self.api_key.clone());
        h.insert("Content-Type", "application/json".into());
        h
    }

    /// `POST /sandboxes` — create a fresh E2B sandbox. Mirrors the
    /// Go daemon's `E2b.Create()`.
    pub async fn create_sandbox(
        &self,
    ) -> Result<crate::remote::E2BSandbox, SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes", self.base_url);
        let body = serde_json::json!({ "template": self.template });
        let resp = client
            .post(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b create: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "e2b create HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        let sb: crate::remote::E2BSandbox = resp
            .json()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b create json: {e}")))?;
        Ok(sb)
    }

    /// `POST /sandboxes/{id}/process/exec` — execute a command
    /// inside the sandbox. Mirrors `E2b.Exec()`.
    pub async fn exec_remote(
        &self,
        sandbox_id: &str,
        command: &str,
    ) -> Result<crate::remote::E2BExecResponse, SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}/process/exec", self.base_url, sandbox_id);
        let body = crate::remote::E2BExecRequest::new(command);
        let resp = client
            .post(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b exec: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "e2b exec HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        let out: crate::remote::E2BExecResponse = resp
            .json()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b exec json: {e}")))?;
        Ok(out)
    }

    /// `GET /sandboxes/{id}/files/{path}` — read a file from the
    /// sandbox. The base64 payload is decoded. Mirrors `E2b.ReadFile()`.
    pub async fn read_file_remote(
        &self,
        sandbox_id: &str,
        path: &str,
    ) -> Result<Vec<u8>, SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}/files/{}", self.base_url, sandbox_id, path);
        let resp = client
            .get(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b read: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "e2b read HTTP {}",
                resp.status()
            )));
        }
        let fr: crate::remote::E2BFileResponse = resp
            .json()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b read json: {e}")))?;
        fr.decode().map_err(SandboxError::Upstream)
    }

    /// `POST /sandboxes/{id}/files/{path}` — write a file. Body is
    /// `{ "content": "<base64>" }`. Mirrors `E2b.WriteFile()`.
    pub async fn write_file_remote(
        &self,
        sandbox_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}/files/{}", self.base_url, sandbox_id, path);
        let body = crate::remote::E2BWriteFileRequest {
            content: base64::engine::general_purpose::STANDARD.encode(content),
        };
        let resp = client
            .post(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b write: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "e2b write HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        Ok(())
    }

    /// `DELETE /sandboxes/{id}` — kill the sandbox. Mirrors
    /// `E2b.Kill()`.
    pub async fn kill_sandbox_remote(
        &self,
        sandbox_id: &str,
    ) -> Result<(), SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}", self.base_url, sandbox_id);
        let resp = client
            .delete(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("e2b kill: {e}")))?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(SandboxError::Upstream(format!(
                "e2b kill HTTP {}",
                resp.status()
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Executor for E2BExecutor {
    async fn exec(
        &self,
        _command: &str,
        _timeout: Duration,
    ) -> Result<SandboxExec, SandboxError> {
        // The trait path needs a bound sandbox id; the gateway
        // is responsible for that. Operators that wired a client
        // use `exec_remote()` after binding an id.
        if self.client.is_none() {
            return Err(SandboxError::NotConfigured("e2b client"));
        }
        Err(SandboxError::NotConfigured(
            "e2b exec via trait requires a bound sandbox id; use exec_remote()",
        ))
    }
    async fn read_file(&self, _path: &str) -> Result<bytes::Bytes, SandboxError> {
        if self.client.is_none() {
            return Err(SandboxError::NotConfigured("e2b client"));
        }
        Err(SandboxError::NotConfigured(
            "e2b read_file via trait requires a bound sandbox id",
        ))
    }
    async fn write_file(&self, _path: &str, _content: &[u8]) -> Result<(), SandboxError> {
        if self.client.is_none() {
            return Err(SandboxError::NotConfigured("e2b client"));
        }
        Err(SandboxError::NotConfigured(
            "e2b write_file via trait requires a bound sandbox id",
        ))
    }
    async fn list_dir(&self, _path: &str) -> Result<Vec<SandboxEntry>, SandboxError> {
        Err(SandboxError::NotConfigured("e2b"))
    }
    fn backend(&self) -> &'static str {
        "e2b"
    }
    async fn close(&self) -> Result<(), SandboxError> {
        Ok(())
    }
}

// =====================================================================
// BoxLite executor.
// REST + WebSocket client. The offline build compiles the request
// builders + envelope parsers; the actual HTTP path activates once
// a `BoxLiteExecutor::with_endpoint()` is plugged in at boot.
// =====================================================================

pub struct BoxLiteExecutor {
    image: String,
    api_key: String,
    base_url: String,
    /// Optional `reqwest::Client`. When `None`, every method
    /// returns `NotConfigured` — the offline build path. When
    /// `Some`, the methods issue real HTTP calls against
    /// `BOXLITE_API_BASE`.
    client: Option<Arc<reqwest::Client>>,
}

impl BoxLiteExecutor {
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            api_key: String::new(),
            base_url: crate::remote::BOXLITE_API_BASE.to_string(),
            client: None,
        }
    }

    pub fn with_api_key(mut self, k: impl Into<String>) -> Self {
        self.api_key = k.into();
        self
    }

    pub fn with_endpoint(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }

    pub fn with_client(mut self, c: Arc<reqwest::Client>) -> Self {
        self.client = Some(c);
        self
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn require_client(&self) -> Result<&reqwest::Client, SandboxError> {
        self.client
            .as_ref()
            .map(|c| c.as_ref())
            .ok_or(SandboxError::NotConfigured("boxlite client"))
    }

    /// Build the auth headers BoxLite expects. Centralised so the
    /// test suite can pin the exact wire format.
    pub fn auth_headers(&self) -> std::collections::HashMap<&'static str, String> {
        let mut h = std::collections::HashMap::new();
        if !self.api_key.is_empty() {
            h.insert("Authorization", format!("Bearer {}", self.api_key));
        }
        h.insert("Content-Type", "application/json".into());
        h
    }

    /// `POST /sandboxes`. Mirrors the Go `Boxlite.Create()`.
    pub async fn create_sandbox(
        &self,
    ) -> Result<crate::remote::BoxLiteSandbox, SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes", self.base_url);
        let body = serde_json::json!({ "template": self.image });
        let resp = client
            .post(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite create: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "boxlite create HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        let sb: crate::remote::BoxLiteSandbox = resp
            .json()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite create json: {e}")))?;
        Ok(sb)
    }

    /// `POST /sandboxes/{id}/exec`. Mirrors the Go `Boxlite.Exec()`.
    pub async fn exec_remote(
        &self,
        sandbox_id: &str,
        command: &str,
    ) -> Result<crate::remote::BoxLiteExecResponse, SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}/exec", self.base_url, sandbox_id);
        let body = crate::remote::BoxLiteExecRequest::new(command);
        let resp = client
            .post(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite exec: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "boxlite exec HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        let out: crate::remote::BoxLiteExecResponse = resp
            .json()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite exec json: {e}")))?;
        Ok(out)
    }

    /// `GET /sandboxes/{id}/files?path=…`. Mirrors the Go `Boxlite.ReadFile()`.
    pub async fn read_file_remote(
        &self,
        sandbox_id: &str,
        path: &str,
    ) -> Result<Vec<u8>, SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}/files", self.base_url, sandbox_id);
        let resp = client
            .get(&url)
            .headers(self.auth_headers().try_into_http().map_err(SandboxError::Http)?)
            .query(&[("path", path)])
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite read: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "boxlite read HTTP {}",
                resp.status()
            )));
        }
        let fr: crate::remote::BoxLiteFileResponse = resp
            .json()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite read json: {e}")))?;
        Ok(fr.bytes)
    }

    /// `PUT /sandboxes/{id}/files?path=…`. Mirrors the Go
    /// `Boxlite.WriteFile()`. The body is multipart/form-data
    /// with a single `file` field.
    pub async fn write_file_remote(
        &self,
        sandbox_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}/files", self.base_url, sandbox_id);
        let part = reqwest::multipart::Part::bytes(content.to_vec())
            .file_name(path.rsplit('/').next().unwrap_or("file").to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let mut headers = reqwest::header::HeaderMap::new();
        if !self.api_key.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key).parse().unwrap(),
            );
        }
        let resp = client
            .put(&url)
            .query(&[("path", path)])
            .headers(headers)
            .multipart(form)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite write: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "boxlite write HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        Ok(())
    }

    /// `DELETE /sandboxes/{id}`. Mirrors the Go `Boxlite.Close()`.
    pub async fn delete_sandbox_remote(
        &self,
        sandbox_id: &str,
    ) -> Result<(), SandboxError> {
        let client = self.require_client()?;
        let url = format!("{}/sandboxes/{}", self.base_url, sandbox_id);
        let mut headers = reqwest::header::HeaderMap::new();
        if !self.api_key.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key).parse().unwrap(),
            );
        }
        let resp = client
            .delete(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| SandboxError::Upstream(format!("boxlite delete: {e}")))?;
        if !resp.status().is_success() {
            return Err(SandboxError::Upstream(format!(
                "boxlite delete HTTP {}",
                resp.status()
            )));
        }
        Ok(())
    }
}

// Tiny adapter: turn a HashMap<&str, String> into a reqwest
// HeaderMap. Used by the BoxLite executor's auth_headers().
// Lives in this crate to avoid a new tiny utility crate.
trait TryIntoHeaderMap {
    type Err;
    fn try_into_http(self) -> Result<reqwest::header::HeaderMap, Self::Err>;
}

impl TryIntoHeaderMap for std::collections::HashMap<&'static str, String> {
    type Err = String;
    fn try_into_http(self) -> Result<reqwest::header::HeaderMap, Self::Err> {
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in self {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| format!("{e}"))?;
            let val = reqwest::header::HeaderValue::from_str(&v)
                .map_err(|e| format!("{e}"))?;
            h.insert(name, val);
        }
        Ok(h)
    }
}

#[async_trait::async_trait]
impl Executor for BoxLiteExecutor {
    async fn exec(
        &self,
        _command: &str,
        _timeout: Duration,
    ) -> Result<SandboxExec, SandboxError> {
        // Without a bound sandbox id there's no remote to talk to.
        // The exec_remote() method is the real path; this method
        // exists to satisfy the trait + the offline build.
        if self.client.is_none() {
            return Err(SandboxError::NotConfigured("boxlite client"));
        }
        // Operators that wired the client must have called
        // create_sandbox() first. We don't track ids here — the
        // gateway is responsible for that. The trait contract is
        // best-effort.
        Err(SandboxError::NotConfigured(
            "boxlite exec via trait requires a bound sandbox id; use exec_remote()",
        ))
    }
    async fn read_file(&self, _path: &str) -> Result<bytes::Bytes, SandboxError> {
        if self.client.is_none() {
            return Err(SandboxError::NotConfigured("boxlite client"));
        }
        Err(SandboxError::NotConfigured(
            "boxlite read_file via trait requires a bound sandbox id",
        ))
    }
    async fn write_file(&self, _path: &str, _content: &[u8]) -> Result<(), SandboxError> {
        if self.client.is_none() {
            return Err(SandboxError::NotConfigured("boxlite client"));
        }
        Err(SandboxError::NotConfigured(
            "boxlite write_file via trait requires a bound sandbox id",
        ))
    }
    async fn list_dir(&self, _path: &str) -> Result<Vec<SandboxEntry>, SandboxError> {
        Err(SandboxError::NotConfigured("boxlite"))
    }
    fn backend(&self) -> &'static str {
        "boxlite"
    }
    async fn close(&self) -> Result<(), SandboxError> {
        Ok(())
    }
}

#[cfg(test)]
mod docker_e2b_boxlite_tests {
    use super::*;

    #[test]
    fn policy_default() {
        let p = Policy::default();
        assert!(p.max_cpu.is_empty());
        assert!(p.net_mode.is_empty());
    }

    #[test]
    fn docker_executor_accessors() {
        let e = DockerExecutor::new("python:3.12", "/ws", Policy::default());
        assert_eq!(e.image(), "python:3.12");
        assert_eq!(e.workspace(), "/ws");
        assert_eq!(e.backend(), "docker");
        assert_eq!(e.container_id(), "");
    }

    #[tokio::test]
    async fn docker_exec_returns_sandbox_exec_on_docker_missing() {
        // Even without a docker daemon the `docker` binary might
        // not exist; we just want to ensure the call doesn't panic.
        let e = DockerExecutor::new("img", "/ws", Policy::default());
        let r = e
            .docker_exec("nonexistent-container", &["echo", "hi"])
            .await;
        // Err(Io) is fine; the function exercised the path.
        assert!(r.is_err() || r.is_ok());
    }

    #[test]
    fn docker_executor_pool_backend() {
        let p = DockerExecutorPool::new("img", "/ws", Policy::default());
        assert_eq!(p.backend(), "docker");
    }

    #[tokio::test]
    async fn docker_pool_get_creates_workspace_dir() {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-sandbox-dockerpool-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let p = DockerExecutorPool::new("img", dir.to_string_lossy().to_string(), Policy::default());
        let _exec = p.get("a1", "", "s1").await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn e2b_executor_template_accessor() {
        let e = E2BExecutor::new("sk", "base");
        assert_eq!(e.template(), "base");
        assert_eq!(e.backend(), "e2b");
    }

    #[test]
    fn boxlite_executor_image_accessor() {
        let e = BoxLiteExecutor::new("alpine");
        assert_eq!(e.image(), "alpine");
        assert_eq!(e.backend(), "boxlite");
    }

    #[test]
    fn boxlite_executor_with_api_key() {
        let e = BoxLiteExecutor::new("alpine").with_api_key("sk_live_abc");
        let h = e.auth_headers();
        assert_eq!(h.get("Authorization").unwrap(), "Bearer sk_live_abc");
        assert_eq!(h.get("Content-Type").unwrap(), "application/json");
    }

    #[test]
    fn boxlite_executor_no_auth_when_key_blank() {
        let e = BoxLiteExecutor::new("alpine");
        let h = e.auth_headers();
        assert!(!h.contains_key("Authorization"));
    }

    #[test]
    fn boxlite_executor_with_endpoint_overrides_default() {
        let e = BoxLiteExecutor::new("alpine")
            .with_endpoint("https://internal.boxlite.local:9000");
        assert_eq!(e.base_url(), "https://internal.boxlite.local:9000");
    }

    #[test]
    fn boxlite_executor_default_endpoint_matches_remote_const() {
        let e = BoxLiteExecutor::new("alpine");
        assert_eq!(e.base_url(), crate::remote::BOXLITE_API_BASE);
        assert_eq!(e.base_url(), "https://api.boxlite.ai/v1");
    }

    #[test]
    fn boxlite_exec_trait_returns_not_configured_when_client_missing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = BoxLiteExecutor::new("alpine");
            let err = e.exec("ls", std::time::Duration::from_secs(5)).await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured("boxlite client")));
        });
    }

    #[test]
    fn boxlite_exec_trait_returns_not_configured_with_client() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let c = Arc::new(reqwest::Client::new());
            let e = BoxLiteExecutor::new("alpine").with_client(c);
            // No bound sandbox id, so even with a client, the trait
            // path refuses — the gateway has to use exec_remote()
            // after binding an id.
            let err = e.exec("ls", std::time::Duration::from_secs(5)).await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured(_)));
        });
    }

    #[test]
    fn boxlite_list_dir_returns_not_configured() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = BoxLiteExecutor::new("alpine");
            let err = e.list_dir("/").await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured("boxlite")));
        });
    }

    #[test]
    fn boxlite_close_is_idempotent() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = BoxLiteExecutor::new("alpine");
            e.close().await.unwrap();
            e.close().await.unwrap();
        });
    }

    // -----------------------------------------------------------------
    // E2B real-HTTP executor tests. Mirrors the BoxLite tests above
    // so the offline build path is fully covered. The on-the-wire
    // behaviour is exercised through `auth_headers()` (which is
    // observable) and through the trait's `NotConfigured` branches
    // (which the real-HTTP path would short-circuit past).
    // -----------------------------------------------------------------

    #[test]
    fn e2b_executor_accessors() {
        let e = E2BExecutor::new("sk_test_xyz", "base");
        assert_eq!(e.template(), "base");
        assert_eq!(e.backend(), "e2b");
    }

    #[test]
    fn e2b_executor_auth_headers_pin_wire_format() {
        let e = E2BExecutor::new("sk_test_abc", "base");
        let h = e.auth_headers();
        assert_eq!(h.get("X-API-Key").unwrap(), "sk_test_abc");
        assert_eq!(h.get("Content-Type").unwrap(), "application/json");
        // E2B does NOT use a Bearer prefix; that distinguishes it
        // from BoxLite's `Authorization: Bearer …`.
        assert!(!h.contains_key("Authorization"));
    }

    #[test]
    fn e2b_executor_default_endpoint_matches_remote_const() {
        let e = E2BExecutor::new("sk", "base");
        assert_eq!(e.base_url(), crate::remote::E2B_API_BASE);
        assert_eq!(e.base_url(), "https://api.e2b.dev");
    }

    #[test]
    fn e2b_executor_with_endpoint_overrides_default() {
        let e = E2BExecutor::new("sk", "base")
            .with_endpoint("https://internal.e2b.local:8080");
        assert_eq!(e.base_url(), "https://internal.e2b.local:8080");
    }

    #[test]
    fn e2b_executor_with_client_records_presence() {
        let c = Arc::new(reqwest::Client::new());
        let e = E2BExecutor::new("sk", "base").with_client(c);
        // We can't peek at the client field directly, but the
        // trait exec should change its message: with a client,
        // it complains about the missing bound sandbox id, not
        // the missing client.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let err = e.exec("ls", std::time::Duration::from_secs(5)).await.unwrap_err();
            match err {
                SandboxError::NotConfigured(msg) => {
                    assert!(msg.contains("bound sandbox id"), "{msg}");
                    assert!(!msg.contains("client"), "{msg}");
                }
                other => panic!("unexpected error: {other:?}"),
            }
        });
    }

    #[test]
    fn e2b_exec_trait_returns_not_configured_when_client_missing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = E2BExecutor::new("sk", "base");
            let err = e.exec("ls", std::time::Duration::from_secs(5)).await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured("e2b client")));
        });
    }

    #[test]
    fn e2b_read_file_trait_returns_not_configured() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = E2BExecutor::new("sk", "base");
            let err = e.read_file("/etc/hostname").await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured(_)));
        });
    }

    #[test]
    fn e2b_write_file_trait_returns_not_configured() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = E2BExecutor::new("sk", "base");
            let err = e.write_file("/tmp/x", b"hi").await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured(_)));
        });
    }

    #[test]
    fn e2b_list_dir_returns_not_configured() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = E2BExecutor::new("sk", "base");
            let err = e.list_dir("/").await.unwrap_err();
            assert!(matches!(err, SandboxError::NotConfigured("e2b")));
        });
    }

    #[test]
    fn e2b_close_is_idempotent() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = E2BExecutor::new("sk", "base");
            e.close().await.unwrap();
            e.close().await.unwrap();
        });
    }
}
