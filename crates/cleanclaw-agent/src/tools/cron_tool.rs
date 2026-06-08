//! Built-in `cron` tool.
//!
//! Lets the LLM schedule reminders / recurring jobs ("5 分钟后提醒
//! 我", "每天 9 点喝水", "每 30 分钟检查一次"). The actual firing is
//! handled by the cron scheduler in the gateway.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use chrono::Utc;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_cron::{compute_next_run, new_job, parse_duration, validate_cron, validate_once};
use cleanclaw_store::models::CronJobRecord;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct CronTool {
    pub store: Arc<dyn Store>,
    pub user_id: String,
    pub agent_id: String,
}

impl CronTool {
    pub fn new(store: Arc<dyn Store>, user_id: String, agent_id: String) -> Self {
        Self { store, user_id, agent_id }
    }
}

#[derive(Deserialize, Serialize)]
struct CreateArgs {
    name: String,
    schedule: String,
    message: String,
    /// "cron" | "interval" | "once"
    r#type: Option<String>,
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &str {
        "create_cron_job"
    }
    fn description(&self) -> &str {
        "Create a scheduled task. Use this for any user request that names a specific time, an interval, or a recurring schedule (e.g. \"5 分钟后提醒\", \"every Monday 9am\", \"each day at 8\"). When the schedule fires, the agent receives `message` as a fresh inbound prompt on the same channel the request originated from."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Short task name (for listing / debugging)."
                },
                "schedule": {
                    "type": "string",
                    "description": "When to fire. For type='cron': a cron expression like '0 9 * * *'. For type='interval': a duration like '5m' / '30m' / '2h'. For type='once': an ISO-8601 datetime in UTC like '2026-05-02T15:56:52'."
                },
                "message": {
                    "type": "string",
                    "description": "The prompt the agent should receive when the schedule fires."
                },
                "type": {
                    "type": "string",
                    "description": "Schedule type. 'once' / 'cron' / 'interval'. Defaults to 'interval' if the schedule parses as a duration, else 'cron'."
                }
            },
            "required": ["name", "schedule", "message"]
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: CreateArgs = serde_json::from_value(args)
            .map_err(|e| CleanClawError::InvalidArgument(format!("parse: {e}")))?;
        if a.name.is_empty() || a.schedule.is_empty() || a.message.is_empty() {
            return Err(CleanClawError::InvalidArgument(
                "name, schedule, and message are required".into(),
            ));
        }

        // Determine type if not specified
        let type_ = a.r#type.unwrap_or_else(|| {
            if validate_cron(&a.schedule).is_ok() {
                "cron".into()
            } else if parse_duration(&a.schedule).is_ok() {
                "interval".into()
            } else if validate_once(&a.schedule).is_ok() {
                "once".into()
            } else {
                "cron".into()
            }
        });

        // Validate
        match type_.as_str() {
            "cron" => {
                validate_cron(&a.schedule)?;
            }
            "interval" => {
                parse_duration(&a.schedule)?;
            }
            "once" => {
                validate_once(&a.schedule)?;
            }
            other => {
                return Err(CleanClawError::InvalidArgument(format!(
                    "unknown cron type: {other}"
                )))
            }
        }

        let mut job = new_job(
            &self.agent_id,
            &a.name,
            &type_,
            &a.schedule,
            &a.message,
            &ctx.channel,
            &ctx.chat_id,
            &ctx.account_id,
        );
        job.user_id = self.user_id.clone();

        // Compute next_run
        let now = Utc::now();
        let next_run: Option<chrono::DateTime<Utc>> = match type_.as_str() {
            "once" => validate_once(&a.schedule).ok(),
            _ => compute_next_run(&job, now).ok(),
        };
        job.next_run = next_run;

        self.store.save_cron_job(&job).await?;
        Ok(json!({
            "ok": true,
            "id": job.id,
            "name": job.name,
            "type": job.r#type,
            "schedule": job.schedule,
            "next_run": job.next_run,
        }))
    }
}

pub struct ListCronTool {
    pub store: Arc<dyn Store>,
    pub agent_id: String,
}

impl ListCronTool {
    pub fn new(store: Arc<dyn Store>, agent_id: String) -> Self {
        Self { store, agent_id }
    }
}

#[async_trait]
impl Tool for ListCronTool {
    fn name(&self) -> &str {
        "list_cron_jobs"
    }
    fn description(&self) -> &str {
        "List all scheduled tasks for this agent."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
        let jobs = self.store.list_cron_jobs_by_agent(&self.agent_id).await?;
        let filtered: Vec<&CronJobRecord> = jobs.iter().collect();
        Ok(json!({"jobs": filtered}))
    }
}

pub struct DeleteCronTool {
    pub store: Arc<dyn Store>,
    pub user_id: String,
}

impl DeleteCronTool {
    pub fn new(store: Arc<dyn Store>, user_id: String) -> Self {
        Self { store, user_id }
    }
}

#[async_trait]
impl Tool for DeleteCronTool {
    fn name(&self) -> &str {
        "delete_cron_job"
    }
    fn description(&self) -> &str {
        "Delete a scheduled task by ID."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The cron job ID to delete"}
            },
            "required": ["id"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CleanClawError::InvalidArgument("id required".into()))?;
        self.store.delete_cron_job(id).await?;
        Ok(json!({"ok": true, "id": id}))
    }
}
