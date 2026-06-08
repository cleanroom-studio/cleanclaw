//! Runtime Config: assembled at gateway boot from env + system-scope DB rows.
//!
//! Mirrors the union type in .

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub channels: HashMap<String, ChannelConfig>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub teams: HashMap<String, TeamEntry>,
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(rename = "cronJobs", default)]
    pub cron_jobs: Vec<CronJob>,
    #[serde(default)]
    pub heartbeat: HeartbeatCfg,
    #[serde(default)]
    pub storage: StorageCfg,
    #[serde(default)]
    pub sandbox: SandboxCfg,
    #[serde(rename = "toolProviders", default)]
    pub tool_providers: HashMap<String, ToolProviderCfg>,
    #[serde(default)]
    pub tools: HashMap<String, ToolCategoryCfg>,
    #[serde(rename = "objectStore", default)]
    pub object_store: ObjectStoreCfg,
    #[serde(default)]
    pub hooks: HooksCfg,
    #[serde(default)]
    pub plugins: PluginsCfg,
    #[serde(default)]
    pub gateway: GatewayCfg,
    #[serde(rename = "taskQueue", default)]
    pub task_queue: TaskQueueCfg,
    #[serde(default)]
    pub skills: SkillsCfg,
    #[serde(default)]
    pub memory: MemoryCfg,
    #[serde(default)]
    pub privacy: PrivacyCfg,
    #[serde(rename = "skillsLearner", default)]
    pub skills_learner: SkillsLearnerCfg,
}

impl Config {
    pub fn empty() -> Self {
        Self::default()
    }
}
