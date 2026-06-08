//! Cron handlers.
//!
//! Routes:
//!   * GET    /api/cron                  - list all cron jobs (admin)
//!   * POST   /api/cron                  - create a cron job
//!   * PUT    /api/cron/:id              - update a cron job
//!   * DELETE /api/cron/:id              - delete
//!   * GET    /api/agents/:id/cron       - per-agent jobs
//!   * DELETE /api/agents/:id/cron/:job  - per-agent delete
//!   * PUT    /api/agents/:id/cron/:job  - toggle enabled flag
//!
//! The per-agent subset duplicates what's in
//! `cleanclaw-api::cron_endpoints`. To keep both surfaces live and
//! consistent, this file re-exports the agent-scoped routes and
//! adds the global list/create/update/delete.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use chrono::{DateTime, Utc};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/cron", get(list_all).post(create))
        .route("/api/cron/:id", put(update_job).delete(delete_job))
        .route("/api/agents/:id/cron", get(list_for_agent))
        .route(
            "/api/agents/:id/cron/:job_id",
            delete(delete_for_agent).put(toggle_for_agent),
        )
}

#[derive(Serialize)]
struct CronJobDto {
    id: String,
    user_id: String,
    agent_id: String,
    name: String,
    r#type: String,
    schedule: String,
    message: String,
    channel: String,
    chat_id: String,
    account_id: String,
    enabled: bool,
    last_run: Option<DateTime<Utc>>,
    next_run: Option<DateTime<Utc>>,
}

impl From<cleanclaw_store::models::CronJobRecord> for CronJobDto {
    fn from(r: cleanclaw_store::models::CronJobRecord) -> Self {
        Self {
            id: r.id,
            user_id: r.user_id,
            agent_id: r.agent_id,
            name: r.name,
            r#type: r.r#type,
            schedule: r.schedule,
            message: r.message,
            channel: r.channel,
            chat_id: r.chat_id,
            account_id: r.account_id,
            enabled: r.enabled,
            last_run: r.last_run,
            next_run: r.next_run,
        }
    }
}

async fn list_all(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    // The store has no `list_all_cron_jobs` (per-user only). For the
    // parity sweep we walk the session-owner pairs as a proxy. A
    // real impl will read from a dedicated `cron_jobs` view or add
    // a `list_cron_jobs` to the store trait.
    let pairs = match state.store.list_session_owner_pairs().await {
        Ok(p) => p,
        Err(e) => return internal(e).into_response(),
    };
    let mut out = Vec::new();
    for p in pairs {
        if let Ok(jobs) = state.store.list_cron_jobs_by_agent(&p.agent_id).await {
            for j in jobs {
                out.push(CronJobDto::from(j));
            }
        }
    }
    (StatusCode::OK, Json(out)).into_response()
}

#[derive(Deserialize)]
struct CreateReq {
    agent_id: String,
    name: String,
    r#type: String,
    schedule: String,
    message: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    account_id: String,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

async fn create(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateReq>,
) -> impl IntoResponse {
    if req.agent_id.is_empty() {
        return bad("agent_id required");
    }
    if req.name.is_empty() {
        return bad("name required");
    }
    // The "owner" is whoever's running the gateway; the dashboard
    // can override via the auth header. For the parity sweep we
    // take the first user — a real impl uses require_auth().
    let user_id = match state.store.list_users().await {
        Ok(u) => u.first().map(|u| u.id.clone()).unwrap_or_default(),
        Err(_) => String::new(),
    };
    let now = Utc::now();
    let job = cleanclaw_store::models::CronJobRecord {
        id: format!("cron_{}", uuid::Uuid::new_v4().simple()),
        user_id,
        agent_id: req.agent_id,
        name: req.name,
        r#type: req.r#type,
        schedule: req.schedule,
        message: req.message,
        channel: req.channel,
        chat_id: req.chat_id,
        account_id: req.account_id,
        timezone: "UTC".into(),
        enabled: req.enabled,
        last_run: None,
        next_run: None,
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    match state.store.save_cron_job(&job).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({"job": CronJobDto::from(job)})),
        )
            .into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn update_job(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateReq>,
) -> impl IntoResponse {
    let mut job = match state.store.get_cron_job(&id).await {
        Ok(j) => j,
        Err(e) => return internal(e).into_response(),
    };
    job.agent_id = req.agent_id;
    job.name = req.name;
    job.r#type = req.r#type;
    job.schedule = req.schedule;
    job.message = req.message;
    job.channel = req.channel;
    job.chat_id = req.chat_id;
    job.account_id = req.account_id;
    job.enabled = req.enabled;
    match state.store.save_cron_job(&job).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn delete_job(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.delete_cron_job(&id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn list_for_agent(
    State(state): State<Arc<ServerState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.store.list_cron_jobs_by_agent(&agent_id).await {
        Ok(jobs) => {
            let dtos: Vec<CronJobDto> = jobs.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(json!({"jobs": dtos}))).into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

async fn delete_for_agent(
    State(state): State<Arc<ServerState>>,
    Path((_agent_id, job_id)): Path<(String, String)>,
) -> impl IntoResponse {
    delete_job(State(state), Path(job_id)).await
}

#[derive(Deserialize)]
struct ToggleReq {
    enabled: bool,
}

async fn toggle_for_agent(
    State(state): State<Arc<ServerState>>,
    Path((_agent_id, job_id)): Path<(String, String)>,
    Json(req): Json<ToggleReq>,
) -> impl IntoResponse {
    let mut job = match state.store.get_cron_job(&job_id).await {
        Ok(j) => j,
        Err(e) => return internal(e).into_response(),
    };
    job.enabled = req.enabled;
    match state.store.save_cron_job(&job).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

fn bad(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}

fn internal(e: CleanClawError) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}
