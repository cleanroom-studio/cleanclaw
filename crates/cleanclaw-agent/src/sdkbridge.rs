//! SDK bridge.
//!
//! The Go side adapts the `codeany-ai/open-agent-sdk-go` SDK to
//! CleanClaw's `Tool` interface. We don't depend on that SDK here;
//! instead we expose a `ToolBridge` trait that lets any
//! `cleanclaw-agent::tools::Tool` (or third-party implementor) be
//! consumed by an external LLM-loop runner. The concrete loop
//! runtime picks its own bridge target; the agent loop in
//! `loop_runner.rs` uses this trait to stay decoupled.

use async_trait::async_trait;
use cleanclaw_core::Result;
use serde_json::Value;

use crate::tools::{Tool, ToolContext};

/// Adapt any `Tool` into the SDK bridge shape. Returns a JSON
/// Schema object suitable for `tools = [...]` request payloads.
pub fn schema_for(tool: &dyn Tool) -> Value {
    tool.parameters()
}

/// Tools marked `read_only` are safe to run concurrently without
/// serializing on the per-chat task queue. The agent loop reads
/// this to decide whether to spawn a parallel branch.
pub fn is_read_only(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file"
            | "list_dir"
            | "web_fetch"
            | "web_search"
            | "memory_search"
            | "load_skill"
            | "current_time"
            | "http_get"
            | "echo"
    )
}

/// A tool's `name → Tool` map. Useful when an external runner
/// (a test harness, a sidecar, a bridge to a different agent
/// framework) needs the same dispatch surface as the in-process
/// `ToolRegistry` but without the rest of the agent loop.
#[async_trait]
pub trait ToolBridge: Send + Sync {
    /// Names of all registered tools.
    fn names(&self) -> Vec<String>;
    /// JSON-Schema parameter object for `name`.
    fn schema(&self, name: &str) -> Option<Value>;
    /// Synchronous metadata (no I/O).
    fn describe(&self, name: &str) -> Option<BridgeDescriptor>;
    /// Dispatch a call. `ctx` is shared with the agent loop so
    /// per-turn state (channel / chat / session) is observable.
    async fn call(&self, ctx: &ToolContext, name: &str, args: Value) -> Result<Value>;
}

/// Plain-data descriptor for a tool. Useful for log lines and
/// for surfacing the tool catalog to non-Rust callers.
#[derive(Debug, Clone)]
pub struct BridgeDescriptor {
    pub name: String,
    pub description: String,
    pub read_only: bool,
}

/// Default bridge over a list of `Arc<dyn Tool>` (the same shape
/// the agent's `ToolRegistry` uses, but exposed without
/// `register`/`get` accessor methods so external callers can't
/// mutate the registry at runtime).
pub struct StaticBridge {
    tools: Vec<(String, std::sync::Arc<dyn Tool>)>,
}

impl StaticBridge {
    pub fn new(tools: Vec<(String, std::sync::Arc<dyn Tool>)>) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl ToolBridge for StaticBridge {
    fn names(&self) -> Vec<String> {
        self.tools.iter().map(|(n, _)| n.clone()).collect()
    }
    fn schema(&self, name: &str) -> Option<Value> {
        self.tools
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.parameters())
    }
    fn describe(&self, name: &str) -> Option<BridgeDescriptor> {
        self.tools
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| BridgeDescriptor {
                name: name.to_string(),
                description: t.description().to_string(),
                read_only: is_read_only(name),
            })
    }
    async fn call(&self, ctx: &ToolContext, name: &str, args: Value) -> Result<Value> {
        let tool = self
            .tools
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone());
        let tool =
            tool.ok_or_else(|| cleanclaw_core::CleanClawError::NotFound(format!("tool {name}")))?;
        tool.call(ctx, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use async_trait::async_trait;
    use serde_json::json;

    struct DummyTool;
    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }
        async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
            Ok(json!({"ok": true}))
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::default()
    }

    #[tokio::test]
    async fn bridge_names_and_schema() {
        let bridge = StaticBridge::new(vec![("dummy".to_string(), std::sync::Arc::new(DummyTool))]);
        assert_eq!(bridge.names(), vec!["dummy".to_string()]);
        let s = bridge.schema("dummy").unwrap();
        assert_eq!(s["type"], "object");
    }

    #[tokio::test]
    async fn bridge_call_dispatches() {
        let bridge = StaticBridge::new(vec![("dummy".to_string(), std::sync::Arc::new(DummyTool))]);
        let r = bridge.call(&ctx(), "dummy", json!({})).await.unwrap();
        assert_eq!(r["ok"], true);
    }

    #[tokio::test]
    async fn bridge_unknown_tool_errors() {
        let bridge = StaticBridge::new(vec![]);
        let r = bridge.call(&ctx(), "nope", json!({})).await;
        assert!(r.is_err());
    }

    #[test]
    fn read_only_set_covers_safe_tools() {
        assert!(is_read_only("read_file"));
        assert!(is_read_only("web_fetch"));
        assert!(!is_read_only("exec"));
        assert!(!is_read_only("apply_patch"));
    }
}
