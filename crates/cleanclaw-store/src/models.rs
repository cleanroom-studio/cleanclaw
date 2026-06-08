//! Database models — records that map 1:1 to tables.
//!
//! Mirrors the Go types in .

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---- Users -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub display_name: String,
    pub role: String, // "super_admin" | "admin" | "user"
    pub status: String,
    pub apikey_id: String,
    pub external_id: String,
    pub avatar_url: String,
    pub agent_quota: i32, // -1 unlimited, 0 admin-only, N max
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionRecord {
    pub sid: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub r#type: String, // "admin" | "user" | "agent"
    pub created_at: DateTime<Utc>,
    /// Hash of the most recently rotated-out key. Used to refuse
    /// `rotate` calls that would re-issue the same token within the
    /// rotation grace window (a defence against replay/reuse attacks
    /// if an old token is still in flight).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash_set_at: Option<DateTime<Utc>>,
}

// ---- Agents ------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub config: serde_json::Value,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFileRecord {
    pub agent_id: String,
    pub user_id: String, // "" = shared template
    pub filename: String,
    pub content: String,
    pub updated_at: DateTime<Utc>,
}

// ---- Sessions ----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub user_id: String,
    pub agent_id: String,
    pub session_key: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub project_id: String,
    pub title: String,
    pub messages: serde_json::Value,
    pub message_count: i32,
    pub updated_at: DateTime<Utc>,
    pub chatter_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub key: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub project_id: String,
    pub title: String,
    pub message_count: i32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOwnerPair {
    pub user_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageRecord {
    pub user_id: String,
    pub agent_id: String,
    pub session_key: String,
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub content_parts: serde_json::Value,
    pub tool_calls: serde_json::Value,
    pub tool_call_id: String,
    pub name: String,
    pub metadata: serde_json::Value,
    pub thinking: String,
    pub raw_assistant: serde_json::Value,
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub chatter_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEventRecord {
    pub user_id: String,
    pub agent_id: String,
    pub session_key: String,
    pub seq: i64,
    pub r#type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub chatter_user_id: String,
}

// ---- Configs / Cron / Projects / Goals / Channel leases ---------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRecord {
    pub id: String,
    pub kind: String, // "provider" | "channel" | "setting"
    pub scope: String,
    pub user_id: String,
    pub agent_id: String,
    pub name: String,
    pub enabled: bool,
    pub credential_key: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub user_id: String,
    pub agent_id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalRecord {
    pub id: String,
    pub agent_id: String,
    pub session_key: String,
    pub owner_user_id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub project_id: String,
    pub objective: String,
    pub status: String,
    pub token_budget: Option<i64>,
    pub tokens_used: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobRecord {
    pub id: String,
    pub user_id: String,
    pub agent_id: String,
    pub name: String,
    pub r#type: String, // "cron" | "interval" | "once"
    pub schedule: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    pub account_id: String,
    pub timezone: String,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub failure_count: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelLeaseRecord {
    pub channel: String,
    pub account_id: String,
    pub holder_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageRecord {
    pub day: chrono::NaiveDate,
    pub user_id: String,
    pub agent_id: String,
    pub session_key: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_create_tokens: i64,
    pub request_count: i64,
}
