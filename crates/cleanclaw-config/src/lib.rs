//! In-memory runtime configuration assembled at gateway boot from env +
//! system-scope DB rows.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod env;
pub mod resolved;
pub mod runtime;
pub mod scope;

pub use env::{
    home_dir, load_env, scrub_boot_secrets, EnvConfig, EnvGateway, EnvLog, EnvSandbox, EnvStorage,
};
pub use resolved::ResolvedAgent;
pub use runtime::*;
pub use scope::Scope;

// ---- Primitives --------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead", default)]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite", default)]
    pub cache_write: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub reasoning: bool,
    pub input: Vec<String>,
    #[serde(default)]
    pub cost: ModelCost,
    #[serde(rename = "contextWindow", default)]
    pub context_window: u32,
    #[serde(rename = "maxTokens", default)]
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(rename = "apiKey", default)]
    pub api_key: String,
    #[serde(rename = "apiBase", default)]
    pub api_base: String,
    #[serde(rename = "apiType", default)]
    pub api_type: String,
    #[serde(rename = "authType", default)]
    pub auth_type: String,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

// ---- Tool provider / chain ---------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolProviderCfg {
    #[serde(rename = "apiKey", default)]
    pub api_key: String,
    pub endpoint: String,
    #[serde(default)]
    pub options: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCategoryCfg {
    pub primary: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
    #[serde(rename = "autoFallback")]
    pub auto_fallback: Option<bool>,
}

impl ToolCategoryCfg {
    pub fn fallback_enabled(&self) -> bool {
        self.auto_fallback.unwrap_or(true)
    }

    pub fn chain(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.primary.is_empty() {
            out.push(self.primary.clone());
        }
        for f in &self.fallbacks {
            if !f.is_empty() {
                out.push(f.clone());
            }
        }
        out
    }
}

// ---- Sandbox / Cron / Hooks --------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxCfg {
    #[serde(default)]
    pub enabled: bool,
    pub image: String,
    #[serde(rename = "dockerImage", default)]
    pub docker_image: String,
    #[serde(rename = "e2bTemplate", default)]
    pub e2b_template: String,
    #[serde(rename = "boxliteSnapshot", default)]
    pub boxlite_snapshot: String,
    pub policy: String,
    pub backend: String,
    #[serde(rename = "e2bKey", default)]
    pub e2b_key: String,
    #[serde(rename = "boxliteUrl", default)]
    pub boxlite_url: String,
    #[serde(rename = "boxliteClientId", default)]
    pub boxlite_client_id: String,
    #[serde(rename = "boxliteKey", default)]
    pub boxlite_key: String,
    #[serde(rename = "boxlitePrefix", default)]
    pub boxlite_prefix: String,
    pub network: String,
    #[serde(rename = "idleTTLSec", default)]
    pub idle_ttl_sec: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksCfg {
    #[serde(default)]
    pub enabled: bool,
    pub token: String,
    pub path: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginEntryCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginsCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub entries: HashMap<String, PluginEntryCfg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskQueueCfg {
    #[serde(rename = "maxConcurrent", default)]
    pub max_concurrent: i32,
    #[serde(rename = "taskTimeoutSec", default)]
    pub task_timeout_sec: i64,
}

// ---- Agent / Channel ---------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDefaults {
    pub model: String,
    #[serde(rename = "maxTokens", default)]
    pub max_tokens: u32,
    pub temperature: f64,
    #[serde(rename = "maxToolIterations", default)]
    pub max_tool_iterations: u32,
    #[serde(rename = "maxParallelToolCalls", default)]
    pub max_parallel_tool_calls: u32,
    pub thinking: String,
    #[serde(default)]
    pub policy: String,
    #[serde(rename = "promptMode", default)]
    pub prompt_mode: String,
    #[serde(rename = "splitReplies", default)]
    pub split_replies: Option<bool>,
    #[serde(rename = "autoPersist", default)]
    pub auto_persist: Option<bool>,
}

pub const PROMPT_MODE_AGENT: &str = "agent";
pub const PROMPT_MODE_CHATBOT: &str = "chatbot";
pub const PROMPT_MODE_CUSTOMIZE: &str = "customize";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentsConfig {
    #[serde(default)]
    pub defaults: AgentDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountConfig {
    #[serde(rename = "botToken", default)]
    pub bot_token: String,
    #[serde(rename = "appToken", default)]
    pub app_token: String,
    #[serde(rename = "baseUrl", default)]
    pub base_url: String,
    #[serde(rename = "userId", default)]
    pub user_id: String,
    #[serde(rename = "encryptKey", default)]
    pub encrypt_key: String,
    #[serde(rename = "useLongConn", default)]
    pub use_long_conn: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "botToken", default)]
    pub bot_token: String,
    #[serde(rename = "appToken", default)]
    pub app_token: String,
    #[serde(default)]
    pub accounts: HashMap<String, AccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Peer {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Match {
    pub channel: String,
    #[serde(rename = "accountId", default)]
    pub account_id: String,
    pub peer: Option<Peer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Binding {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    #[serde(default)]
    pub r#match: Match,
}

// ---- Skills / Memory / Privacy ----------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsConfig {
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(rename = "alwaysLoad", default)]
    pub always_load: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillEntryCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "apiKey", default)]
    pub api_key: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsLoadCfg {
    #[serde(rename = "extraDirs", default)]
    pub extra_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsInstallCfg {
    #[serde(rename = "nodeManager", default)]
    pub node_manager: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsCfg {
    #[serde(default)]
    pub install: SkillsInstallCfg,
    #[serde(default)]
    pub entries: HashMap<String, SkillEntryCfg>,
    #[serde(rename = "agentEntries", default)]
    pub agent_entries: HashMap<String, HashMap<String, SkillEntryCfg>>,
    #[serde(default)]
    pub load: SkillsLoadCfg,
    #[serde(rename = "alwaysLoad", default)]
    pub always_load: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoPersistCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "everyNTurns", default)]
    pub every_n_turns: u32,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FtsCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "dbPath", default)]
    pub db_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryCfg {
    #[serde(rename = "autoPersist", default)]
    pub auto_persist: AutoPersistCfg,
    #[serde(default)]
    pub fts: FtsCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PiiScrubCfg {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrivacyCfg {
    #[serde(rename = "piiScrubbing", default)]
    pub pii_scrubbing: PiiScrubCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsLearnerCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "minToolCalls", default)]
    pub min_tool_calls: u32,
    pub model: String,
}

// ---- Object store / Heartbeat / MCPServer / Gateway -------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    #[serde(rename = "accessKey", default)]
    pub access_key: String,
    #[serde(rename = "secretKey", default)]
    pub secret_key: String,
    #[serde(rename = "useSSL", default)]
    pub use_ssl: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalObjectStoreCfg {
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectStoreCfg {
    pub r#type: String,
    #[serde(default)]
    pub local: LocalObjectStoreCfg,
    #[serde(rename = "accountId", default)]
    pub account_id: String,
    #[serde(rename = "aliyunIntern", default)]
    pub aliyun_intern: bool,
    #[serde(default)]
    pub s3: S3Config,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeartbeatCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "everyNMinutes", default)]
    pub every_n_minutes: u32,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamEntry {
    pub name: String,
    #[serde(default)]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronJob {
    pub id: String,
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub name: String,
    pub r#type: String,
    pub schedule: String,
    pub message: String,
    pub channel: String,
    #[serde(rename = "chatId")]
    pub chat_id: String,
    pub timezone: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageCfg {
    pub r#type: String,
    pub dsn: String,
    #[serde(rename = "autoMigrate", default)]
    pub auto_migrate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitCfg {
    pub rpm: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayAuth {
    pub mode: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayEndpoint {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayHttpEndpoints {
    #[serde(rename = "chatCompletions", default)]
    pub chat_completions: GatewayEndpoint,
    #[serde(default)]
    pub agents: GatewayEndpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayHttp {
    #[serde(default)]
    pub endpoints: GatewayHttpEndpoints,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayCfg {
    pub port: u16,
    pub bind: String,
    #[serde(default)]
    pub auth: GatewayAuth,
    #[serde(default)]
    pub http: GatewayHttp,
    #[serde(rename = "rateLimit", default)]
    pub rate_limit: RateLimitCfg,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_category_chain_with_primary_and_fallbacks() {
        let c = ToolCategoryCfg {
            primary: "brave/web".into(),
            fallbacks: vec!["exa/auto".into(), "searxng/default".into()],
            auto_fallback: Some(false),
        };
        assert_eq!(c.chain(), vec!["brave/web", "exa/auto", "searxng/default"]);
        assert!(!c.fallback_enabled());
    }

    #[test]
    fn tool_category_fallback_defaults_to_true() {
        let c = ToolCategoryCfg {
            primary: "brave".into(),
            fallbacks: vec![],
            auto_fallback: None,
        };
        assert!(c.fallback_enabled());
    }
}

// =====================================================================
// Convenience helpers — Mirrors the Go `config.MergedAgentConfig`
// and `config.LoadEnv` glue that the gateway boot path depends on.
// =====================================================================

impl Config {
    /// Look up an MCP server by name. Returns the empty string when
    /// the key is missing — callers handle the default.
    pub fn mcp_server_url(&self, name: &str) -> String {
        self.mcp_servers
            .get(name)
            .map(|s| s.url.clone())
            .unwrap_or_default()
    }

    /// Look up a tool category config by name.
    pub fn tool_category(&self, name: &str) -> Option<&ToolCategoryCfg> {
        self.tools.get(name)
    }

    /// Number of configured providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Number of configured channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

#[cfg(test)]
mod config_helpers_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn mcp_server_url_returns_present() {
        let mut servers = HashMap::new();
        servers.insert(
            "github".to_string(),
            McpServerConfig {
                url: "https://example.com".into(),
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                headers: HashMap::new(),
            },
        );
        let c = Config {
            mcp_servers: servers,
            ..Config::default()
        };
        assert_eq!(c.mcp_server_url("github"), "https://example.com");
    }

    #[test]
    fn mcp_server_url_missing_returns_empty() {
        let c = Config::default();
        assert_eq!(c.mcp_server_url("nope"), "");
    }

    #[test]
    fn tool_category_lookup() {
        let mut tools = HashMap::new();
        tools.insert(
            "web_search".to_string(),
            ToolCategoryCfg {
                primary: "brave".into(),
                fallbacks: vec![],
                auto_fallback: None,
            },
        );
        let c = Config {
            tools,
            ..Config::default()
        };
        let tc = c.tool_category("web_search").unwrap();
        assert_eq!(tc.primary, "brave");
    }

    #[test]
    fn tool_category_missing_returns_none() {
        let c = Config::default();
        assert!(c.tool_category("nope").is_none());
    }

    #[test]
    fn provider_and_channel_counts() {
        let mut providers = HashMap::new();
        providers.insert("a".to_string(), ProviderConfig::default());
        providers.insert("b".to_string(), ProviderConfig::default());
        let mut channels = HashMap::new();
        channels.insert("telegram".to_string(), ChannelConfig::default());
        let c = Config {
            providers,
            channels,
            ..Config::default()
        };
        assert_eq!(c.provider_count(), 2);
        assert_eq!(c.channel_count(), 1);
    }
}
