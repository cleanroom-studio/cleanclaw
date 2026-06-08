//! Wire types for the web frontend. Mirrors the TypeScript interfaces
//! in . Every shape is a Rust struct
//! with `Serialize` + `Deserialize`, `#[serde(default)]` on optional
//! fields, and `#[serde(rename_all = "camelCase")]` where the JSON
//! uses camelCase but the Rust field is snake_case (the
//! `AgentUpdatePayload.splitReplies` family, etc.).
//!
//! Optional handling: `Option<T>` for everything that the Go side
//! may omit. `#[serde(default)]` is added on enum discriminants so a
//! missing `kind`/`type`/`scope` field doesn't fail the parse.
//!
//! These types are used by both the SSR HTML renderers (to type the
//! template arguments) and the W3 typed API client.

use serde::{Deserialize, Serialize};

// =====================================================================
// Common enums
// =====================================================================

/// `ScopeName` — system / user / agent scope for providers + channels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeName {
    #[default]
    System,
    User,
    Agent,
}

impl ScopeName {
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeName::System => "system",
            ScopeName::User => "user",
            ScopeName::Agent => "agent",
        }
    }
}

/// `ApikeyType` — admin / user / agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApikeyType {
    #[default]
    Admin,
    User,
    Agent,
}

impl ApikeyType {
    pub fn as_str(self) -> &'static str {
        match self {
            ApikeyType::Admin => "admin",
            ApikeyType::User => "user",
            ApikeyType::Agent => "agent",
        }
    }
}

/// `TokenUsageRange` — 24h / 7d / 30d.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenUsageRange {
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
    #[serde(rename = "30d")]
    D30,
}

impl Default for TokenUsageRange {
    fn default() -> Self {
        TokenUsageRange::D7
    }
}

impl TokenUsageRange {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenUsageRange::H24 => "24h",
            TokenUsageRange::D7 => "7d",
            TokenUsageRange::D30 => "30d",
        }
    }
}

/// Deploy mode — `self-hosted` (default) or `hosted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployMode {
    #[serde(rename = "self-hosted")]
    SelfHosted,
    #[serde(rename = "hosted")]
    Hosted,
}

impl Default for DeployMode {
    fn default() -> Self {
        DeployMode::SelfHosted
    }
}

/// Chat message role.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    #[default]
    User,
    Assistant,
    Tool,
}

/// Chat stream event `type` discriminant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatStreamType {
    #[default]
    Content,
    ContentDelta,
    ToolCall,
    ToolResult,
    Steer,
    Error,
    Done,
    SubagentProgress,
}

impl ChatStreamType {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatStreamType::Content => "content",
            ChatStreamType::ContentDelta => "content_delta",
            ChatStreamType::ToolCall => "tool_call",
            ChatStreamType::ToolResult => "tool_result",
            ChatStreamType::Steer => "steer",
            ChatStreamType::Error => "error",
            ChatStreamType::Done => "done",
            ChatStreamType::SubagentProgress => "subagent_progress",
        }
    }
}

/// Subagent progress phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubagentPhase {
    Thinking,
    Running,
    #[serde(rename = "final-delivery")]
    FinalDelivery,
    #[default]
    Done,
}

impl SubagentPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            SubagentPhase::Thinking => "thinking",
            SubagentPhase::Running => "running",
            SubagentPhase::FinalDelivery => "final-delivery",
            SubagentPhase::Done => "done",
        }
    }
}

/// Tool source: builtin / mcp / plugin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSource {
    Builtin,
    Mcp,
    Plugin,
    #[serde(other)]
    #[default]
    Other,
}

impl ToolSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolSource::Builtin => "builtin",
            ToolSource::Mcp => "mcp",
            ToolSource::Plugin => "plugin",
            ToolSource::Other => "other",
        }
    }
}

/// Skill install source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillInstallSource {
    Skillssh,
    Clawhub,
    Github,
    Auto,
}

impl Default for SkillInstallSource {
    fn default() -> Self {
        SkillInstallSource::Auto
    }
}

/// WeChat login status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WechatLoginStatus {
    Wait,
    Scaned,
    Confirmed,
    Expired,
}

// =====================================================================
// Status / dashboard
// =====================================================================

/// `/api/status` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusResponse {
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_open: Option<bool>,
    pub running: bool,
    pub port: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub uptime: String,
    #[serde(default)]
    pub agents: Vec<AgentInfo>,
    #[serde(default)]
    pub channels: Vec<ChannelInfo>,
    pub provider: ProviderInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_jobs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_admin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub model: String,
    pub workspace: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelInfo {
    #[serde(rename = "type")]
    pub kind: String,
    pub bot_username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub name: String,
    pub model: String,
    pub api_base: String,
    pub api_key: String,
}

// =====================================================================
// Auth
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// `/api/me` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<MeUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act_as_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy_mode: Option<DeployMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeUser {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_quota: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateMeRequest {
    pub display_name: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_e2b_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_boxlite_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_boxlite_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_boxlite_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_boxlite_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// =====================================================================
// Agents
// =====================================================================

/// `/api/agents/{id}` detail. Mirrors the 100+ line `AgentDetail` in
/// `lib/api.ts`. The `plugins` and `skills` fields preserve the
/// original camelCase JSON keys.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDetail {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_public: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_model_config: Option<bool>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_replies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_persist: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soul: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkillsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always_load: Option<Vec<String>>,
}

/// PATCH payload for `/api/agents/{id}`. `camelCase` to match the
/// `splitReplies` / `autoPersist` etc. field names in the JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUpdatePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soul: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<AgentSkillsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_public: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_model_config: Option<bool>,
    /// Allowed values: `""`, `"agent"`, `"chatbot"`, `"customize"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_replies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_replies_reset: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_persist: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_persist_reset: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins_reset: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<AgentSkillsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentRegisteredTool {
    pub name: String,
    pub description: String,
    pub source: ToolSource,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPlugin {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// =====================================================================
// Skills
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEnvSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub location: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_spec: Option<Vec<SkillEnvSpec>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntryCfg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillSearchResult {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub source: String,
    pub installs: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InstallSkillRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillInstallSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InstallSkillResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// =====================================================================
// Plugins
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: String,
    pub status: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

// =====================================================================
// Cron
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CronJobInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub schedule: String,
    pub agent_id: String,
    pub channel: String,
    pub chat_id: String,
    pub message: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentCronJob {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub schedule: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub timezone: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<String>,
    pub created_at: String,
}

// =====================================================================
// Models / providers
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub cost: ModelCost,
    pub context_window: u64,
    pub max_tokens: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderData {
    pub api_key: String,
    pub api_base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelEntry>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRow {
    pub id: String,
    pub scope: ScopeName,
    pub scope_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRow {
    pub id: String,
    pub scope: ScopeName,
    pub scope_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigResponse {
    #[serde(default)]
    pub providers: std::collections::HashMap<String, ProviderData>,
    pub agents: ConfigAgentsBlock,
    #[serde(default)]
    pub channels: std::collections::HashMap<String, ConfigChannelEntry>,
    pub storage: ConfigStorage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<ConfigSandbox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wechat: Option<ConfigWechat>,
    #[serde(default)]
    pub hooks: ConfigHooks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_jobs: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<ConfigSkills>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ConfigMeta>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigAgentsBlock {
    pub defaults: ConfigAgentDefaults,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigAgentDefaults {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_tool_iterations: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigChannelEntry {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigStorage {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsn: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigSandbox {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2b_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxlite_snapshot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2b_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxlite_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxlite_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxlite_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxlite_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigWechat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_replies: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigHooks {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigSkills {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_entries: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_default_model: Option<String>,
}

// =====================================================================
// Chat
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration_cap_reached: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration_cap_value: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_mode: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryMessage {
    pub role: ChatRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ToolResultMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_urls: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_channel: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TodoState {
    pub items: Vec<TodoItem>,
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatHistoryResult {
    pub history: Vec<ChatHistoryMessage>,
    pub latest_event_seq: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub preview: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminChatSessionEntry {
    pub id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub preview: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatStreamEvent {
    #[serde(rename = "type")]
    pub kind: ChatStreamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ChatStreamData>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ToolResultMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SubagentPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SendChatRequest {
    pub agent_id: String,
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SendChatResponse {
    pub response: String,
}

// =====================================================================
// Projects / sessions
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateProjectRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteProjectResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_count: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    pub path: String,
    pub size: u64,
    pub mod_time: u64,
}

// =====================================================================
// Tools (provider-backed capabilities)
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolProviderCatalog {
    pub name: String,
    pub label: String,
    pub needs_key: bool,
    pub needs_url: bool,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCategoryCatalog {
    pub name: String,
    pub label: String,
    pub providers: Vec<ToolProviderCatalog>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolProviderSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCategorySettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallbacks: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_fallback: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub categories: Vec<ToolCategoryCatalog>,
    pub tool_providers: std::collections::HashMap<String, ToolProviderSettings>,
    pub tools: std::collections::HashMap<String, ToolCategorySettings>,
}

// =====================================================================
// Admin: API keys, agent bindings, users
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct APIKey {
    pub id: String,
    pub name: String,
    pub key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct APIKeyCreateResponse {
    pub apikey: APIKey,
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct APIKeyRotateResponse {
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentBindings(pub std::collections::HashMap<String, String>);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindAgentRequest {
    pub api_key_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindAgentResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminCreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_quota: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminUpdateUserRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_quota: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminResetPasswordRequest {
    pub password: String,
}

// =====================================================================
// Per-agent IM channels
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentChannel {
    #[serde(rename = "type")]
    pub kind: String,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_username: Option<String>,
    pub bot_token: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

// =====================================================================
// Token usage
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsageRank {
    pub key: String,
    pub tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsageReport {
    pub range: TokenUsageRange,
    pub totals: TokenUsageTotals,
    pub top_agents: Vec<TokenUsageRank>,
    pub top_users: Vec<TokenUsageRank>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentTokenUsage {
    pub range: TokenUsageRange,
    pub agent_id: String,
    pub sessions: Vec<TokenUsageRank>,
}

// =====================================================================
// Skill install / config update helpers
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkillEntriesBody {
    pub skills: UpdateSkillEntriesInner,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateSkillEntriesInner {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_entries: Option<serde_json::Value>,
}

// =====================================================================
// Connection-result envelopes (used by the per-platform IM bridges)
// =====================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectTelegramResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectDiscordResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectSlackResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StartWeChatLoginResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr_code_img: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PollWeChatLoginResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<WechatLoginStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectLineResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectFeishuResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_open_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_long_conn: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DisconnectChannelResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_name_serializes() {
        assert_eq!(serde_json::to_string(&ScopeName::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::from_str::<ScopeName>("\"user\"").unwrap(), ScopeName::User);
    }

    #[test]
    fn apikey_type_serializes() {
        assert_eq!(serde_json::to_string(&ApikeyType::Admin).unwrap(), "\"admin\"");
    }

    #[test]
    fn token_usage_range_default_is_7d() {
        assert_eq!(TokenUsageRange::default().as_str(), "7d");
    }

    #[test]
    fn status_response_roundtrip() {
        let s = StatusResponse {
            configured: true,
            running: true,
            port: 8080,
            uptime: "1h".into(),
            provider: ProviderInfo {
                name: "openai".into(),
                model: "gpt-4o".into(),
                api_base: "https://api.openai.com/v1".into(),
                api_key: "***".into(),
            },
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        let r: StatusResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(r.port, 8080);
        assert_eq!(r.provider.model, "gpt-4o");
    }

    #[test]
    fn agent_detail_camelcase() {
        let j = r#"{"id":"a1","model":"gpt-4o","maxTokens":1024,"isPublic":true,"shareModelConfig":false}"#;
        let r: AgentDetail = serde_json::from_str(j).unwrap();
        assert_eq!(r.id, "a1");
        assert_eq!(r.max_tokens, Some(1024));
        assert_eq!(r.is_public, Some(true));
        assert_eq!(r.share_model_config, Some(false));
    }

    #[test]
    fn agent_update_payload_camelcase() {
        let j = r#"{"name":"a","splitReplies":true,"splitRepliesReset":false,"autoPersistReset":true}"#;
        let r: AgentUpdatePayload = serde_json::from_str(j).unwrap();
        assert_eq!(r.split_replies, Some(true));
        assert_eq!(r.split_replies_reset, Some(false));
        assert_eq!(r.auto_persist_reset, Some(true));
    }

    #[test]
    fn chat_role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ChatRole::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&ChatRole::Assistant).unwrap(), "\"assistant\"");
    }

    #[test]
    fn chat_stream_type_kebab_for_final_delivery() {
        let p = SubagentPhase::FinalDelivery;
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"final-delivery\"");
    }

    #[test]
    fn config_response_full() {
        let j = r#"{
            "providers": {"openai": {"apiKey":"k","apiBase":"u"}},
            "agents": {"defaults": {"model":"m","maxTokens":1,"temperature":0.5,"maxToolIterations":3}},
            "channels": {"telegram": {"enabled": true}},
            "storage": {"type": "sqlite"},
            "hooks": {"enabled": false}
        }"#;
        let r: ConfigResponse = serde_json::from_str(j).unwrap();
        assert_eq!(r.providers.len(), 1);
        assert_eq!(r.agents.defaults.model, "m");
    }

    #[test]
    fn todo_state_roundtrip() {
        let s = TodoState {
            items: vec![TodoItem { text: "do x".into(), done: false }],
            raw: "- [ ] do x".into(),
        };
        let j = serde_json::to_string(&s).unwrap();
        let r: TodoState = serde_json::from_str(&j).unwrap();
        assert_eq!(r.items.len(), 1);
        assert!(!r.items[0].done);
    }

    #[test]
    fn token_usage_range_serializes() {
        assert_eq!(serde_json::to_string(&TokenUsageRange::H24).unwrap(), "\"24h\"");
        assert_eq!(serde_json::to_string(&TokenUsageRange::D7).unwrap(), "\"7d\"");
        assert_eq!(serde_json::to_string(&TokenUsageRange::D30).unwrap(), "\"30d\"");
    }

    #[test]
    fn channel_row_type_field() {
        let j = r#"{"id":"c","scope":"agent","scopeId":"a","type":"telegram","enabled":true}"#;
        let r: ChannelRow = serde_json::from_str(j).unwrap();
        assert_eq!(r.kind, "telegram");
        assert_eq!(r.scope, ScopeName::Agent);
    }

    #[test]
    fn agent_bindings_transparent() {
        let j = r#"{"a1":"k1","a2":""}"#;
        let r: AgentBindings = serde_json::from_str(j).unwrap();
        assert_eq!(r.0.get("a1").unwrap(), "k1");
        assert_eq!(r.0.get("a2").unwrap(), "");
    }

    #[test]
    fn tool_source_other_deserializes() {
        // `ToolSource::Other` uses `#[serde(other)]` so unknown values
        // don't fail the parse.
        let j = r#"{"name":"x","description":"d","source":"weird"}"#;
        let r: AgentRegisteredTool = serde_json::from_str(j).unwrap();
        assert!(matches!(r.source, ToolSource::Other));
    }

    #[test]
    fn wechat_login_status_kinds() {
        for s in ["wait", "scaned", "confirmed", "expired"] {
            let r: WechatLoginStatus = serde_json::from_str(&format!("\"{s}\"")).unwrap();
            let _ = r; // round-trip ok
        }
    }
}
