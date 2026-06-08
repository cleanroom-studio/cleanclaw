//! `delegate` tool — round-robin / weighted delegation to a pool of
//! agents.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub trait DelegateRouter: Send + Sync {
    /// Pick a target agent_id from the configured pool. Returns Err
    /// if the pool is empty.
    fn pick(&self) -> Result<String>;
}

pub struct DelegateTool {
    pub router: Arc<dyn DelegateRouter>,
    pub spawner: Arc<dyn super::subagent::SubAgentSpawner>,
    pub caller_agent_id: String,
}

#[derive(Deserialize)]
struct Args {
    task: String,
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "Delegate a task to a worker agent picked from a configured pool. The worker's response is returned."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "What the worker should do"}
            },
            "required": ["task"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: Args = serde_json::from_value(args)?;
        if a.task.is_empty() {
            return Err(CleanClawError::InvalidArgument("task is required".into()));
        }
        let target = self.router.pick()?;
        let reply = self
            .spawner
            .spawn_subagent(&self.caller_agent_id, &target, &a.task)?;
        Ok(json!({"agent_id": target, "reply": reply}))
    }
}
