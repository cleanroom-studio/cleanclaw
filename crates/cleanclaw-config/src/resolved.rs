//! Resolved per-agent view of the merged config tree.
//!
//!

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResolvedAgent {
    pub id: String,
    #[serde(rename = "userId", default)]
    pub user_id: String,
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    pub home: String,
    pub workspace: String,
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
    pub skills: SkillsConfig,
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    pub sandbox: SandboxCfg,
    #[serde(default)]
    pub policy_preset: String,
    #[serde(rename = "toolProviders", default)]
    pub tool_providers: HashMap<String, ToolProviderCfg>,
    #[serde(default)]
    pub tools: HashMap<String, ToolCategoryCfg>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// Per-channel admin allowlist for write-mode slash commands.
    #[serde(default)]
    pub admins: HashMap<String, Vec<String>>,
    /// Agent-scope `agents.defaults.*` overrides (post-resolution).
    #[serde(default)]
    pub overrides: AgentDefaults,
}
