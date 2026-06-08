//! SQLite-backed store. SQLite serializes all writes through a single
//! connection — sqlx's `SqlitePool` with max_connections=1 mirrors the
//! CleanClaw pattern (one writer, predictable latency on busy installs).

use super::models::*;
use super::store::Store;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use cleanclaw_core::{CleanClawError, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use std::time::Duration;

pub struct SqliteStore {
    pool: SqlitePool,
    path: String,
}

impl SqliteStore {
    /// Open a SQLite store at `path`. If `path` is `":memory:"`, an
    /// in-memory DB is used (handy for tests).
    pub async fn open(path: &str) -> Result<Self> {
        let opts = SqliteConnectOptions::from_str(path)
            .map_err(|e| CleanClawError::Internal(format!("sqlite open: {e}")))?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await
            .map_err(|e| CleanClawError::Internal(format!("sqlite connect: {e}")))?;

        Ok(Self {
            pool,
            path: path.to_string(),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn migrate(&self) -> Result<()> {
        sqlx::raw_sql(super::migrations::SCHEMA_SQLITE)
            .execute(&self.pool)
            .await
            .map_err(|e| CleanClawError::Internal(format!("migrate sqlite: {e}")))?;
        Ok(())
    }

    async fn close(&self) {
        self.pool.close().await;
    }

    // ---- Users ----

    async fn create_user(&self, u: &UserRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO users
               (id, username, email, password_hash, display_name, role, status,
                apikey_id, external_id, avatar_url, agent_quota, created_at, updated_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(&u.id)
        .bind(&u.username)
        .bind(&u.email)
        .bind(&u.password_hash)
        .bind(&u.display_name)
        .bind(&u.role)
        .bind(&u.status)
        .bind(&u.apikey_id)
        .bind(&u.external_id)
        .bind(&u.avatar_url)
        .bind(u.agent_quota)
        .bind(u.created_at)
        .bind(u.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_user(&self, id: &str) -> Result<UserRecord> {
        let row = sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(CleanClawError::NotFound(format!("user {id}")))?;
        Ok(row_to_user(&row))
    }

    async fn get_user_by_login(&self, login: &str) -> Result<UserRecord> {
        let row = sqlx::query("SELECT * FROM users WHERE username = ? OR email = ? LIMIT 1")
            .bind(login)
            .bind(login)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(CleanClawError::NotFound(format!("user {login}")))?;
        Ok(row_to_user(&row))
    }

    async fn get_user_by_external(&self, apikey_id: &str, external_id: &str) -> Result<UserRecord> {
        let row =
            sqlx::query("SELECT * FROM users WHERE apikey_id = ? AND external_id = ? LIMIT 1")
                .bind(apikey_id)
                .bind(external_id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(CleanClawError::NotFound("app_user".into()))?;
        Ok(row_to_user(&row))
    }

    async fn list_users(&self) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_user).collect())
    }

    async fn update_user(&self, u: &UserRecord) -> Result<()> {
        sqlx::query(
            r#"UPDATE users SET
                username = ?, email = ?, password_hash = ?, display_name = ?,
                role = ?, status = ?, apikey_id = ?, external_id = ?,
                avatar_url = ?, agent_quota = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(&u.username)
        .bind(&u.email)
        .bind(&u.password_hash)
        .bind(&u.display_name)
        .bind(&u.role)
        .bind(&u.status)
        .bind(&u.apikey_id)
        .bind(&u.external_id)
        .bind(&u.avatar_url)
        .bind(u.agent_quota)
        .bind(Utc::now())
        .bind(&u.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_user(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn count_users(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.try_get::<i64, _>("c").unwrap_or(0))
    }

    // ---- Web sessions ----

    async fn create_web_session(&self, s: &WebSessionRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO web_sessions (sid, user_id, created_at, expires_at) VALUES (?,?,?,?)",
        )
        .bind(&s.sid)
        .bind(&s.user_id)
        .bind(s.created_at)
        .bind(s.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_web_session(&self, sid: &str) -> Result<WebSessionRecord> {
        let row = sqlx::query("SELECT * FROM web_sessions WHERE sid = ?")
            .bind(sid)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| CleanClawError::NotFound(format!("session {sid}")))?;
        Ok(WebSessionRecord {
            sid: row.get("sid"),
            user_id: row.get("user_id"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
        })
    }

    async fn delete_web_session(&self, sid: &str) -> Result<()> {
        sqlx::query("DELETE FROM web_sessions WHERE sid = ?")
            .bind(sid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_expired_web_sessions(&self, before: Duration) -> Result<()> {
        let cutoff = Utc::now() - chrono::Duration::from_std(before).unwrap_or_default();
        sqlx::query("DELETE FROM web_sessions WHERE expires_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- API keys ----

    async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKeyRecord>> {
        let rows = sqlx::query("SELECT * FROM apikeys WHERE user_id = ? ORDER BY created_at DESC")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_api_key).collect())
    }

    async fn get_api_key(&self, id: &str) -> Result<ApiKeyRecord> {
        let row = sqlx::query("SELECT * FROM apikeys WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| CleanClawError::NotFound(format!("apikey {id}")))?;
        Ok(row_to_api_key(&row))
    }

    async fn create_api_key(&self, k: &ApiKeyRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO apikeys (id, user_id, name, key_hash, key_prefix, type, created_at, prev_hash, prev_hash_set_at) VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(&k.id)
        .bind(&k.user_id)
        .bind(&k.name)
        .bind(&k.key_hash)
        .bind(&k.key_prefix)
        .bind(&k.r#type)
        .bind(k.created_at)
        .bind(&k.prev_hash)
        .bind(k.prev_hash_set_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_api_key(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM apikeys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM apikey_agents WHERE apikey_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn rotate_api_key(&self, id: &str, key_hash: &str, key_prefix: &str) -> Result<()> {
        // Promote the current key_hash to prev_hash (with timestamp)
        // before overwriting it, so a follow-up lookup by the old hash
        // can be detected inside the rotation grace window.
        let current: Option<String> =
            sqlx::query_scalar("SELECT key_hash FROM apikeys WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        if let Some(prev) = current {
            sqlx::query(
                "UPDATE apikeys SET key_hash = ?, key_prefix = ?, prev_hash = ?, prev_hash_set_at = ? WHERE id = ?",
            )
            .bind(key_hash)
            .bind(key_prefix)
            .bind(prev)
            .bind(chrono::Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query("UPDATE apikeys SET key_hash = ?, key_prefix = ? WHERE id = ?")
                .bind(key_hash)
                .bind(key_prefix)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    async fn lookup_api_key_by_hash(&self, key_hash: &str) -> Result<ApiKeyRecord> {
        let row = sqlx::query("SELECT * FROM apikeys WHERE key_hash = ? LIMIT 1")
            .bind(key_hash)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(CleanClawError::Unauthorized)?;
        Ok(row_to_api_key(&row))
    }

    async fn set_api_key_agents(&self, apikey_id: &str, agent_ids: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM apikey_agents WHERE apikey_id = ?")
            .bind(apikey_id)
            .execute(&mut *tx)
            .await?;
        for aid in agent_ids {
            sqlx::query("INSERT INTO apikey_agents (apikey_id, agent_id) VALUES (?, ?)")
                .bind(apikey_id)
                .bind(aid)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn list_api_key_agents(&self, apikey_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT agent_id FROM apikey_agents WHERE apikey_id = ?")
            .bind(apikey_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| r.get::<String, _>("agent_id"))
            .collect())
    }

    // ---- Agents ----

    async fn list_agents(&self, owner_user_id: &str) -> Result<Vec<AgentRecord>> {
        let rows = sqlx::query("SELECT * FROM agents WHERE user_id = ? ORDER BY created_at ASC")
            .bind(owner_user_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_agent).collect())
    }

    async fn get_agent(&self, agent_id: &str) -> Result<AgentRecord> {
        let row = sqlx::query("SELECT * FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| CleanClawError::NotFound(format!("agent {agent_id}")))?;
        Ok(row_to_agent(&row))
    }

    async fn save_agent(&self, a: &AgentRecord) -> Result<()> {
        let now = Utc::now();
        let config_str = serde_json::to_string(&a.config)?;
        sqlx::query(
            r#"INSERT INTO agents (id, user_id, name, config, is_public, created_at, updated_at)
               VALUES (?,?,?,?,?,?,?)
               ON CONFLICT(id) DO UPDATE SET
                   user_id = excluded.user_id,
                   name = excluded.name,
                   config = excluded.config,
                   is_public = excluded.is_public,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&a.id)
        .bind(&a.user_id)
        .bind(&a.name)
        .bind(&config_str)
        .bind(a.is_public)
        .bind(a.created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_agent(&self, agent_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_all_agents(&self) -> Result<Vec<AgentRecord>> {
        let rows = sqlx::query("SELECT * FROM agents ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_agent).collect())
    }

    // ---- Agent files ----

    async fn get_workspace_file(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
    ) -> Result<(String, Vec<u8>)> {
        // Owner-fallback overlay: try (chatter, file), then ('', file).
        if let Some(rec) = self
            .get_workspace_file_exact_opt(agent_id, user_id, filename)
            .await?
        {
            return Ok((rec.user_id, rec.content.into_bytes()));
        }
        let rec = self
            .get_workspace_file_exact_opt(agent_id, "", filename)
            .await?
            .ok_or_else(|| CleanClawError::NotFound(format!("file {filename}")))?;
        Ok((rec.user_id, rec.content.into_bytes()))
    }

    async fn get_workspace_file_exact(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
    ) -> Result<AgentFileRecord> {
        self.get_workspace_file_exact_opt(agent_id, user_id, filename)
            .await?
            .ok_or_else(|| CleanClawError::NotFound(format!("file {filename}")))
    }

    async fn save_workspace_file(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
        data: &[u8],
    ) -> Result<()> {
        let content = String::from_utf8_lossy(data).to_string();
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO agent_files (agent_id, user_id, filename, content, updated_at)
               VALUES (?,?,?,?,?)
               ON CONFLICT(agent_id, user_id, filename) DO UPDATE SET
                   content = excluded.content,
                   updated_at = excluded.updated_at"#,
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(filename)
        .bind(&content)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_workspace_files(&self, agent_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT filename FROM agent_files WHERE agent_id = ? ORDER BY filename",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| r.get::<String, _>("filename"))
            .collect())
    }

    // ---- Sessions ----

    async fn get_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<SessionRecord> {
        let row = sqlx::query(
            "SELECT * FROM sessions WHERE user_id = ? AND agent_id = ? AND session_key = ?",
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(CleanClawError::NotFound(format!("session {session_key}")))?;
        Ok(row_to_session(&row))
    }

    async fn save_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        s: &SessionRecord,
    ) -> Result<()> {
        let now = Utc::now();
        let messages_str = serde_json::to_string(&s.messages)?;
        sqlx::query(
            r#"INSERT INTO sessions
                 (user_id, agent_id, session_key, channel, account_id, chat_id, project_id,
                  title, messages, message_count, updated_at, chatter_user_id)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
               ON CONFLICT(user_id, agent_id, session_key) DO UPDATE SET
                   channel = excluded.channel,
                   account_id = excluded.account_id,
                   chat_id = excluded.chat_id,
                   project_id = excluded.project_id,
                   title = excluded.title,
                   messages = excluded.messages,
                   message_count = excluded.message_count,
                   updated_at = excluded.updated_at,
                   chatter_user_id = excluded.chatter_user_id"#,
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .bind(&s.channel)
        .bind(&s.account_id)
        .bind(&s.chat_id)
        .bind(&s.project_id)
        .bind(&s.title)
        .bind(&messages_str)
        .bind(s.message_count)
        .bind(now)
        .bind(&s.chatter_user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: &str, agent_id: &str) -> Result<Vec<SessionMeta>> {
        let rows = sqlx::query(
            r#"SELECT session_key, channel, account_id, chat_id, project_id, title,
                      message_count, updated_at
               FROM sessions WHERE user_id = ? AND agent_id = ?
               ORDER BY updated_at DESC"#,
        )
        .bind(user_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SessionMeta {
                key: r.get("session_key"),
                channel: r.get("channel"),
                account_id: r.get("account_id"),
                chat_id: r.get("chat_id"),
                project_id: r.get("project_id"),
                title: r.get("title"),
                message_count: r.get("message_count"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    async fn list_session_owner_pairs(&self) -> Result<Vec<SessionOwnerPair>> {
        let rows = sqlx::query("SELECT DISTINCT user_id, agent_id FROM sessions")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| SessionOwnerPair {
                user_id: r.get("user_id"),
                agent_id: r.get("agent_id"),
            })
            .collect())
    }

    async fn delete_session(&self, user_id: &str, agent_id: &str, session_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE user_id = ? AND agent_id = ? AND session_key = ?")
            .bind(user_id)
            .bind(agent_id)
            .bind(session_key)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM session_messages WHERE user_id = ? AND agent_id = ? AND session_key = ?",
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM session_events WHERE user_id = ? AND agent_id = ? AND session_key = ?",
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn rename_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        title: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET title = ?, updated_at = ? WHERE user_id = ? AND agent_id = ? AND session_key = ?",
        )
        .bind(title)
        .bind(Utc::now())
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- Session messages ----

    async fn append_session_message(&self, m: &SessionMessageRecord) -> Result<i64> {
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM session_messages WHERE user_id = ? AND agent_id = ? AND session_key = ?",
        )
        .bind(&m.user_id)
        .bind(&m.agent_id)
        .bind(&m.session_key)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            r#"INSERT INTO session_messages
                 (user_id, agent_id, session_key, seq, role, content, content_parts,
                  tool_calls, tool_call_id, name, metadata, thinking, raw_assistant,
                  origin, created_at, chatter_user_id)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(&m.user_id)
        .bind(&m.agent_id)
        .bind(&m.session_key)
        .bind(next_seq)
        .bind(&m.role)
        .bind(&m.content)
        .bind(serde_json::to_string(&m.content_parts)?)
        .bind(serde_json::to_string(&m.tool_calls)?)
        .bind(&m.tool_call_id)
        .bind(&m.name)
        .bind(serde_json::to_string(&m.metadata)?)
        .bind(&m.thinking)
        .bind(serde_json::to_string(&m.raw_assistant)?)
        .bind(&m.origin)
        .bind(Utc::now())
        .bind(&m.chatter_user_id)
        .execute(&self.pool)
        .await?;

        Ok(next_seq)
    }

    async fn list_session_messages(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<SessionMessageRecord>> {
        let rows = sqlx::query(
            r#"SELECT * FROM session_messages
               WHERE user_id = ? AND agent_id = ? AND session_key = ?
               ORDER BY seq ASC"#,
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_session_message).collect())
    }

    async fn append_session_event(&self, e: &SessionEventRecord) -> Result<i64> {
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM session_events WHERE user_id = ? AND agent_id = ? AND session_key = ?",
        )
        .bind(&e.user_id)
        .bind(&e.agent_id)
        .bind(&e.session_key)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            r#"INSERT INTO session_events
                 (user_id, agent_id, session_key, seq, type, data, created_at, chatter_user_id)
               VALUES (?,?,?,?,?,?,?,?)"#,
        )
        .bind(&e.user_id)
        .bind(&e.agent_id)
        .bind(&e.session_key)
        .bind(next_seq)
        .bind(&e.r#type)
        .bind(serde_json::to_string(&e.data)?)
        .bind(Utc::now())
        .bind(&e.chatter_user_id)
        .execute(&self.pool)
        .await?;
        Ok(next_seq)
    }

    async fn list_session_events(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<SessionEventRecord>> {
        let rows = sqlx::query(
            r#"SELECT * FROM session_events
               WHERE user_id = ? AND agent_id = ? AND session_key = ?
               ORDER BY seq ASC"#,
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(session_key)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_session_event).collect())
    }

    // ---- Configs ----

    async fn get_config(
        &self,
        kind: &str,
        user_id: &str,
        agent_id: &str,
        name: &str,
    ) -> Result<ConfigRecord> {
        let row = sqlx::query(
            "SELECT * FROM configs WHERE kind = ? AND user_id = ? AND agent_id = ? AND name = ?",
        )
        .bind(kind)
        .bind(user_id)
        .bind(agent_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(CleanClawError::NotFound(format!("config {name}")))?;
        Ok(row_to_config(&row))
    }

    async fn list_configs(
        &self,
        kind: &str,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Vec<ConfigRecord>> {
        let rows = sqlx::query(
            "SELECT * FROM configs WHERE kind = ? AND user_id = ? AND agent_id = ? ORDER BY name",
        )
        .bind(kind)
        .bind(user_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_config).collect())
    }

    async fn save_config(&self, rec: &ConfigRecord) -> Result<()> {
        let now = Utc::now();
        let data_str = serde_json::to_string(&rec.data)?;
        sqlx::query(
            r#"INSERT INTO configs
                 (id, kind, scope, user_id, agent_id, name, enabled, credential_key, data,
                  created_at, updated_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?)
               ON CONFLICT(kind, user_id, agent_id, name) DO UPDATE SET
                   enabled = excluded.enabled,
                   credential_key = excluded.credential_key,
                   data = excluded.data,
                   scope = excluded.scope,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&rec.id)
        .bind(&rec.kind)
        .bind(&rec.scope)
        .bind(&rec.user_id)
        .bind(&rec.agent_id)
        .bind(&rec.name)
        .bind(rec.enabled)
        .bind(&rec.credential_key)
        .bind(&data_str)
        .bind(rec.created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_config(
        &self,
        kind: &str,
        user_id: &str,
        agent_id: &str,
        name: &str,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM configs WHERE kind = ? AND user_id = ? AND agent_id = ? AND name = ?",
        )
        .bind(kind)
        .bind(user_id)
        .bind(agent_id)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_configs_all_kinds(&self) -> Result<Vec<ConfigRecord>> {
        let rows = sqlx::query("SELECT * FROM configs ORDER BY kind, name")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_config).collect())
    }

    async fn lookup_channel_by_credential(
        &self,
        name: &str,
        credential_key: &str,
    ) -> Result<Option<ConfigRecord>> {
        // The (name, credential_key) pair is the natural unique
        // key for channel rows. credential_key is empty for
        // system-scope rows, so we OR-match both shapes.
        let row = sqlx::query(
            "SELECT * FROM configs WHERE kind = 'channel' \
             AND name = ? AND (credential_key = ? OR credential_key = '') \
             ORDER BY user_id DESC LIMIT 1",
        )
        .bind(name)
        .bind(credential_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_to_config(&r)))
    }

    // ---- Projects ----

    async fn list_projects(&self, user_id: &str, agent_id: &str) -> Result<Vec<ProjectRecord>> {
        let rows = sqlx::query(
            "SELECT * FROM projects WHERE user_id = ? AND agent_id = ? ORDER BY updated_at DESC",
        )
        .bind(user_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_project).collect())
    }

    async fn get_project(
        &self,
        user_id: &str,
        agent_id: &str,
        project_id: &str,
    ) -> Result<ProjectRecord> {
        let row = sqlx::query(
            "SELECT * FROM projects WHERE user_id = ? AND agent_id = ? AND project_id = ?",
        )
        .bind(user_id)
        .bind(agent_id)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(CleanClawError::NotFound(format!("project {project_id}")))?;
        Ok(row_to_project(&row))
    }

    async fn save_project(&self, p: &ProjectRecord) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO projects
                 (user_id, agent_id, project_id, name, description, created_at, updated_at)
               VALUES (?,?,?,?,?,?,?)
               ON CONFLICT(user_id, agent_id, project_id) DO UPDATE SET
                   name = excluded.name,
                   description = excluded.description,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&p.user_id)
        .bind(&p.agent_id)
        .bind(&p.project_id)
        .bind(&p.name)
        .bind(&p.description)
        .bind(p.created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_project(&self, user_id: &str, agent_id: &str, project_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM projects WHERE user_id = ? AND agent_id = ? AND project_id = ?")
            .bind(user_id)
            .bind(agent_id)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Goals ----

    async fn save_goal(&self, g: &GoalRecord) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO agent_goals
                 (id, agent_id, session_key, owner_user_id, channel, account_id, chat_id,
                  project_id, objective, status, token_budget, tokens_used, created_at, updated_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)
               ON CONFLICT(id) DO UPDATE SET
                   objective = excluded.objective,
                   status = excluded.status,
                   token_budget = excluded.token_budget,
                   tokens_used = excluded.tokens_used,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&g.id)
        .bind(&g.agent_id)
        .bind(&g.session_key)
        .bind(&g.owner_user_id)
        .bind(&g.channel)
        .bind(&g.account_id)
        .bind(&g.chat_id)
        .bind(&g.project_id)
        .bind(&g.objective)
        .bind(&g.status)
        .bind(g.token_budget)
        .bind(g.tokens_used)
        .bind(g.created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_goal(&self, agent_id: &str, session_key: &str) -> Result<GoalRecord> {
        let row = sqlx::query("SELECT * FROM agent_goals WHERE agent_id = ? AND session_key = ?")
            .bind(agent_id)
            .bind(session_key)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(CleanClawError::NotFound("goal".into()))?;
        Ok(row_to_goal(&row))
    }

    async fn list_goals(&self, agent_id: &str) -> Result<Vec<GoalRecord>> {
        let rows =
            sqlx::query("SELECT * FROM agent_goals WHERE agent_id = ? ORDER BY created_at DESC")
                .bind(agent_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.iter().map(row_to_goal).collect())
    }

    async fn list_all_goals(&self) -> Result<Vec<GoalRecord>> {
        let rows = sqlx::query("SELECT * FROM agent_goals ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_goal).collect())
    }

    async fn delete_goal(&self, agent_id: &str, session_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM agent_goals WHERE agent_id = ? AND session_key = ?")
            .bind(agent_id)
            .bind(session_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Cron jobs ----

    async fn save_cron_job(&self, j: &CronJobRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO cron_jobs
                 (id, user_id, agent_id, name, type, schedule, message, channel, chat_id,
                  account_id, timezone, enabled, last_run, next_run, locked_by, locked_at,
                  failure_count, created_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
               ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   type = excluded.type,
                   schedule = excluded.schedule,
                   message = excluded.message,
                   channel = excluded.channel,
                   chat_id = excluded.chat_id,
                   account_id = excluded.account_id,
                   timezone = excluded.timezone,
                   enabled = excluded.enabled,
                   last_run = excluded.last_run,
                   next_run = excluded.next_run,
                   failure_count = excluded.failure_count"#,
        )
        .bind(&j.id)
        .bind(&j.user_id)
        .bind(&j.agent_id)
        .bind(&j.name)
        .bind(&j.r#type)
        .bind(&j.schedule)
        .bind(&j.message)
        .bind(&j.channel)
        .bind(&j.chat_id)
        .bind(&j.account_id)
        .bind(&j.timezone)
        .bind(j.enabled)
        .bind(j.last_run)
        .bind(j.next_run)
        .bind(&j.locked_by)
        .bind(j.locked_at)
        .bind(j.failure_count)
        .bind(j.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_cron_job(&self, id: &str) -> Result<CronJobRecord> {
        let row = sqlx::query("SELECT * FROM cron_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(CleanClawError::NotFound(format!("cron {id}")))?;
        Ok(row_to_cron(&row))
    }

    async fn list_cron_jobs_by_agent(&self, agent_id: &str) -> Result<Vec<CronJobRecord>> {
        let rows =
            sqlx::query("SELECT * FROM cron_jobs WHERE agent_id = ? ORDER BY created_at DESC")
                .bind(agent_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.iter().map(row_to_cron).collect())
    }

    async fn list_due_cron_jobs(&self, _now_unix: i64, _limit: i64) -> Result<Vec<CronJobRecord>> {
        let now: DateTime<Utc> = Utc::now();
        let rows = sqlx::query(
            "SELECT * FROM cron_jobs WHERE enabled = 1 AND (next_run IS NULL OR next_run <= ?)
             ORDER BY next_run ASC LIMIT ?",
        )
        .bind(now)
        .bind(_limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_cron).collect())
    }

    async fn list_all_cron_jobs(&self) -> Result<Vec<CronJobRecord>> {
        let rows = sqlx::query("SELECT * FROM cron_jobs ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_cron).collect())
    }

    async fn delete_cron_job(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM cron_jobs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Channel leases ----

    async fn try_acquire_channel_lease(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
        ttl: Duration,
    ) -> Result<bool> {
        let expires_at = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();
        let mut tx = self.pool.begin().await?;

        // Try UPDATE first (renew if same holder / steal if expired).
        let res = sqlx::query(
            r#"UPDATE channel_leases
               SET holder_id = ?, expires_at = ?
               WHERE channel = ? AND account_id = ? AND (expires_at < ? OR holder_id = ?)"#,
        )
        .bind(holder_id)
        .bind(expires_at)
        .bind(channel)
        .bind(account_id)
        .bind(Utc::now())
        .bind(holder_id)
        .execute(&mut *tx)
        .await?;

        if res.rows_affected() > 0 {
            tx.commit().await?;
            return Ok(true);
        }

        // No row or someone else holds a non-expired lease.
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_leases WHERE channel = ? AND account_id = ?",
        )
        .bind(channel)
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;

        if exists == 0 {
            sqlx::query(
                r#"INSERT INTO channel_leases (channel, account_id, holder_id, expires_at)
                   VALUES (?,?,?,?)"#,
            )
            .bind(channel)
            .bind(account_id)
            .bind(holder_id)
            .bind(expires_at)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(true);
        }
        tx.commit().await?;
        Ok(false)
    }

    async fn renew_channel_lease(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
        ttl: Duration,
    ) -> Result<()> {
        let expires_at = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();
        sqlx::query(
            "UPDATE channel_leases SET expires_at = ? WHERE channel = ? AND account_id = ? AND holder_id = ?",
        )
        .bind(expires_at)
        .bind(channel)
        .bind(account_id)
        .bind(holder_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn release_channel_lease(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM channel_leases WHERE channel = ? AND account_id = ? AND holder_id = ?",
        )
        .bind(channel)
        .bind(account_id)
        .bind(holder_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- Token usage ----

    async fn upsert_token_usage(&self, r: &TokenUsageRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO token_usage_daily
                 (day, user_id, agent_id, session_key, provider, model,
                  input_tokens, output_tokens, cache_read_tokens, cache_create_tokens, request_count)
               VALUES (?,?,?,?,?,?,?,?,?,?,?)
               ON CONFLICT(day, user_id, agent_id, session_key, provider, model) DO UPDATE SET
                   input_tokens = input_tokens + excluded.input_tokens,
                   output_tokens = output_tokens + excluded.output_tokens,
                   cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                   cache_create_tokens = cache_create_tokens + excluded.cache_create_tokens,
                   request_count = request_count + excluded.request_count"#,
        )
        .bind(r.day)
        .bind(&r.user_id)
        .bind(&r.agent_id)
        .bind(&r.session_key)
        .bind(&r.provider)
        .bind(&r.model)
        .bind(r.input_tokens)
        .bind(r.output_tokens)
        .bind(r.cache_read_tokens)
        .bind(r.cache_create_tokens)
        .bind(r.request_count)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_token_usage(&self, since_day: NaiveDate) -> Result<Vec<TokenUsageRecord>> {
        let rows = sqlx::query("SELECT * FROM token_usage_daily WHERE day >= ? ORDER BY day DESC")
            .bind(since_day)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_usage).collect())
    }
}

impl SqliteStore {
    async fn get_workspace_file_exact_opt(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
    ) -> Result<Option<AgentFileRecord>> {
        let row = sqlx::query(
            "SELECT * FROM agent_files WHERE agent_id = ? AND user_id = ? AND filename = ?",
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(filename)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| AgentFileRecord {
            agent_id: r.get("agent_id"),
            user_id: r.get("user_id"),
            filename: r.get("filename"),
            content: r.get("content"),
            updated_at: r.get("updated_at"),
        }))
    }
}

// ---- Row mappers --------------------------------------------------------

fn row_to_user(r: &sqlx::sqlite::SqliteRow) -> UserRecord {
    UserRecord {
        id: r.get("id"),
        username: r.get("username"),
        email: r.get("email"),
        password_hash: r.get("password_hash"),
        display_name: r.get("display_name"),
        role: r.get("role"),
        status: r.get("status"),
        apikey_id: r.get("apikey_id"),
        external_id: r.get("external_id"),
        avatar_url: r.get("avatar_url"),
        agent_quota: r.get("agent_quota"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_api_key(r: &sqlx::sqlite::SqliteRow) -> ApiKeyRecord {
    ApiKeyRecord {
        id: r.get("id"),
        user_id: r.get("user_id"),
        name: r.get("name"),
        key_hash: r.get("key_hash"),
        key_prefix: r.get("key_prefix"),
        r#type: r.get("type"),
        created_at: r.get("created_at"),
        prev_hash: r.try_get("prev_hash").ok(),
        prev_hash_set_at: r.try_get("prev_hash_set_at").ok(),
    }
}

fn row_to_agent(r: &sqlx::sqlite::SqliteRow) -> AgentRecord {
    let config_str: String = r.get("config");
    let config = serde_json::from_str(&config_str).unwrap_or(serde_json::json!({}));
    AgentRecord {
        id: r.get("id"),
        user_id: r.get("user_id"),
        name: r.get("name"),
        config,
        is_public: {
            let v: i64 = r.get("is_public");
            v != 0
        },
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_session(r: &sqlx::sqlite::SqliteRow) -> SessionRecord {
    let messages_str: String = r.get("messages");
    let messages = serde_json::from_str(&messages_str).unwrap_or(serde_json::json!([]));
    SessionRecord {
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        session_key: r.get("session_key"),
        channel: r.get("channel"),
        account_id: r.get("account_id"),
        chat_id: r.get("chat_id"),
        project_id: r.get("project_id"),
        title: r.get("title"),
        messages,
        message_count: r.get("message_count"),
        updated_at: r.get("updated_at"),
        chatter_user_id: r.get("chatter_user_id"),
    }
}

fn row_to_session_message(r: &sqlx::sqlite::SqliteRow) -> SessionMessageRecord {
    fn parse_json(s: String) -> serde_json::Value {
        serde_json::from_str(&s).unwrap_or(serde_json::Value::Null)
    }
    SessionMessageRecord {
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        session_key: r.get("session_key"),
        seq: r.get("seq"),
        role: r.get("role"),
        content: r.get("content"),
        content_parts: parse_json(r.get("content_parts")),
        tool_calls: parse_json(r.get("tool_calls")),
        tool_call_id: r.get("tool_call_id"),
        name: r.get("name"),
        metadata: parse_json(r.get("metadata")),
        thinking: r.get("thinking"),
        raw_assistant: parse_json(r.get("raw_assistant")),
        origin: r.get("origin"),
        created_at: r.get("created_at"),
        chatter_user_id: r.get("chatter_user_id"),
    }
}

fn row_to_session_event(r: &sqlx::sqlite::SqliteRow) -> SessionEventRecord {
    let data_str: String = r.get("data");
    SessionEventRecord {
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        session_key: r.get("session_key"),
        seq: r.get("seq"),
        r#type: r.get("type"),
        data: serde_json::from_str(&data_str).unwrap_or(serde_json::Value::Null),
        created_at: r.get("created_at"),
        chatter_user_id: r.get("chatter_user_id"),
    }
}

fn row_to_config(r: &sqlx::sqlite::SqliteRow) -> ConfigRecord {
    let data_str: String = r.get("data");
    let enabled: i64 = r.get("enabled");
    ConfigRecord {
        id: r.get("id"),
        kind: r.get("kind"),
        scope: r.get("scope"),
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        name: r.get("name"),
        enabled: enabled != 0,
        credential_key: r.get("credential_key"),
        data: serde_json::from_str(&data_str).unwrap_or(serde_json::json!({})),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_project(r: &sqlx::sqlite::SqliteRow) -> ProjectRecord {
    ProjectRecord {
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        project_id: r.get("project_id"),
        name: r.get("name"),
        description: r.get("description"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_goal(r: &sqlx::sqlite::SqliteRow) -> GoalRecord {
    let token_budget: Option<i64> = r.try_get("token_budget").ok();
    GoalRecord {
        id: r.get("id"),
        agent_id: r.get("agent_id"),
        session_key: r.get("session_key"),
        owner_user_id: r.get("owner_user_id"),
        channel: r.get("channel"),
        account_id: r.get("account_id"),
        chat_id: r.get("chat_id"),
        project_id: r.get("project_id"),
        objective: r.get("objective"),
        status: r.get("status"),
        token_budget,
        tokens_used: r.get("tokens_used"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_cron(r: &sqlx::sqlite::SqliteRow) -> CronJobRecord {
    CronJobRecord {
        id: r.get("id"),
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        name: r.get("name"),
        r#type: r.get("type"),
        schedule: r.get("schedule"),
        message: r.get("message"),
        channel: r.get("channel"),
        chat_id: r.get("chat_id"),
        account_id: r.get("account_id"),
        timezone: r.get("timezone"),
        enabled: {
            let v: i64 = r.get("enabled");
            v != 0
        },
        last_run: r.try_get("last_run").ok(),
        next_run: r.try_get("next_run").ok(),
        locked_by: r.try_get("locked_by").ok(),
        locked_at: r.try_get("locked_at").ok(),
        failure_count: r.get("failure_count"),
        created_at: r.get("created_at"),
    }
}

fn row_to_usage(r: &sqlx::sqlite::SqliteRow) -> TokenUsageRecord {
    TokenUsageRecord {
        day: r.get("day"),
        user_id: r.get("user_id"),
        agent_id: r.get("agent_id"),
        session_key: r.get("session_key"),
        provider: r.get("provider"),
        model: r.get("model"),
        input_tokens: r.get("input_tokens"),
        output_tokens: r.get("output_tokens"),
        cache_read_tokens: r.get("cache_read_tokens"),
        cache_create_tokens: r.get("cache_create_tokens"),
        request_count: r.get("request_count"),
    }
}
