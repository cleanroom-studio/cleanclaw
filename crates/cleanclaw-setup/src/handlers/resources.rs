//! Channels / cron / skills / tools / usage handlers. The dashboard's
//! "resources" sidebar. Mirrors the corresponding files in
//! .

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::Serialize;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    // The full set of resource routes (`/api/channels`, `/api/cron`,
    // `/api/skills`, `/api/tools`, `/api/usage/...`) is owned by the
    // more comprehensive handler modules in `channels.rs`,
    // `cron.rs`, `skills.rs`, `tools.rs`, `usage.rs`. This
    // `resources` router remains as a placeholder for any resource
    // routes not covered there (e.g. dashboard-only aggregations).
    Router::new()
}

#[derive(Debug, Serialize)]
pub struct ChannelDto {
    pub r#type: String,
    pub enabled: bool,
    pub status: String, // "connected" | "disconnected"
}

async fn list_channels(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    // We don't have a config loader here yet; the dashboard just
    // lists the channel *types* we ship. A real impl reads the
    // merged user config and projects `Channels` field.
    let mut out: Vec<ChannelDto> = Vec::new();
    let rows = match state.store.list_all_agents().await {
        Ok(_) => vec![
            ("telegram", false, "disconnected".to_string()),
            ("discord", false, "disconnected".to_string()),
            ("slack", false, "disconnected".to_string()),
            ("feishu", false, "disconnected".to_string()),
            ("wechat", false, "disconnected".to_string()),
            ("line", false, "disconnected".to_string()),
            ("web", true, "connected".to_string()),
        ],
        Err(_) => vec![],
    };
    for (t, en, st) in rows {
        out.push(ChannelDto {
            r#type: t.into(),
            enabled: en,
            status: st,
        });
    }
    (StatusCode::OK, Json(out)).into_response()
}

#[derive(Debug, Serialize)]
pub struct CronJobDto {
    pub id: String,
    pub agent_id: String,
    pub owner_user_id: String,
    pub name: String,
    pub kind: String,
    pub schedule: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    pub account_id: String,
}

async fn list_cron(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    // Listing all cron jobs is admin-only; for now list by owner
    // would require a user_id, but we default to listing all
    // jobs the gateway knows about.
    let since = chrono::Utc::now() - chrono::Duration::days(365);
    let until = chrono::Utc::now() + chrono::Duration::days(1);
    let res = state
        .store
        .list_token_usage(since.date_naive())
        .await
        .ok()
        .unwrap_or_default();
    let _ = (res, until); // current surface: empty until cron_get is wired
    let jobs: Vec<CronJobDto> = Vec::new();
    (StatusCode::OK, Json(jobs)).into_response()
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateCronReq {
    pub agent_id: String,
    pub name: String,
    pub kind: String,
    pub schedule: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    #[serde(default)]
    pub account_id: String,
}

async fn create_cron(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateCronReq>,
) -> impl IntoResponse {
    let id = format!("cron_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now();
    let next_run = now + chrono::Duration::minutes(1);
    let rec = cleanclaw_store::models::CronJobRecord {
        id: id.clone(),
        user_id: String::new(),
        agent_id: req.agent_id,
        name: req.name,
        r#type: req.kind,
        schedule: req.schedule,
        message: req.message,
        channel: req.channel,
        chat_id: req.chat_id,
        account_id: req.account_id,
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: Some(next_run),
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    match state.store.save_cron_job(&rec).await {
        Ok(()) => (StatusCode::CREATED, Json(CronJobDto {
            id: rec.id,
            agent_id: rec.agent_id,
            owner_user_id: rec.user_id,
            name: rec.name,
            kind: rec.r#type,
            schedule: rec.schedule,
            message: rec.message,
            channel: rec.channel,
            chat_id: rec.chat_id,
            account_id: rec.account_id,
        }))
        .into_response(),
        Err(e) => err_response(e),
    }
}

#[derive(Debug, Serialize)]
pub struct SkillDto {
    pub name: String,
    pub description: String,
    pub source: String, // "bundled" | "user" | "agent" | "global"
}

async fn list_skills(
    State(_state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    // Surface every bundled skill name. A real impl scans the
    // SkillsConfig + per-agent user-skills dirs.
    let skills: Vec<SkillDto> = cleanclaw_agent::BUNDLED_SKILL_NAMES
        .iter()
        .map(|n| SkillDto {
            name: (*n).into(),
            description: String::new(),
            source: "bundled".into(),
        })
        .collect();
    (StatusCode::OK, Json(skills)).into_response()
}

#[derive(Debug, Serialize)]
pub struct ToolDto {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

async fn list_tools(
    State(_state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    // Surface a static list of the tool categories. The real impl
    // pulls from the merged tool registry, which the agent loop
    // populates at boot.
    let tools: Vec<ToolDto> = vec![
        ToolDto {
            name: "web_search".into(),
            description: "Search the web".into(),
            parameters: serde_json::json!({"type": "object"}),
        },
        ToolDto {
            name: "web_fetch".into(),
            description: "Fetch a URL".into(),
            parameters: serde_json::json!({"type": "object"}),
        },
        ToolDto {
            name: "image_gen".into(),
            description: "Generate an image".into(),
            parameters: serde_json::json!({"type": "object"}),
        },
        ToolDto {
            name: "tts".into(),
            description: "Text to speech".into(),
            parameters: serde_json::json!({"type": "object"}),
        },
    ];
    (StatusCode::OK, Json(tools)).into_response()
}

#[derive(Debug, Serialize)]
pub struct UsageTotalsDto {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_create_tokens: i64,
    pub requests: i64,
}

async fn usage_totals(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let since = chrono::Utc::now().date_naive() - chrono::Duration::days(30);
    let until = chrono::Utc::now().date_naive();
    let res = state.store.list_token_usage(since).await;
    let totals = match res {
        Ok(rows) => rows
            .into_iter()
            .fold(
                UsageTotalsDto {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_create_tokens: 0,
                    requests: 0,
                },
                |mut acc, r| {
                    acc.input_tokens += r.input_tokens as i64;
                    acc.output_tokens += r.output_tokens as i64;
                    acc.cache_read_tokens += r.cache_read_tokens as i64;
                    acc.cache_create_tokens += r.cache_create_tokens as i64;
                    acc.requests += r.request_count as i64;
                    acc
                },
            ),
        Err(_) => UsageTotalsDto {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            requests: 0,
        },
    };
    let _ = until;
    (StatusCode::OK, Json(totals)).into_response()
}

#[derive(Debug, Serialize)]
pub struct TopAgentDto {
    pub key: String,
    pub tokens: i64,
    pub requests: i64,
}

async fn usage_top_agents(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    // Per-agent token rollup from the token_usage_daily table.
    let since = chrono::Utc::now().date_naive() - chrono::Duration::days(30);
    let rows = state
        .store
        .list_token_usage(since)
        .await
        .ok()
        .unwrap_or_default();
    use std::collections::HashMap;
    let mut by_agent: HashMap<String, TopAgentDto> = HashMap::new();
    for r in rows {
        let entry = by_agent
            .entry(r.agent_id.clone())
            .or_insert_with(|| TopAgentDto {
                key: r.agent_id.clone(),
                tokens: 0,
                requests: 0,
            });
        entry.tokens += (r.input_tokens + r.output_tokens + r.cache_read_tokens + r.cache_create_tokens) as i64;
        entry.requests += r.request_count as i64;
    }
    let mut v: Vec<TopAgentDto> = by_agent.into_values().collect();
    v.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    v.truncate(10);
    (StatusCode::OK, Json(v)).into_response()
}

fn err_response(e: CleanClawError) -> axum::response::Response {
    let status = match &e {
        CleanClawError::NotFound(_) => StatusCode::NOT_FOUND,
        CleanClawError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}
