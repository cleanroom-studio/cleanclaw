//! `spawn_subagent` tool — delegate a task to another agent.
//!
//! The actual
//! routing is done by the `SubAgentSpawner` trait implemented by the
//! gateway / chat service.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub trait SubAgentSpawner: Send + Sync {
    fn spawn_subagent(
        &self,
        parent_agent_id: &str,
        target_agent_id: &str,
        task: &str,
    ) -> Result<String>;
}

pub struct SpawnSubAgentTool {
    pub spawner: Arc<dyn SubAgentSpawner>,
    pub caller_agent_id: String,
}

#[derive(Deserialize)]
struct Args {
    agent_id: String,
    task: String,
}

#[async_trait]
impl Tool for SpawnSubAgentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }
    fn description(&self) -> &str {
        "Spawn another agent as a sub-task and return its response. Use this to delegate work to specialized agents."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agentId": {"type": "string", "description": "ID of the agent to spawn"},
                "task": {"type": "string", "description": "The message/prompt to send"}
            },
            "required": ["agentId", "task"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: Args = serde_json::from_value(args)?;
        if a.agent_id.is_empty() {
            return Err(CleanClawError::InvalidArgument(
                "agentId is required".into(),
            ));
        }
        if a.task.is_empty() {
            return Err(CleanClawError::InvalidArgument("task is required".into()));
        }
        if a.agent_id == self.caller_agent_id {
            return Err(CleanClawError::InvalidArgument(
                "cannot spawn yourself as a sub-agent".into(),
            ));
        }
        let reply = self
            .spawner
            .spawn_subagent(&self.caller_agent_id, &a.agent_id, &a.task)?;
        Ok(json!({"agent_id": a.agent_id, "reply": reply}))
    }
}

use std::sync::Arc;
