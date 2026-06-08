//! Store trait — the unified persistence interface.
//!
//! All per-user tables require a real `users.id`; callers that haven't
//! resolved a user must 401, not invent a placeholder.

use crate::models::*;
use async_trait::async_trait;
use cleanclaw_core::Result;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    Sqlite,
    Postgres,
}

impl StorageType {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "postgres" | "pg" | "postgresql" => StorageType::Postgres,
            _ => StorageType::Sqlite,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageType::Sqlite => "sqlite",
            StorageType::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub r#type: StorageType,
    pub dsn: String,
    pub auto_migrate: bool,
}

#[async_trait]
pub trait Store: Send + Sync {
    // ---- Lifecycle ----
    async fn migrate(&self) -> Result<()>;
    async fn close(&self);

    // ---- Users ----
    async fn create_user(&self, u: &UserRecord) -> Result<()>;
    async fn get_user(&self, id: &str) -> Result<UserRecord>;
    async fn get_user_by_login(&self, username_or_email: &str) -> Result<UserRecord>;
    async fn get_user_by_external(&self, apikey_id: &str, external_id: &str) -> Result<UserRecord>;
    async fn list_users(&self) -> Result<Vec<UserRecord>>;
    async fn update_user(&self, u: &UserRecord) -> Result<()>;
    async fn delete_user(&self, id: &str) -> Result<()>;
    async fn count_users(&self) -> Result<i64>;

    // ---- Web sessions ----
    async fn create_web_session(&self, sess: &WebSessionRecord) -> Result<()>;
    async fn get_web_session(&self, sid: &str) -> Result<WebSessionRecord>;
    async fn delete_web_session(&self, sid: &str) -> Result<()>;
    async fn delete_expired_web_sessions(&self, before: Duration) -> Result<()>;

    // ---- API keys ----
    async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKeyRecord>>;
    async fn get_api_key(&self, id: &str) -> Result<ApiKeyRecord>;
    async fn create_api_key(&self, k: &ApiKeyRecord) -> Result<()>;
    async fn delete_api_key(&self, id: &str) -> Result<()>;
    async fn rotate_api_key(&self, id: &str, key_hash: &str, key_prefix: &str) -> Result<()>;
    async fn lookup_api_key_by_hash(&self, key_hash: &str) -> Result<ApiKeyRecord>;
    async fn set_api_key_agents(&self, apikey_id: &str, agent_ids: &[String]) -> Result<()>;
    async fn list_api_key_agents(&self, apikey_id: &str) -> Result<Vec<String>>;

    // ---- Agents ----
    async fn list_agents(&self, owner_user_id: &str) -> Result<Vec<AgentRecord>>;
    async fn get_agent(&self, agent_id: &str) -> Result<AgentRecord>;
    async fn save_agent(&self, agent: &AgentRecord) -> Result<()>;
    async fn delete_agent(&self, agent_id: &str) -> Result<()>;
    async fn list_all_agents(&self) -> Result<Vec<AgentRecord>>;

    // ---- Agent files (SOUL.md / IDENTITY.md / …) ----
    async fn get_workspace_file(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
    ) -> Result<(String, Vec<u8>)>;
    async fn get_workspace_file_exact(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
    ) -> Result<AgentFileRecord>;
    async fn save_workspace_file(
        &self,
        agent_id: &str,
        user_id: &str,
        filename: &str,
        data: &[u8],
    ) -> Result<()>;
    async fn list_workspace_files(&self, agent_id: &str) -> Result<Vec<String>>;

    // ---- Sessions ----
    async fn get_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<SessionRecord>;
    async fn save_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        session: &SessionRecord,
    ) -> Result<()>;
    async fn list_sessions(&self, user_id: &str, agent_id: &str) -> Result<Vec<SessionMeta>>;
    async fn list_session_owner_pairs(&self) -> Result<Vec<SessionOwnerPair>>;
    async fn delete_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<()>;
    async fn rename_session(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        title: &str,
    ) -> Result<()>;

    // ---- Session messages (append-only) ----
    async fn append_session_message(&self, m: &SessionMessageRecord) -> Result<i64>;
    async fn list_session_messages(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<SessionMessageRecord>>;
    async fn append_session_event(&self, e: &SessionEventRecord) -> Result<i64>;
    async fn list_session_events(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
    ) -> Result<Vec<SessionEventRecord>>;

    // ---- Configs (scope-tagged) ----
    async fn get_config(
        &self,
        kind: &str,
        user_id: &str,
        agent_id: &str,
        name: &str,
    ) -> Result<ConfigRecord>;
    async fn list_configs(
        &self,
        kind: &str,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Vec<ConfigRecord>>;
    async fn save_config(&self, rec: &ConfigRecord) -> Result<()>;
    async fn delete_config(
        &self,
        kind: &str,
        user_id: &str,
        agent_id: &str,
        name: &str,
    ) -> Result<()>;
    async fn list_configs_all_kinds(&self) -> Result<Vec<ConfigRecord>>;

    /// Look up a single channel config row by `(name, credential_key)`
    /// — the indexed path the orchestrator's `resolve_channel_owner`
    /// uses to map an inbound IM message to its owning user.
    /// Replaces the per-user list-scan fallback that previously
    /// ran on every inbound. Returns `Ok(None)` when no row
    /// matches.
    async fn lookup_channel_by_credential(
        &self,
        name: &str,
        credential_key: &str,
    ) -> Result<Option<ConfigRecord>>;

    // ---- Projects ----
    async fn list_projects(&self, user_id: &str, agent_id: &str) -> Result<Vec<ProjectRecord>>;
    async fn get_project(
        &self,
        user_id: &str,
        agent_id: &str,
        project_id: &str,
    ) -> Result<ProjectRecord>;
    async fn save_project(&self, p: &ProjectRecord) -> Result<()>;
    async fn delete_project(
        &self,
        user_id: &str,
        agent_id: &str,
        project_id: &str,
    ) -> Result<()>;

    // ---- Goals ----
    async fn save_goal(&self, g: &GoalRecord) -> Result<()>;
    async fn get_goal(&self, agent_id: &str, session_key: &str) -> Result<GoalRecord>;
    async fn list_goals(&self, agent_id: &str) -> Result<Vec<GoalRecord>>;
    /// List every goal across all agents. Used by the
    /// `GoalManager::tick` poller which doesn't have an agent-id
    /// scope. Returns the full set; the caller filters by status.
    async fn list_all_goals(&self) -> Result<Vec<GoalRecord>>;
    async fn delete_goal(&self, agent_id: &str, session_key: &str) -> Result<()>;

    // ---- Cron jobs ----
    async fn save_cron_job(&self, j: &CronJobRecord) -> Result<()>;
    async fn get_cron_job(&self, id: &str) -> Result<CronJobRecord>;
    async fn list_cron_jobs_by_agent(&self, agent_id: &str) -> Result<Vec<CronJobRecord>>;
    async fn list_due_cron_jobs(&self, now_unix: i64, limit: i64) -> Result<Vec<CronJobRecord>>;
    async fn list_all_cron_jobs(&self) -> Result<Vec<CronJobRecord>>;
    async fn delete_cron_job(&self, id: &str) -> Result<()>;

    // ---- Channel leases ----
    async fn try_acquire_channel_lease(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
        ttl: Duration,
    ) -> Result<bool>;
    async fn renew_channel_lease(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
        ttl: Duration,
    ) -> Result<()>;
    async fn release_channel_lease(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
    ) -> Result<()>;

    // ---- Token usage ----
    async fn upsert_token_usage(&self, r: &TokenUsageRecord) -> Result<()>;
    async fn list_token_usage(
        &self,
        since_day: chrono::NaiveDate,
    ) -> Result<Vec<TokenUsageRecord>>;
}
