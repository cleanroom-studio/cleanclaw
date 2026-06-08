//! Tool registry.
//!
//! Tools are async functions that take a `Value` and return a
//! `Value`. The registry dispatches tool calls from the LLM by name.

pub mod apply_patch;
pub mod bash_session;
pub mod builtins;
pub mod cron_tool;
pub mod delegate;
pub mod env_scrub;
pub mod exec;
pub mod file;
pub mod goal;
pub mod load_skill;
pub mod media;
pub mod memory_search;
pub mod skill_install;
pub mod subagent;
pub mod web;

use async_trait::async_trait;
use cleanclaw_core::Result;
use cleanclaw_provider::{Message, ToolDefinition};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub type ToolFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>;

/// Per-tool execution context — gives tools access to the agent's
/// per-turn state (channel / chat / session) without going through a
/// global.
#[derive(Clone, Default)]
pub struct ToolContext {
    pub agent_id: String,
    pub owner_user_id: String,
    pub chatter_user_id: String,
    pub channel: String,
    pub chat_id: String,
    pub account_id: String,
    pub session_key: String,
    pub project_id: String,
    pub is_admin: bool,
    pub workspace_root: String,
    pub extra: Arc<HashMap<String, Value>>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, t: Arc<dyn Tool>) {
        self.tools.insert(t.name().to_string(), t);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tools.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn as_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect()
    }

    pub async fn dispatch(&self, ctx: &ToolContext, name: &str, args: Value) -> Result<Value> {
        let t = self
            .tools
            .get(name)
            .ok_or_else(|| cleanclaw_core::CleanClawError::NotFound(format!("tool {name}")))?;
        t.call(ctx, args).await
    }
}

pub fn tool_result_message(tool_call_id: &str, content: &str) -> Message {
    let mut m = Message::tool_result(tool_call_id, content);
    m.content = content.to_string();
    m
}

pub fn tool_definitions_message(tools: &[ToolDefinition]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut out = String::from("Available tools:\n");
    for t in tools {
        out.push_str(&format!("- `{}` — {}\n", t.name, t.description));
    }
    out
}

// =====================================================================
// Image generation tool. Mirrors
// .
// =====================================================================

pub mod image_gen;
pub mod tts;
