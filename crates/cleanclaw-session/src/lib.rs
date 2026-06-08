//! Session manager.
//!
//! Two parallel persistence shapes:
//!   - `Get` / `Save` operate on the LLM-facing working set (post-compaction).
//!     This is what the agent loop reads/writes every turn.
//!   - `append_message` / `archived_messages` operate on the append-only
//!     per-turn archive (`session_messages` table). Compaction never
//!     touches it, so UI history / audit reads see the original
//!     conversation regardless of how many times the working set has
//!     been pruned/summarized.

use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use cleanclaw_provider::message::Message as ProviderMessage;
use cleanclaw_store::models::{SessionMessageRecord, SessionRecord};
use cleanclaw_store::Store;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("store: {0}")]
    Store(#[from] cleanclaw_core::CleanClawError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("userID is required")]
    MissingUser,
}

/// Storage abstraction. Two implementations:
///   - `StoreAdapter` — wraps a `cleanclaw_store::Store` for production.
///   - `FileBackend` — scans a directory of `web_*.jsonl` files for
///     single-binary dev installs. Mirrors the Go "file-only mode".
#[async_trait::async_trait]
pub trait SessionBackend: Send + Sync + 'static {
    /// Read the working-set messages for a session.
    async fn get_session(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<ProviderMessage>, SessionError>;
    /// Overwrite the working-set messages. Channel/triple/project are
    /// upserted alongside.
    #[allow(clippy::too_many_arguments)]
    async fn save_session(
        &self,
        agent_id: &str,
        session_key: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        project_id: &str,
        chatter_user_id: &str,
        messages: &[ProviderMessage],
    ) -> Result<(), SessionError>;
    /// Append one message to the audit archive.
    async fn append_message(
        &self,
        agent_id: &str,
        session_key: &str,
        msg: &ProviderMessage,
    ) -> Result<(), SessionError>;
    /// List the audit archive.
    async fn list_messages(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<ProviderMessage>, SessionError>;
    /// Drop a session row + its archive.
    async fn delete_session(&self, agent_id: &str, session_key: &str) -> Result<(), SessionError>;
    /// Set the human-readable title.
    async fn rename_session(
        &self,
        agent_id: &str,
        session_key: &str,
        title: &str,
    ) -> Result<(), SessionError>;
    /// Reassign project_id (or detach with "").
    async fn move_session(
        &self,
        agent_id: &str,
        session_key: &str,
        project_id: &str,
    ) -> Result<(), SessionError>;
    /// Most recent session_key for the (channel, account, chat) triple.
    async fn resolve_active_session_key(
        &self,
        agent_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
    ) -> Result<Option<String>, SessionError>;
    /// session_key → (channel, account_id, chat_id) inverse lookup.
    async fn lookup_triple(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> Result<(String, String, String), SessionError>;
    /// session_key → project_id.
    async fn lookup_project(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> Result<String, SessionError>;
    /// List sidebar sessions (one row per active chat, newest first).
    async fn list_web_sessions(&self, agent_id: &str) -> Result<Vec<WebSession>, SessionError>;
}

/// One chat surfaced to the dashboard. ID is the session_key (the row
/// PK), not the chat_id. Older URLs pointing at a chat_id still resolve
/// via the agent-side fallback (`resolve_session_key`) so existing
/// bookmarks don't break.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSession {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel: String,
    #[serde(
        rename = "accountId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub account_id: String,
    #[serde(rename = "chatId", default, skip_serializing_if = "String::is_empty")]
    pub chat_id: String,
    #[serde(
        rename = "projectId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub project_id: String,
    pub title: String,
    pub preview: String,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    #[serde(
        rename = "thumbnailUrl",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub thumbnail_url: String,
}

// =====================================================================
// Session — the in-memory state for one conversation thread.
// =====================================================================

pub struct Session {
    #[allow(dead_code)]
    file_path: PathBuf,
    backend: Arc<dyn SessionBackend>,
    #[allow(dead_code)]
    user_id: String,
    agent_id: String,
    session_key: String,
    channel: Mutex<String>,
    account_id: Mutex<String>,
    chat_id: Mutex<String>,
    project_id: Mutex<String>,
    chatter_user_id: Mutex<String>,
    messages: Mutex<Vec<ProviderMessage>>,
    last_consolidated: Mutex<usize>,
    snapshot: Mutex<Option<Vec<ProviderMessage>>>,
    turn_depth: Mutex<usize>,
    steer_buf: Mutex<Vec<ProviderMessage>>,
}

impl Session {
    pub fn key(&self) -> &str {
        &self.session_key
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub async fn channel(&self) -> String {
        self.channel.lock().await.clone()
    }

    pub async fn chat_id(&self) -> String {
        self.chat_id.lock().await.clone()
    }

    pub async fn project_id(&self) -> String {
        self.project_id.lock().await.clone()
    }

    pub async fn chatter_user_id(&self) -> String {
        self.chatter_user_id.lock().await.clone()
    }

    /// Bind the per-turn conversation participant so the next save
    /// stamps `chatter_user_id`. Pass `""` to clear.
    pub async fn set_chatter(&self, uid: &str) {
        *self.chatter_user_id.lock().await = uid.to_string();
    }

    /// Append a message and persist it. Best-effort archive write
    /// (logged on failure) — losing one archive row is recoverable
    /// from the working set.
    pub async fn append(&self, mut msg: ProviderMessage) {
        if msg.timestamp.is_none() {
            msg.timestamp = Some(Utc::now().timestamp_millis());
        }
        let chatter = self.chatter_user_id.lock().await.clone();
        let channel = self.channel.lock().await.clone();
        let account_id = self.account_id.lock().await.clone();
        let chat_id = self.chat_id.lock().await.clone();
        let project_id = self.project_id.lock().await.clone();
        {
            let mut msgs = self.messages.lock().await;
            msgs.push(msg.clone());
        }
        let messages = self.messages.lock().await.clone();
        if let Err(e) = self
            .backend
            .save_session(
                &self.agent_id,
                &self.session_key,
                &channel,
                &account_id,
                &chat_id,
                &project_id,
                &chatter,
                &messages,
            )
            .await
        {
            tracing::warn!(error = %e, "session: save_session failed");
        }
        if let Err(e) = self
            .backend
            .append_message(&self.agent_id, &self.session_key, &msg)
            .await
        {
            tracing::warn!(error = %e, "session: archive append failed");
        }
        let _ = chatter; // silence unused if no backend
    }

    /// Full append-only archive (falls back to working set when no
    /// archive is configured or the archive is empty).
    pub async fn archived_messages(&self) -> Vec<ProviderMessage> {
        match self
            .backend
            .list_messages(&self.agent_id, &self.session_key)
            .await
        {
            Ok(v) if !v.is_empty() => v,
            _ => self.get_messages().await,
        }
    }

    pub async fn get_messages(&self) -> Vec<ProviderMessage> {
        self.messages.lock().await.clone()
    }

    /// Steering: turnDepth counts in-flight HandleMessage turns. A
    /// counter rather than a bool so overlapping turns don't strand
    /// the active flag. Steer messages are only accepted while at
    /// least one turn is active.
    pub async fn begin_turn(&self) {
        *self.turn_depth.lock().await += 1;
    }

    /// End the in-flight turn. Returns any leftover steer messages
    /// when the last in-flight turn ends (end-of-turn race: a message
    /// pushed after the loop's final drain).
    pub async fn end_turn(&self) -> Vec<ProviderMessage> {
        let mut depth = self.turn_depth.lock().await;
        if *depth > 0 {
            *depth -= 1;
        }
        if *depth > 0 {
            return Vec::new();
        }
        let mut buf = self.steer_buf.lock().await;
        if buf.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut *buf)
    }

    /// Buffer a steering message iff a turn is in-flight. Returns
    /// `false` when no turn is active so the caller can fall back to
    /// dispatching the message as a fresh turn.
    pub async fn push_steer_if_active(&self, msg: ProviderMessage) -> bool {
        let depth = *self.turn_depth.lock().await;
        if depth == 0 {
            return false;
        }
        self.steer_buf.lock().await.push(msg);
        true
    }

    /// Atomically return and clear the buffered steer messages. The
    /// running loop calls this between tool iterations.
    pub async fn drain_steer(&self) -> Vec<ProviderMessage> {
        let mut buf = self.steer_buf.lock().await;
        if buf.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut *buf)
    }

    pub async fn unconsolidated_count(&self) -> usize {
        let msgs = self.messages.lock().await;
        let lc = *self.last_consolidated.lock().await;
        msgs.len().saturating_sub(lc)
    }

    pub async fn mark_consolidated(&self, index: usize) {
        *self.last_consolidated.lock().await = index;
    }

    /// Replace all messages (used after context compaction). Persists
    /// the new working set.
    pub async fn replace_messages(&self, msgs: Vec<ProviderMessage>) {
        {
            let mut m = self.messages.lock().await;
            *m = msgs;
        }
        *self.last_consolidated.lock().await = 0;
        let chatter = self.chatter_user_id.lock().await.clone();
        let channel = self.channel.lock().await.clone();
        let account_id = self.account_id.lock().await.clone();
        let chat_id = self.chat_id.lock().await.clone();
        let project_id = self.project_id.lock().await.clone();
        let messages = self.messages.lock().await.clone();
        if let Err(e) = self
            .backend
            .save_session(
                &self.agent_id,
                &self.session_key,
                &channel,
                &account_id,
                &chat_id,
                &project_id,
                &chatter,
                &messages,
            )
            .await
        {
            tracing::warn!(error = %e, "session: save_session (replace) failed");
        }
    }

    pub async fn clear(&self) {
        {
            let mut m = self.messages.lock().await;
            m.clear();
        }
        *self.last_consolidated.lock().await = 0;
        if let Err(e) = self
            .backend
            .delete_session(&self.agent_id, &self.session_key)
            .await
        {
            tracing::warn!(error = %e, "session: delete_session failed");
        }
    }

    /// Save the current message list as a restore point (for undo).
    pub async fn snapshot(&self) {
        let cur = self.messages.lock().await.clone();
        *self.snapshot.lock().await = Some(cur);
    }

    /// Restore the last snapshot. Returns false if no snapshot exists.
    pub async fn undo(&self) -> bool {
        let snap = self.snapshot.lock().await.take();
        if snap.is_none() {
            return false;
        }
        let snap = snap.unwrap();
        {
            let mut m = self.messages.lock().await;
            *m = snap;
        }
        *self.last_consolidated.lock().await = 0;
        true
    }

    pub async fn has_snapshot(&self) -> bool {
        self.snapshot.lock().await.is_some()
    }
}

// =====================================================================
// Manager — owns the in-memory cache + key resolution policy.
// =====================================================================

pub struct Manager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// For file mode, track the most recently minted session_key per
    /// (channel, account, chat) triple so `open_new_session` is sticky:
    /// the next `get` for that triple returns the new key rather than
    /// minting yet another. Mirrors the DB-side "active session" row.
    active_keys: Mutex<HashMap<String, String>>,
    data_dir: PathBuf,
    backend: Arc<dyn SessionBackend>,
    user_id: String,
    agent_id: String,
}

impl Manager {
    /// File-backed dev mode. No DB.
    pub fn new(data_dir: impl Into<PathBuf>) -> Arc<Self> {
        let dir = data_dir.into();
        let backend: Arc<dyn SessionBackend> = Arc::new(FileBackend::new(dir.clone()));
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            active_keys: Mutex::new(HashMap::new()),
            data_dir: dir,
            backend,
            user_id: String::new(),
            agent_id: String::new(),
        })
    }

    /// Store-backed production mode. `user_id` is required and used to
    /// scope store SQL.
    pub fn with_backend(
        backend: Arc<dyn SessionBackend>,
        user_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            active_keys: Mutex::new(HashMap::new()),
            data_dir: PathBuf::new(),
            backend,
            user_id: user_id.into(),
            agent_id: agent_id.into(),
        })
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Returns the active session for the (channel, accountID, chatID)
    /// triple, creating it if necessary.
    pub async fn get(
        self: &Arc<Self>,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        project_id: &str,
    ) -> Arc<Session> {
        let key = self.resolve_or_mint_key(channel, account_id, chat_id).await;
        let s = self
            .get_by_key_inner(&key, channel, account_id, chat_id, project_id)
            .await;
        // Record the active key for file-mode (where the backend
        // doesn't persist it). For store mode this is a no-op
        // (resolve_active_session_key would have returned Some).
        self.record_active_key(channel, account_id, chat_id, &key)
            .await;
        s
    }

    pub async fn get_by_key(self: &Arc<Self>, session_key: &str) -> Arc<Session> {
        self.get_by_key_inner(session_key, "", "", "", "").await
    }

    async fn get_by_key_inner(
        self: &Arc<Self>,
        key: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        project_id: &str,
    ) -> Arc<Session> {
        let key = key.to_string();
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions.get(&key).cloned() {
            // Reload from backend on every Get so multi-replica writes
            // are seen (matches the Go behavior).
            if let Ok(msgs) = self.backend.get_session(&self.agent_id, &key).await {
                let mut m = s.messages.lock().await;
                *m = msgs;
            }
            // Late-bind triple + project on cached entries.
            if !channel.is_empty() || !project_id.is_empty() {
                let mut ch = s.channel.lock().await.clone();
                let mut ac = s.account_id.lock().await.clone();
                let mut ci = s.chat_id.lock().await.clone();
                let mut pi = s.project_id.lock().await.clone();
                if ch.is_empty() && !channel.is_empty() {
                    ch = channel.to_string();
                    ac = account_id.to_string();
                    ci = chat_id.to_string();
                }
                if pi.is_empty() && !project_id.is_empty() {
                    pi = project_id.to_string();
                }
                *s.channel.lock().await = ch;
                *s.account_id.lock().await = ac;
                *s.chat_id.lock().await = ci;
                *s.project_id.lock().await = pi;
                return s;
            }
            return s;
        }

        // First load.
        let file_path = if channel == "web" {
            self.data_dir.join(format!("web_{key}.jsonl"))
        } else {
            self.data_dir.join(format!("{key}.jsonl"))
        };
        let initial_messages = self
            .backend
            .get_session(&self.agent_id, &key)
            .await
            .unwrap_or_default();
        let s = Arc::new(Session {
            file_path,
            backend: self.backend.clone(),
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            session_key: key.clone(),
            channel: Mutex::new(channel.to_string()),
            account_id: Mutex::new(account_id.to_string()),
            chat_id: Mutex::new(chat_id.to_string()),
            project_id: Mutex::new(project_id.to_string()),
            chatter_user_id: Mutex::new(String::new()),
            messages: Mutex::new(initial_messages),
            last_consolidated: Mutex::new(0),
            snapshot: Mutex::new(None),
            turn_depth: Mutex::new(0),
            steer_buf: Mutex::new(Vec::new()),
        });
        sessions.insert(key, s.clone());
        s
    }

    async fn resolve_or_mint_key(&self, channel: &str, account_id: &str, chat_id: &str) -> String {
        if let Ok(Some(k)) = self
            .backend
            .resolve_active_session_key(&self.agent_id, channel, account_id, chat_id)
            .await
        {
            return k;
        }
        let triple = format!("{channel}|{account_id}|{chat_id}");
        if let Some(k) = self.active_keys.lock().await.get(&triple) {
            return k.clone();
        }
        if channel == "web" && !chat_id.is_empty() {
            return chat_id.to_string();
        }
        generate_session_key()
    }

    async fn record_active_key(&self, channel: &str, account_id: &str, chat_id: &str, key: &str) {
        let triple = format!("{channel}|{account_id}|{chat_id}");
        self.active_keys
            .lock()
            .await
            .insert(triple, key.to_string());
    }

    pub async fn lookup_project(&self, session_key: &str) -> String {
        if session_key.is_empty() {
            return String::new();
        }
        self.backend
            .lookup_project(&self.agent_id, session_key)
            .await
            .unwrap_or_default()
    }

    pub async fn lookup_triple(
        &self,
        session_key: &str,
    ) -> Result<(String, String, String), SessionError> {
        self.backend
            .lookup_triple(&self.agent_id, session_key)
            .await
    }

    pub async fn session_exists(&self, session_key: &str) -> bool {
        if session_key.is_empty() {
            return false;
        }
        // FileBackend has no negative-lookup primitive; the Go code
        // returns `true` (assume-yes) so legacy chat_id fallback isn't
        // preferred over the caller's intent. Match that here.
        if self.backend.as_ref().type_id() == std::any::TypeId::of::<FileBackend>() {
            return true;
        }
        self.backend
            .get_session(&self.agent_id, session_key)
            .await
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    pub async fn resolve_session_key(&self, session_id: &str) -> String {
        if session_id.is_empty() {
            return String::new();
        }
        if self.session_exists(session_id).await {
            return session_id.to_string();
        }
        if let Ok(Some(k)) = self
            .backend
            .resolve_active_session_key(&self.agent_id, "web", "", session_id)
            .await
        {
            return k;
        }
        // Check our own active_keys cache (file mode) — the chat_id
        // may have been minted into a session_key by open_new_session.
        let triple = format!("web|{session_id}");
        if let Some(k) = self
            .active_keys
            .lock()
            .await
            .get(&format!("web||{session_id}"))
        {
            return k.clone();
        }
        let _ = triple;
        session_id.to_string()
    }

    /// Mint a brand-new session under the (channel, accountID, chatID)
    /// triple. The next `get` for that triple will pick it up because
    /// it has the freshest `updated_at`.
    pub async fn open_new_session(
        self: &Arc<Self>,
        channel: &str,
        account_id: &str,
        chat_id: &str,
    ) -> String {
        let key = generate_session_key();
        // Persist an empty row so the active-session lookup resolves
        // to this key, not the previous one.
        let _ = self
            .backend
            .save_session(
                &self.agent_id,
                &key,
                channel,
                account_id,
                chat_id,
                "",
                "",
                &[],
            )
            .await;
        // Record the active key for file mode where the backend
        // doesn't persist it.
        self.record_active_key(channel, account_id, chat_id, &key)
            .await;
        let file_path = if channel == "web" {
            self.data_dir.join(format!("web_{key}.jsonl"))
        } else {
            self.data_dir.join(format!("{key}.jsonl"))
        };
        let s = Arc::new(Session {
            file_path,
            backend: self.backend.clone(),
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            session_key: key.clone(),
            channel: Mutex::new(channel.to_string()),
            account_id: Mutex::new(account_id.to_string()),
            chat_id: Mutex::new(chat_id.to_string()),
            project_id: Mutex::new(String::new()),
            chatter_user_id: Mutex::new(String::new()),
            messages: Mutex::new(Vec::new()),
            last_consolidated: Mutex::new(0),
            snapshot: Mutex::new(None),
            turn_depth: Mutex::new(0),
            steer_buf: Mutex::new(Vec::new()),
        });
        self.sessions.lock().await.insert(key.clone(), s);
        key
    }

    pub async fn list_web_sessions(&self) -> Vec<WebSession> {
        self.backend
            .list_web_sessions(&self.agent_id)
            .await
            .unwrap_or_default()
    }

    pub async fn delete_session_by_id(&self, session_id: &str) -> Result<(), SessionError> {
        let key = self.resolve_session_key(session_id).await;
        self.sessions.lock().await.remove(&key);
        self.backend.delete_session(&self.agent_id, &key).await
    }

    pub async fn rename_session_by_id(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<(), SessionError> {
        let key = self.resolve_session_key(session_id).await;
        self.backend
            .rename_session(&self.agent_id, &key, title)
            .await
    }

    pub async fn move_session_by_id(
        &self,
        session_id: &str,
        project_id: &str,
    ) -> Result<(), SessionError> {
        let key = self.resolve_session_key(session_id).await;
        {
            let sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get(&key) {
                *s.project_id.lock().await = project_id.to_string();
            }
        }
        self.backend
            .move_session(&self.agent_id, &key, project_id)
            .await
    }

    /// Resolve a chatId → session_key. Falls back to the legacy
    /// `web_<id>` form when no row exists.
    pub async fn resolve_web_session_key(&self, chat_id: &str) -> String {
        if let Ok(Some(k)) = self
            .backend
            .resolve_active_session_key(&self.agent_id, "web", "", chat_id)
            .await
        {
            return k;
        }
        format!("web_{chat_id}")
    }
}

fn generate_session_key() -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    let suffix: String = buf
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    let now = Utc::now().timestamp_millis();
    format!("s-{now}-{suffix}")
}

// =====================================================================
// FileBackend — dev-mode persistence (scans web_*.jsonl files).
// =====================================================================

pub struct FileBackend {
    data_dir: PathBuf,
}

impl FileBackend {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn safe_id(id: &str) -> String {
        id.replace('/', "_").replace("..", "_")
    }

    fn load_file(path: &Path) -> Vec<ProviderMessage> {
        let Ok(data) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in data.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(m) = serde_json::from_str::<ProviderMessage>(line) {
                out.push(m);
            }
        }
        out
    }

    #[allow(dead_code)]
    fn append_to_file(path: &Path, msg: &ProviderMessage) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let blob = serde_json::to_string(msg)?;
        use std::io::Write;
        writeln!(f, "{blob}")?;
        Ok(())
    }

    fn rewrite_file(path: &Path, msgs: &[ProviderMessage]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(path)?;
        use std::io::Write;
        for m in msgs {
            let blob = serde_json::to_string(m)?;
            writeln!(f, "{blob}")?;
        }
        Ok(())
    }

    fn meta_path(&self, session_id: &str) -> PathBuf {
        self.data_dir
            .join(format!("web_{}.meta.json", Self::safe_id(session_id)))
    }

    fn session_path(&self, channel: &str, session_key: &str) -> PathBuf {
        // Web-channel sessions use the `web_<key>.jsonl` filename
        // convention so the dashboard's scan can find them. Other
        // channels use the bare key.
        if channel == "web" {
            self.data_dir.join(format!("web_{session_key}.jsonl"))
        } else {
            self.data_dir.join(format!("{session_key}.jsonl"))
        }
    }
}

#[async_trait::async_trait]
impl SessionBackend for FileBackend {
    async fn get_session(
        &self,
        _agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<ProviderMessage>, SessionError> {
        // Try both naming conventions: explicit `web_` prefix and bare
        // key. Web-channel sessions use the prefixed form, but callers
        // passing a raw chat_id (no channel) need the bare fallback.
        let p1 = self.data_dir.join(format!("web_{session_key}.jsonl"));
        let msgs = Self::load_file(&p1);
        if !msgs.is_empty() {
            return Ok(msgs);
        }
        let p2 = self.data_dir.join(format!("{session_key}.jsonl"));
        Ok(Self::load_file(&p2))
    }

    async fn save_session(
        &self,
        _agent_id: &str,
        session_key: &str,
        channel: &str,
        _account_id: &str,
        _chat_id: &str,
        _project_id: &str,
        _chatter_user_id: &str,
        messages: &[ProviderMessage],
    ) -> Result<(), SessionError> {
        let path = self.session_path(channel, session_key);
        Self::rewrite_file(&path, messages)?;
        Ok(())
    }

    async fn append_message(
        &self,
        _agent_id: &str,
        _session_key: &str,
        _msg: &ProviderMessage,
    ) -> Result<(), SessionError> {
        // FileBackend has no separate archive — the working-set file
        // *is* the archive. `save_session` already wrote the full
        // list; appending again would double-write.
        Ok(())
    }

    async fn list_messages(
        &self,
        _agent_id: &str,
        _session_key: &str,
    ) -> Result<Vec<ProviderMessage>, SessionError> {
        Ok(Vec::new())
    }

    async fn delete_session(&self, _agent_id: &str, session_key: &str) -> Result<(), SessionError> {
        let path = self.data_dir.join(format!("{session_key}.jsonl"));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(self.meta_path(session_key));
        Ok(())
    }

    async fn rename_session(
        &self,
        _agent_id: &str,
        session_key: &str,
        title: &str,
    ) -> Result<(), SessionError> {
        let blob = serde_json::json!({ "title": title });
        std::fs::write(self.meta_path(session_key), blob.to_string())?;
        Ok(())
    }

    async fn move_session(
        &self,
        _agent_id: &str,
        _session_key: &str,
        _project_id: &str,
    ) -> Result<(), SessionError> {
        Ok(())
    }

    async fn resolve_active_session_key(
        &self,
        _agent_id: &str,
        _channel: &str,
        _account_id: &str,
        _chat_id: &str,
    ) -> Result<Option<String>, SessionError> {
        Ok(None)
    }

    async fn lookup_triple(
        &self,
        _agent_id: &str,
        _session_key: &str,
    ) -> Result<(String, String, String), SessionError> {
        Ok((String::new(), String::new(), String::new()))
    }

    async fn lookup_project(
        &self,
        _agent_id: &str,
        _session_key: &str,
    ) -> Result<String, SessionError> {
        Ok(String::new())
    }

    async fn list_web_sessions(&self, _agent_id: &str) -> Result<Vec<WebSession>, SessionError> {
        let pattern = self.data_dir.join("web_*.jsonl");
        let Ok(paths) = glob_simple(&pattern) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for path in paths {
            let Some(base) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let id = base
                .strip_prefix("web_")
                .and_then(|s| s.strip_suffix(".jsonl"))
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let msgs = Self::load_file(&path);
            let mut preview = String::new();
            let mut thumbnail = String::new();
            for m in &msgs {
                if !matches!(m.role, cleanclaw_provider::message::Role::User) {
                    continue;
                }
                let text = m.content.clone();
                if text.is_empty() {
                    for p in &m.content_parts {
                        if let cleanclaw_provider::message::ContentPart::Text { text: t } = p {
                            preview.push_str(t);
                            preview.push('\n');
                        }
                    }
                } else {
                    preview = text;
                }
                for p in &m.content_parts {
                    if let cleanclaw_provider::message::ContentPart::ImageUrl { url } = p {
                        if !url.is_empty() {
                            thumbnail = url.clone();
                        }
                    }
                }
                break;
            }
            preview = preview.trim().to_string();
            if preview.is_empty() {
                preview = if !thumbnail.is_empty() {
                    "[image]".into()
                } else {
                    continue;
                };
            }
            if preview.len() > 100 {
                preview = format!("{}...", &preview[..100]);
            }

            // Title from meta file, fallback to preview.
            let mut title = match std::fs::read_to_string(self.meta_path(&id)) {
                Ok(s) => serde_json::from_str::<serde_json::Value>(&s)
                    .ok()
                    .and_then(|v| v.get("title").and_then(|t| t.as_str().map(String::from)))
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            if title.is_empty() {
                title = preview.clone();
                if title.len() > 60 {
                    title = format!("{}...", &title[..60]);
                }
            }

            out.push(WebSession {
                id: id.clone(),
                channel: "web".into(),
                account_id: String::new(),
                chat_id: id,
                project_id: String::new(),
                title,
                preview,
                created_at: modified,
                updated_at: modified,
                thumbnail_url: thumbnail,
            });
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(out)
    }
}

fn glob_simple(pattern: &Path) -> std::io::Result<Vec<PathBuf>> {
    let pat = pattern.to_string_lossy();
    let Ok(entries) = glob::glob(&pat) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        out.push(entry);
    }
    Ok(out)
}

// =====================================================================
// StoreAdapter — bridges the cleanclaw_store::Store to SessionBackend.
// =====================================================================

pub struct StoreAdapter {
    store: Arc<dyn Store>,
}

impl StoreAdapter {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }
}

fn messages_from_value(v: &serde_json::Value) -> Vec<ProviderMessage> {
    match v {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| serde_json::from_value(item.clone()).ok())
            .collect(),
        serde_json::Value::Null => Vec::new(),
        _ => Vec::new(),
    }
}

fn messages_to_value(msgs: &[ProviderMessage]) -> serde_json::Value {
    serde_json::Value::Array(
        msgs.iter()
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
            .collect(),
    )
}

fn role_to_str(r: &cleanclaw_provider::message::Role) -> &'static str {
    use cleanclaw_provider::message::Role;
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn str_to_role(s: &str) -> cleanclaw_provider::message::Role {
    use cleanclaw_provider::message::Role;
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => Role::Tool,
    }
}

fn parts_to_value(parts: &[cleanclaw_provider::message::ContentPart]) -> serde_json::Value {
    serde_json::Value::Array(
        parts
            .iter()
            .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null))
            .collect(),
    )
}

fn tool_calls_to_value(tcs: &[cleanclaw_provider::message::ToolCall]) -> serde_json::Value {
    serde_json::Value::Array(
        tcs.iter()
            .map(|t| serde_json::to_value(t).unwrap_or(serde_json::Value::Null))
            .collect(),
    )
}

fn msg_to_record(
    agent_id: &str,
    user_id: &str,
    session_key: &str,
    m: &ProviderMessage,
) -> SessionMessageRecord {
    let now = Utc::now();
    SessionMessageRecord {
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        session_key: session_key.to_string(),
        seq: 0, // auto-assigned by store on insert
        role: role_to_str(&m.role).to_string(),
        content: m.content.clone(),
        content_parts: parts_to_value(&m.content_parts),
        tool_calls: tool_calls_to_value(&m.tool_calls),
        tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
        name: m.name.clone().unwrap_or_default(),
        metadata: serde_json::Value::Null,
        thinking: m.thinking.clone().unwrap_or_default(),
        raw_assistant: m.raw.clone().unwrap_or(serde_json::Value::Null),
        origin: String::new(),
        created_at: now,
        chatter_user_id: String::new(),
    }
}

fn record_to_msg(r: &SessionMessageRecord) -> ProviderMessage {
    use cleanclaw_provider::message::{ContentPart, ToolCall};
    let role = str_to_role(&r.role);
    let content_parts: Vec<ContentPart> =
        serde_json::from_value(r.content_parts.clone()).unwrap_or_default();
    let tool_calls: Vec<ToolCall> =
        serde_json::from_value(r.tool_calls.clone()).unwrap_or_default();
    ProviderMessage {
        role,
        content: r.content.clone(),
        content_parts,
        tool_calls,
        tool_call_id: if r.tool_call_id.is_empty() {
            None
        } else {
            Some(r.tool_call_id.clone())
        },
        name: if r.name.is_empty() {
            None
        } else {
            Some(r.name.clone())
        },
        cache_control: None,
        raw: if r.raw_assistant.is_null() {
            None
        } else {
            Some(r.raw_assistant.clone())
        },
        thinking: if r.thinking.is_empty() {
            None
        } else {
            Some(r.thinking.clone())
        },
        timestamp: Some(r.created_at.timestamp_millis()),
    }
}

#[async_trait::async_trait]
impl SessionBackend for StoreAdapter {
    async fn get_session(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<ProviderMessage>, SessionError> {
        let user_id = ""; // backend uses get_session with default user scope
        match self.store.get_session(user_id, agent_id, session_key).await {
            Ok(rec) => Ok(messages_from_value(&rec.messages)),
            Err(cleanclaw_core::CleanClawError::NotFound(_)) => Ok(Vec::new()),
            Err(e) => Err(SessionError::Store(e)),
        }
    }

    async fn save_session(
        &self,
        agent_id: &str,
        session_key: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        project_id: &str,
        chatter_user_id: &str,
        messages: &[ProviderMessage],
    ) -> Result<(), SessionError> {
        let user_id = "";
        let now = Utc::now();
        let rec = SessionRecord {
            user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
            session_key: session_key.to_string(),
            channel: channel.to_string(),
            account_id: account_id.to_string(),
            chat_id: chat_id.to_string(),
            project_id: project_id.to_string(),
            title: String::new(),
            messages: messages_to_value(messages),
            message_count: messages.len() as i32,
            updated_at: now,
            chatter_user_id: chatter_user_id.to_string(),
        };
        self.store
            .save_session(user_id, agent_id, session_key, &rec)
            .await?;
        Ok(())
    }

    async fn append_message(
        &self,
        agent_id: &str,
        session_key: &str,
        msg: &ProviderMessage,
    ) -> Result<(), SessionError> {
        let user_id = "";
        let rec = msg_to_record(agent_id, user_id, session_key, msg);
        let _ = self.store.append_session_message(&rec).await?;
        Ok(())
    }

    async fn list_messages(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<ProviderMessage>, SessionError> {
        let user_id = "";
        let recs = self
            .store
            .list_session_messages(user_id, agent_id, session_key)
            .await?;
        Ok(recs.iter().map(record_to_msg).collect())
    }

    async fn delete_session(&self, agent_id: &str, session_key: &str) -> Result<(), SessionError> {
        let user_id = "";
        self.store
            .delete_session(user_id, agent_id, session_key)
            .await?;
        Ok(())
    }

    async fn rename_session(
        &self,
        agent_id: &str,
        session_key: &str,
        title: &str,
    ) -> Result<(), SessionError> {
        let user_id = "";
        self.store
            .rename_session(user_id, agent_id, session_key, title)
            .await?;
        Ok(())
    }

    async fn move_session(
        &self,
        _agent_id: &str,
        _session_key: &str,
        _project_id: &str,
    ) -> Result<(), SessionError> {
        Ok(())
    }

    async fn resolve_active_session_key(
        &self,
        _agent_id: &str,
        _channel: &str,
        _account_id: &str,
        _chat_id: &str,
    ) -> Result<Option<String>, SessionError> {
        Ok(None)
    }

    async fn lookup_triple(
        &self,
        _agent_id: &str,
        _session_key: &str,
    ) -> Result<(String, String, String), SessionError> {
        Ok((String::new(), String::new(), String::new()))
    }

    async fn lookup_project(
        &self,
        _agent_id: &str,
        _session_key: &str,
    ) -> Result<String, SessionError> {
        Ok(String::new())
    }

    async fn list_web_sessions(&self, _agent_id: &str) -> Result<Vec<WebSession>, SessionError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_provider::message::{ContentPart, Role, ToolCall};

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-session-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: Role::User,
            content: text.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        }
    }

    fn assist_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: Role::Assistant,
            content: text.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        }
    }

    #[tokio::test]
    async fn manager_creates_and_appends() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "chat-1", "").await;
        assert_eq!(s.key(), "chat-1"); // web channel uses chat_id as key
        s.append(user_msg("hi")).await;
        s.append(assist_msg("hello!")).await;
        let msgs = s.get_messages().await;
        assert_eq!(msgs.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn non_web_channel_mints_opaque_key() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("telegram", "bot1", "user-42", "").await;
        let k = s.key();
        assert!(
            k.starts_with("s-"),
            "telegram key should be opaque, got {k}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn replace_messages_clears_consolidation_pointer() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "c", "").await;
        s.append(user_msg("a")).await;
        s.append(user_msg("b")).await;
        s.mark_consolidated(2).await;
        assert_eq!(s.unconsolidated_count().await, 0);
        s.replace_messages(vec![user_msg("c")]).await;
        assert_eq!(s.unconsolidated_count().await, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn snapshot_and_undo_round_trip() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "c", "").await;
        s.append(user_msg("v1")).await;
        s.snapshot().await;
        s.append(user_msg("v2")).await;
        assert!(s.has_snapshot().await);
        assert!(s.undo().await);
        let msgs = s.get_messages().await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "v1");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn undo_without_snapshot_returns_false() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "c", "").await;
        assert!(!s.undo().await);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn steering_buffering_during_turn() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "c", "").await;
        s.begin_turn().await;
        let accepted = s.push_steer_if_active(user_msg("steer-1")).await;
        assert!(accepted);
        s.begin_turn().await;
        let accepted = s.push_steer_if_active(user_msg("steer-2")).await;
        assert!(accepted);
        let drained = s.drain_steer().await;
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].content, "steer-1");

        // End first turn: still one in flight → no leftover.
        let leftover = s.end_turn().await;
        assert!(leftover.is_empty());
        // End second turn: depth 0, buffer empty → no leftover.
        let leftover = s.end_turn().await;
        assert!(leftover.is_empty());

        // No turn active now → steer is rejected.
        let accepted = s.push_steer_if_active(user_msg("steer-3")).await;
        assert!(!accepted);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn end_turn_returns_leftover_only_after_last() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "c", "").await;
        s.begin_turn().await;
        s.push_steer_if_active(user_msg("late-1")).await;
        // End while another turn is in flight: nothing returned.
        s.begin_turn().await;
        let none = s.end_turn().await;
        assert!(none.is_empty());
        // End last turn: leftover returned.
        let leftover = s.end_turn().await;
        assert_eq!(leftover.len(), 1);
        assert_eq!(leftover[0].content, "late-1");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn open_new_session_creates_independent_thread() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let key = m.open_new_session("telegram", "bot1", "u-1").await;
        assert!(key.starts_with("s-"));
        // Same triple gets the new key on next get.
        let s = m.get("telegram", "bot1", "u-1", "").await;
        assert_eq!(s.key(), key);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolve_session_key_round_trip() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let _ = m.get("web", "", "abc", "").await;
        // Existing key is returned unchanged.
        let r = m.resolve_session_key("abc").await;
        assert_eq!(r, "abc");
        // Empty input → empty.
        let r = m.resolve_session_key("").await;
        assert!(r.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn delete_session_by_id_clears_cache() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "to-delete", "").await;
        s.append(user_msg("data")).await;
        m.delete_session_by_id("to-delete").await.unwrap();
        // Cache is cleared; the file is gone.
        let path = dir.join("to-delete.jsonl");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn file_persists_messages_across_reload() {
        let dir = tmpdir();
        {
            let m = Manager::new(dir.clone());
            let s = m.get("web", "", "persist", "").await;
            s.append(user_msg("alpha")).await;
            s.append(assist_msg("beta")).await;
        }
        // New manager reads the same file.
        let m2 = Manager::new(dir.clone());
        let s2 = m2.get("web", "", "persist", "").await;
        let msgs = s2.get_messages().await;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "alpha");
        assert_eq!(msgs[1].content, "beta");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn list_web_sessions_scans_dir() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s1 = m.get("web", "", "w-1", "").await;
        s1.append(user_msg("first user msg")).await;
        let s2 = m.get("web", "", "w-2", "").await;
        s2.append(user_msg("second user msg")).await;
        let list = m.list_web_sessions().await;
        assert!(!list.is_empty());
        for w in &list {
            assert!(!w.title.is_empty());
            assert!(!w.preview.is_empty());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_role_message_serializes() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "tool", "").await;
        let tool_msg = ProviderMessage {
            role: Role::Tool,
            content: r#"{"ok":true}"#.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: Some("call_1".into()),
            name: Some("read_file".into()),
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        };
        let _ = tool_msg.timestamp;
        let mut assistant = assist_msg("I'll read the file");
        assistant.tool_calls = vec![ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "/tmp/x"}),
        }];
        s.append(assistant).await;
        s.append(tool_msg).await;
        let msgs = s.get_messages().await;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_1"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn multimodal_user_message_persists_content_parts() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "mm", "").await;
        let mmsg = ProviderMessage {
            role: Role::User,
            content: "what is this?".into(),
            content_parts: vec![ContentPart::ImageUrl {
                url: "https://x/y.png".into(),
            }],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        };
        let _ = mmsg.timestamp;
        s.append(mmsg).await;
        let loaded = s.get_messages().await;
        assert_eq!(loaded[0].content_parts.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn generate_session_key_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let k = generate_session_key();
            assert!(seen.insert(k.clone()));
            assert!(k.starts_with("s-"));
            let parts: Vec<&str> = k.splitn(3, '-').collect();
            assert_eq!(parts.len(), 3);
            assert_eq!(parts[0], "s");
        }
    }

    #[tokio::test]
    async fn safe_id_sanitizes_path_traversal() {
        assert_eq!(FileBackend::safe_id("a/b"), "a_b");
        assert_eq!(FileBackend::safe_id("a..b"), "a_b");
        assert_eq!(FileBackend::safe_id("a/../b"), "a___b");
    }

    #[tokio::test]
    async fn clear_removes_working_set() {
        let dir = tmpdir();
        let m = Manager::new(dir.clone());
        let s = m.get("web", "", "c", "").await;
        s.append(user_msg("x")).await;
        s.clear().await;
        assert!(s.get_messages().await.is_empty());
        let path = dir.join("c.jsonl");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
