//! `goal` tool set — long-running objectives that the agent auto-renews
//! across sessions.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_store::models::GoalRecord;
use cleanclaw_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct GoalTool {
    pub store: Arc<dyn Store>,
}

#[derive(Deserialize)]
struct CreateArgs {
    objective: String,
    #[serde(default)]
    token_budget: Option<i64>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
}

#[async_trait]
impl Tool for GoalTool {
    fn name(&self) -> &str {
        "create_goal"
    }
    fn description(&self) -> &str {
        "Create a long-running goal for this session. The agent will continue working on the objective in the background, up to an optional token budget."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {"type": "string"},
                "token_budget": {"type": "integer", "description": "Optional max tokens"},
                "channel": {"type": "string", "description": "Optional channel override"},
                "chat_id": {"type": "string"},
                "project_id": {"type": "string"}
            },
            "required": ["objective"]
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: CreateArgs = serde_json::from_value(args)?;
        if a.objective.is_empty() {
            return Err(CleanClawError::InvalidArgument(
                "objective is required".into(),
            ));
        }
        let now = cleanclaw_core::now_utc();
        let g = GoalRecord {
            id: cleanclaw_core::IdGen::new().next("goal"),
            agent_id: ctx.agent_id.clone(),
            session_key: ctx.session_key.clone(),
            owner_user_id: ctx.owner_user_id.clone(),
            channel: a.channel.unwrap_or_else(|| ctx.channel.clone()),
            account_id: ctx.account_id.clone(),
            chat_id: a.chat_id.unwrap_or_else(|| ctx.chat_id.clone()),
            project_id: a.project_id.unwrap_or_else(|| ctx.project_id.clone()),
            objective: a.objective,
            status: "active".into(),
            token_budget: a.token_budget,
            tokens_used: 0,
            created_at: now,
            updated_at: now,
        };
        self.store.save_goal(&g).await?;
        Ok(json!({"id": g.id, "status": g.status, "objective": g.objective}))
    }
}

pub struct ListGoalsTool {
    pub store: Arc<dyn Store>,
    pub agent_id: String,
}

#[async_trait]
impl Tool for ListGoalsTool {
    fn name(&self) -> &str {
        "list_goals"
    }
    fn description(&self) -> &str {
        "List the long-running goals attached to this agent."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
        let goals = self.store.list_goals(&self.agent_id).await?;
        Ok(json!({"goals": goals}))
    }
}

pub struct DeleteGoalTool {
    pub store: Arc<dyn Store>,
    pub agent_id: String,
}

#[async_trait]
impl Tool for DeleteGoalTool {
    fn name(&self) -> &str {
        "delete_goal"
    }
    fn description(&self) -> &str {
        "Delete a goal by (agent_id, session_key)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_key": {"type": "string", "description": "The session key the goal is attached to"}
            },
            "required": ["session_key"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: DeleteArgs = serde_json::from_value(args)?;
        self.store
            .delete_goal(&self.agent_id, &a.session_key)
            .await?;
        Ok(json!({"ok": true}))
    }
}

#[derive(Deserialize)]
struct DeleteArgs {
    session_key: String,
}
