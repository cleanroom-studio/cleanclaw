//! Durable blob store for agent-generated artifacts (PDFs, images,
//! downloaded files, intermediate data, …). Mirrors
//! .
//!
//! Two backends:
//!   - `LocalFs` — pod-local filesystem under a root dir. Default for
//!     single-host deployments.
//!   - `S3` — any S3-compatible bucket (AWS S3, MinIO, R2, B2). Stubs
//!     in this crate unless the `s3` feature is enabled (deliberate —
//!     a runtime S3 client is a heavy dep for tests; the gateway
//!     brings its own).
//!
//! `Factory` picks the backend at startup; `Metered` wraps any Store
//! to count bytes flowing through `Put`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace: object not found")]
    NotFound,
    #[error("workspace: signed URLs not supported by this backend")]
    SignedUrlUnsupported,
    #[error("workspace: destination already exists")]
    MoveDestinationExists,
    #[error("workspace: missing endpoint for {0} backend")]
    MissingEndpoint(&'static str),
    #[error("workspace: missing region for {0} backend")]
    MissingRegion(&'static str),
    #[error("workspace: missing accountId for {0} backend")]
    MissingAccountId(&'static str),
    #[error("workspace: unknown type {0}")]
    UnknownType(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("other: {0}")]
    Other(String),
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path: String,
    pub size: i64,
    pub content_type: String,
    pub mod_time: DateTime<Utc>,
}

/// Stable storage interface. Implementations MUST be concurrency-safe.
/// Paths are agent-relative (e.g. "report.pdf", "images/cover.png").
/// Absolute paths are never passed in.
#[async_trait]
pub trait Store: Send + Sync + 'static {
    async fn put(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<(), WorkspaceError>;

    async fn get(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
    ) -> Result<Bytes, WorkspaceError>;

    async fn stat(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
    ) -> Result<ObjectInfo, WorkspaceError>;

    async fn list(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<Vec<ObjectInfo>, WorkspaceError>;

    async fn delete(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
    ) -> Result<(), WorkspaceError>;

    async fn move_scope(
        &self,
        agent_id: &str,
        from_project_id: &str,
        from_session_id: &str,
        to_project_id: &str,
        to_session_id: &str,
    ) -> Result<(), WorkspaceError>;

    async fn signed_url(
        &self,
        _agent_id: &str,
        _project_id: &str,
        _session_id: &str,
        _path: &str,
        _ttl: std::time::Duration,
    ) -> Result<String, WorkspaceError> {
        Err(WorkspaceError::SignedUrlUnsupported)
    }

    /// Downcast hook for tests that need to peek at concrete
    /// backends. Default returns `None`; concrete impls override.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

/// Helper to compute the scope directory for an (agent, project, session)
/// tuple. Same layout the Go `LocalScopeDir` uses:
//
/// * `project_id=""`, `session_id=""` → `<root>/<agent>/<path>`
/// * `project_id=""`, `session_id="x"` → `<root>/<agent>/sessions/x/<path>`
/// * `project_id="p"`, any session     → `<root>/<agent>/projects/p/<path>`
pub fn scope_dir(root: &Path, agent_id: &str, project_id: &str, session_id: &str) -> PathBuf {
    let mut p = root.join(agent_id);
    if !project_id.is_empty() {
        p = p.join("projects").join(project_id);
    } else if !session_id.is_empty() {
        p = p.join("sessions").join(session_id);
    }
    p
}

// =====================================================================
// LocalFs — pod-local filesystem.
// =====================================================================

pub struct LocalFs {
    root: PathBuf,
}

impl LocalFs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Marker method matching Go's `LocalScoper` interface. LocalFs
    /// always has a host directory; S3 / R2 don't.
    pub fn local_scope_dir(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> (PathBuf, bool) {
        (
            scope_dir(&self.root, agent_id, project_id, session_id),
            true,
        )
    }

    fn full_path(&self, agent_id: &str, project_id: &str, session_id: &str, path: &str) -> PathBuf {
        scope_dir(&self.root, agent_id, project_id, session_id).join(path)
    }
}

fn sniff_content_type(path: &Path) -> String {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let from_ext = mime_guess_from_ext(ext);
    from_ext.unwrap_or_else(|| "application/octet-stream".into())
}

fn mime_guess_from_ext(ext: &str) -> Option<String> {
    let m = match ext.to_ascii_lowercase().as_str() {
        "txt" | "log" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "zip" => "application/zip",
        "csv" => "text/csv",
        _ => return None,
    };
    Some(m.to_string())
}

#[async_trait]
impl Store for LocalFs {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    async fn put(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<(), WorkspaceError> {
        let full = self.full_path(agent_id, project_id, session_id, path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let ct = if content_type.is_empty() {
            sniff_content_type(&full)
        } else {
            content_type.to_string()
        };
        let _ = ct; // content type is informational; LocalFs doesn't store it
        tokio::fs::write(&full, &data).await?;
        Ok(())
    }

    async fn get(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
    ) -> Result<Bytes, WorkspaceError> {
        let full = self.full_path(agent_id, project_id, session_id, path);
        match tokio::fs::read(&full).await {
            Ok(b) => Ok(Bytes::from(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(WorkspaceError::NotFound),
            Err(e) => Err(WorkspaceError::Io(e)),
        }
    }

    async fn stat(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
    ) -> Result<ObjectInfo, WorkspaceError> {
        let full = self.full_path(agent_id, project_id, session_id, path);
        let meta = match tokio::fs::metadata(&full).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(WorkspaceError::NotFound)
            }
            Err(e) => return Err(WorkspaceError::Io(e)),
        };
        let mod_time = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                let dt: DateTime<Utc> = (std::time::UNIX_EPOCH + d).into();
                dt
            })
            .unwrap_or_else(Utc::now);
        Ok(ObjectInfo {
            path: path.to_string(),
            size: meta.len() as i64,
            content_type: sniff_content_type(&full),
            mod_time,
        })
    }

    async fn list(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
    ) -> Result<Vec<ObjectInfo>, WorkspaceError> {
        // Loose list: every object under the agent regardless of scope.
        if project_id.is_empty() && session_id.is_empty() {
            let root = self.root.join(agent_id);
            return walk_tree(&root, &root, "").await;
        }
        let scope = scope_dir(&self.root, agent_id, project_id, session_id);
        walk_tree(&scope, &scope, "").await
    }

    async fn delete(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        let full = self.full_path(agent_id, project_id, session_id, path);
        match tokio::fs::remove_file(&full).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(WorkspaceError::NotFound),
            Err(e) => Err(WorkspaceError::Io(e)),
        }
    }

    async fn move_scope(
        &self,
        agent_id: &str,
        from_project_id: &str,
        from_session_id: &str,
        to_project_id: &str,
        to_session_id: &str,
    ) -> Result<(), WorkspaceError> {
        let from = scope_dir(&self.root, agent_id, from_project_id, from_session_id);
        let to = scope_dir(&self.root, agent_id, to_project_id, to_session_id);
        if !from.exists() {
            return Ok(()); // no-op
        }
        if to.exists() {
            // Check non-empty.
            let mut entries = tokio::fs::read_dir(&to).await?;
            if entries.next_entry().await?.is_some() {
                return Err(WorkspaceError::MoveDestinationExists);
            }
        }
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&from, &to).await?;
        Ok(())
    }
}

/// Walk a tree and return ObjectInfo for every regular file found. The
/// returned `path` is relative to `root`.
async fn walk_tree(root: &Path, base: &Path, rel: &str) -> Result<Vec<ObjectInfo>, WorkspaceError> {
    let mut out = Vec::new();
    // Each stack frame tracks (path, relative-from-base).
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), rel.to_string())];
    while let Some((dir, rel)) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(WorkspaceError::Io(e)),
        };
        while let Some(entry) = entries.next_entry().await? {
            let p = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let child_rel = if rel.is_empty() {
                name.to_string()
            } else {
                format!("{rel}/{name}")
            };
            let ft = match entry.file_type().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push((p, child_rel));
            } else if ft.is_file() {
                let meta = entry.metadata().await?;
                let mod_time = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        let dt: DateTime<Utc> = (std::time::UNIX_EPOCH + d).into();
                        dt
                    })
                    .unwrap_or_else(Utc::now);
                let full = base.join(&child_rel);
                out.push(ObjectInfo {
                    path: child_rel,
                    size: meta.len() as i64,
                    content_type: sniff_content_type(&full),
                    mod_time,
                });
            }
        }
    }
    Ok(out)
}

// =====================================================================
// S3 — stub. Wire a real client in the gateway crate.
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    pub use_ssl: bool,
}

/// Stub S3 backend. Returns `SignedUrlUnsupported` for `signed_url` and
/// `Other("not implemented")` for the I/O methods. Use this only for
/// the Factory wiring tests; a real S3 client lives in the gateway.
pub struct S3 {
    config: S3Config,
}

impl S3 {
    pub fn new(config: S3Config) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &S3Config {
        &self.config
    }
}

#[async_trait]
impl Store for S3 {
    async fn put(
        &self,
        _agent_id: &str,
        _project_id: &str,
        _session_id: &str,
        _path: &str,
        _data: Bytes,
        _content_type: &str,
    ) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::Other(
            "cleanclaw-workspace S3 backend is a stub; use the gateway's full S3 client".into(),
        ))
    }
    async fn get(
        &self,
        _a: &str,
        _p: &str,
        _s: &str,
        _path: &str,
    ) -> Result<Bytes, WorkspaceError> {
        Err(WorkspaceError::Other("s3 stub".into()))
    }
    async fn stat(
        &self,
        _a: &str,
        _p: &str,
        _s: &str,
        _path: &str,
    ) -> Result<ObjectInfo, WorkspaceError> {
        Err(WorkspaceError::Other("s3 stub".into()))
    }
    async fn list(&self, _a: &str, _p: &str, _s: &str) -> Result<Vec<ObjectInfo>, WorkspaceError> {
        Err(WorkspaceError::Other("s3 stub".into()))
    }
    async fn delete(
        &self,
        _a: &str,
        _p: &str,
        _s: &str,
        _path: &str,
    ) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::Other("s3 stub".into()))
    }
    async fn move_scope(
        &self,
        _a: &str,
        _fp: &str,
        _fs: &str,
        _tp: &str,
        _ts: &str,
    ) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::Other("s3 stub".into()))
    }
}

// =====================================================================
// Factory — config-driven backend selection.
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct Factory {
    /// One of: "", "local", "aws-s3", "cloudflare-r2", "backblaze-b2",
    /// "aliyun-oss", "minio", "s3".
    pub r#type: String,
    pub local_dir: String,
    pub s3: S3Config,
    pub account_id: String,
    pub aliyun_intern: bool,
}

impl Factory {
    pub fn new(r#type: impl Into<String>) -> Self {
        Self {
            r#type: r#type.into(),
            ..Default::default()
        }
    }

    pub fn with_local_dir(mut self, dir: impl Into<String>) -> Self {
        self.local_dir = dir.into();
        self
    }

    pub fn with_s3(mut self, s3: S3Config) -> Self {
        self.s3 = s3;
        self
    }

    pub fn with_account_id(mut self, id: impl Into<String>) -> Self {
        self.account_id = id.into();
        self
    }

    pub fn with_aliyun_intern(mut self, b: bool) -> Self {
        self.aliyun_intern = b;
        self
    }

    /// Build a Store. `default_local_dir` is the fallback root when
    /// `local_dir` is empty.
    pub fn build(&self, default_local_dir: &str) -> Result<Arc<dyn Store>, WorkspaceError> {
        match self.r#type.as_str() {
            "" | "local" => {
                let root = if self.local_dir.is_empty() {
                    default_local_dir.to_string()
                } else {
                    self.local_dir.clone()
                };
                Ok(Arc::new(LocalFs::new(root)))
            }
            "aws-s3" | "cloudflare-r2" | "backblaze-b2" | "aliyun-oss" | "minio" | "s3" => {
                let mut s3 = self.s3.clone();
                if s3.endpoint.is_empty() {
                    let ep = default_endpoint(
                        &self.r#type,
                        &s3.region,
                        &self.account_id,
                        self.aliyun_intern,
                    )?;
                    s3.endpoint = ep;
                }
                if self.r#type != "minio" && !s3.use_ssl {
                    s3.use_ssl = true;
                }
                Ok(Arc::new(S3::new(s3)))
            }
            other => Err(WorkspaceError::UnknownType(other.to_string())),
        }
    }
}

fn default_endpoint(
    provider: &str,
    region: &str,
    account_id: &str,
    aliyun_internal: bool,
) -> Result<String, WorkspaceError> {
    match provider {
        "aws-s3" => {
            if region.is_empty() {
                return Err(WorkspaceError::MissingRegion("aws-s3"));
            }
            Ok(format!("s3.{region}.amazonaws.com"))
        }
        "cloudflare-r2" => {
            if account_id.is_empty() {
                return Err(WorkspaceError::MissingAccountId("cloudflare-r2"));
            }
            Ok(format!("{account_id}.r2.cloudflarestorage.com"))
        }
        "backblaze-b2" => {
            if region.is_empty() {
                return Err(WorkspaceError::MissingRegion("backblaze-b2"));
            }
            Ok(format!("s3.{region}.backblazeb2.com"))
        }
        "aliyun-oss" => {
            if region.is_empty() {
                return Err(WorkspaceError::MissingRegion("aliyun-oss"));
            }
            if aliyun_internal {
                Ok(format!("oss-{region}-internal.aliyuncs.com"))
            } else {
                Ok(format!("oss-{region}.aliyuncs.com"))
            }
        }
        "minio" | "s3" => Err(WorkspaceError::MissingEndpoint(provider_static(provider))),
        _ => Err(WorkspaceError::UnknownType(provider.to_string())),
    }
}

fn provider_static(p: &str) -> &'static str {
    match p {
        "minio" => "minio",
        "s3" => "s3",
        _ => "unknown",
    }
}

// =====================================================================
// Metered — wraps a Store to count Put bytes per agent.
// =====================================================================

#[derive(Clone)]
pub struct Metered {
    inner: Arc<dyn Store>,
    meter: Arc<dyn Fn(&str, i64) + Send + Sync + 'static>,
}

impl Metered {
    pub fn new<F>(inner: Arc<dyn Store>, meter: F) -> Self
    where
        F: Fn(&str, i64) + Send + Sync + 'static,
    {
        Self {
            inner,
            meter: Arc::new(meter),
        }
    }

    pub fn inner(&self) -> &Arc<dyn Store> {
        &self.inner
    }
}

#[async_trait]
impl Store for Metered {
    async fn put(
        &self,
        agent_id: &str,
        project_id: &str,
        session_id: &str,
        path: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<(), WorkspaceError> {
        let len = data.len() as i64;
        self.inner
            .put(agent_id, project_id, session_id, path, data, content_type)
            .await?;
        (self.meter)(agent_id, len);
        Ok(())
    }
    async fn get(&self, a: &str, p: &str, s: &str, path: &str) -> Result<Bytes, WorkspaceError> {
        self.inner.get(a, p, s, path).await
    }
    async fn stat(
        &self,
        a: &str,
        p: &str,
        s: &str,
        path: &str,
    ) -> Result<ObjectInfo, WorkspaceError> {
        self.inner.stat(a, p, s, path).await
    }
    async fn list(&self, a: &str, p: &str, s: &str) -> Result<Vec<ObjectInfo>, WorkspaceError> {
        self.inner.list(a, p, s).await
    }
    async fn delete(&self, a: &str, p: &str, s: &str, path: &str) -> Result<(), WorkspaceError> {
        self.inner.delete(a, p, s, path).await
    }
    async fn move_scope(
        &self,
        a: &str,
        fp: &str,
        fs: &str,
        tp: &str,
        ts: &str,
    ) -> Result<(), WorkspaceError> {
        self.inner.move_scope(a, fp, fs, tp, ts).await
    }
    async fn signed_url(
        &self,
        a: &str,
        p: &str,
        s: &str,
        path: &str,
        ttl: std::time::Duration,
    ) -> Result<String, WorkspaceError> {
        self.inner.signed_url(a, p, s, path, ttl).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-workspace-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn local_put_get_round_trip() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        store
            .put(
                "a1",
                "",
                "s1",
                "report.pdf",
                Bytes::from_static(b"PDFDATA"),
                "application/pdf",
            )
            .await
            .unwrap();
        let got = store.get("a1", "", "s1", "report.pdf").await.unwrap();
        assert_eq!(got.as_ref(), b"PDFDATA");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_stat_returns_size_and_type() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        store
            .put("a1", "", "s1", "img.png", Bytes::from_static(&[0; 10]), "")
            .await
            .unwrap();
        let info = store.stat("a1", "", "s1", "img.png").await.unwrap();
        assert_eq!(info.size, 10);
        assert_eq!(info.content_type, "image/png");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_get_missing_returns_not_found() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        let err = store.get("a1", "", "s1", "nope.txt").await.unwrap_err();
        assert!(matches!(err, WorkspaceError::NotFound));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_list_returns_relative_paths() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        store
            .put("a1", "", "s1", "a.txt", Bytes::from_static(b"a"), "")
            .await
            .unwrap();
        store
            .put("a1", "", "s1", "sub/b.txt", Bytes::from_static(b"b"), "")
            .await
            .unwrap();
        let list = store.list("a1", "", "s1").await.unwrap();
        let paths: Vec<_> = list.iter().map(|o| o.path.clone()).collect();
        assert!(paths.contains(&"a.txt".to_string()));
        assert!(paths.contains(&"sub/b.txt".to_string()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_delete_removes_file() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        store
            .put("a1", "", "s1", "x", Bytes::from_static(b"x"), "")
            .await
            .unwrap();
        store.delete("a1", "", "s1", "x").await.unwrap();
        let err = store.get("a1", "", "s1", "x").await.unwrap_err();
        assert!(matches!(err, WorkspaceError::NotFound));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_move_scope_relocates_files() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        store
            .put("a1", "", "s1", "f", Bytes::from_static(b"f"), "")
            .await
            .unwrap();
        store
            .move_scope("a1", "", "s1", "proj1", "s1")
            .await
            .unwrap();
        // Old scope is gone.
        let err = store.get("a1", "", "s1", "f").await.unwrap_err();
        assert!(matches!(err, WorkspaceError::NotFound));
        // New scope has the file.
        let got = store.get("a1", "proj1", "s1", "f").await.unwrap();
        assert_eq!(got.as_ref(), b"f");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_move_refuses_existing_destination() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        store
            .put("a1", "", "s1", "a", Bytes::from_static(b"a"), "")
            .await
            .unwrap();
        store
            .put("a1", "p1", "s1", "b", Bytes::from_static(b"b"), "")
            .await
            .unwrap();
        let err = store
            .move_scope("a1", "", "s1", "p1", "s1")
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::MoveDestinationExists));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_signed_url_unsupported() {
        let root = tmpdir();
        let store = LocalFs::new(root.clone());
        let err = store
            .signed_url("a1", "", "s1", "f", std::time::Duration::from_secs(60))
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::SignedUrlUnsupported));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn factory_local_default_dir() {
        let f = Factory::new("local").with_local_dir("/var/lib/ws");
        let s = f.build("/default").unwrap();
        // Verify the underlying local_fs is rooted at our dir.
        let s_local = s
            .as_any()
            .and_then(|a| a.downcast_ref::<LocalFs>())
            .unwrap_or_else(|| panic!("not a LocalFs"));
        assert_eq!(s_local.root(), Path::new("/var/lib/ws"));
    }

    #[tokio::test]
    async fn factory_unknown_type_errors() {
        let err = match Factory::new("not-a-backend").build("/d") {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(matches!(err, WorkspaceError::UnknownType(_)));
    }

    #[test]
    fn factory_aws_s3_endpoint_derivation() {
        let f = Factory::new("aws-s3");
        let s = f
            .with_s3(S3Config {
                region: "us-east-1".into(),
                bucket: "b".into(),
                ..Default::default()
            })
            .build("/d")
            .unwrap();
        // Just check it builds without erroring.
        let _ = s;
    }

    #[test]
    fn factory_aws_s3_requires_region() {
        let f = Factory::new("aws-s3");
        let err = {
            match f.build("/d") {
                Err(e) => e,
                Ok(_) => panic!("expected error"),
            }
        };
        assert!(matches!(err, WorkspaceError::MissingRegion("aws-s3")));
    }

    #[test]
    fn factory_r2_requires_account_id() {
        let f = Factory::new("cloudflare-r2");
        let err = {
            match f.build("/d") {
                Err(e) => e,
                Ok(_) => panic!("expected error"),
            }
        };
        assert!(matches!(
            err,
            WorkspaceError::MissingAccountId("cloudflare-r2")
        ));
    }

    #[test]
    fn factory_r2_uses_account_id_endpoint() {
        let f = Factory::new("cloudflare-r2")
            .with_account_id("abc123")
            .with_s3(S3Config {
                bucket: "b".into(),
                ..Default::default()
            });
        let s = f.build("/d").unwrap();
        let _ = s;
    }

    #[test]
    fn factory_aliyun_internal_endpoint() {
        let f = Factory::new("aliyun-oss")
            .with_aliyun_intern(true)
            .with_s3(S3Config {
                region: "cn-hangzhou".into(),
                bucket: "b".into(),
                ..Default::default()
            });
        let s = f.build("/d").unwrap();
        let _ = s;
    }

    #[test]
    fn factory_minio_requires_endpoint() {
        let f = Factory::new("minio");
        let err = {
            match f.build("/d") {
                Err(e) => e,
                Ok(_) => panic!("expected error"),
            }
        };
        assert!(matches!(err, WorkspaceError::MissingEndpoint("minio")));
    }

    #[test]
    fn factory_minio_with_endpoint_builds() {
        let f = Factory::new("minio").with_s3(S3Config {
            endpoint: "http://minio.local:9000".into(),
            bucket: "b".into(),
            access_key: "x".into(),
            secret_key: "y".into(),
            ..Default::default()
        });
        let s = f.build("/d").unwrap();
        let _ = s;
    }

    #[tokio::test]
    async fn metered_counts_put_bytes() {
        let root = tmpdir();
        let inner = Arc::new(LocalFs::new(root.clone()));
        let total = Arc::new(AtomicI64::new(0));
        let t = total.clone();
        let metered = Metered::new(inner, move |_agent, bytes| {
            t.fetch_add(bytes, Ordering::SeqCst);
        });
        metered
            .put("a1", "", "s", "f1", Bytes::from_static(&[0; 100]), "")
            .await
            .unwrap();
        metered
            .put("a1", "", "s", "f2", Bytes::from_static(&[0; 50]), "")
            .await
            .unwrap();
        assert_eq!(total.load(Ordering::SeqCst), 150);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sniff_default_octet_stream() {
        assert_eq!(
            sniff_content_type(Path::new("x.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn sniff_picks_known_extensions() {
        assert_eq!(sniff_content_type(Path::new("a.png")), "image/png");
        assert_eq!(sniff_content_type(Path::new("a.html")), "text/html");
        assert_eq!(sniff_content_type(Path::new("a.pdf")), "application/pdf");
    }

    #[test]
    fn scope_dir_layouts() {
        let root = Path::new("/w");
        assert_eq!(scope_dir(root, "a", "", ""), PathBuf::from("/w/a"));
        assert_eq!(
            scope_dir(root, "a", "", "s1"),
            PathBuf::from("/w/a/sessions/s1")
        );
        assert_eq!(
            scope_dir(root, "a", "p1", "s1"),
            PathBuf::from("/w/a/projects/p1")
        );
    }
}

// We need a way to downcast `Arc<dyn Store>` to concrete LocalFs in
// tests. The `as_any` method on the `Store` trait above lets concrete
// backends override; `dyn Store` callers go through it.
