//! Cron HTTP endpoints.

use super::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use cleanclaw_core::{CronJobId, Result};
use cleanclaw_cron::{compute_next_run, validate_cron, validate_once};
use cleanclaw_store::models::CronJobRecord;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize)]
pub struct CreateCronRequest {
    pub agent_id: String,
    pub name: String,
    /// "cron" | "interval" | "once"
    pub r#type: String,
    pub schedule: String,
    pub message: String,
    pub channel: String,
    pub chat_id: String,
    #[serde(default)]
    pub account_id: String,
}

pub async fn create_cron(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateCronRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };

    // Validate the schedule based on type
    let validation = match req.r#type.as_str() {
        "cron" => validate_cron(&req.schedule).map(|_| ()),
        "interval" => cleanclaw_cron::parse_duration(&req.schedule).map(|_| ()),
        "once" => validate_once(&req.schedule).map(|_| ()),
        other => Err(cleanclaw_core::CleanClawError::InvalidArgument(format!(
            "unknown cron type: {other}"
        ))),
    };
    if let Err(e) = validation {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response();
    }

    let now = cleanclaw_core::now_utc();
    let next_run = match req.r#type.as_str() {
        "once" => Some(validate_once(&req.schedule).unwrap()),
        _ => compute_next_run(
            &CronJobRecord {
                id: String::new(),
                user_id: ident.user_id.clone(),
                agent_id: req.agent_id.clone(),
                name: req.name.clone(),
                r#type: req.r#type.clone(),
                schedule: req.schedule.clone(),
                message: req.message.clone(),
                channel: req.channel.clone(),
                chat_id: req.chat_id.clone(),
                account_id: req.account_id.clone(),
                timezone: "UTC".into(),
                enabled: true,
                last_run: None,
                next_run: None,
                locked_by: None,
                locked_at: None,
                failure_count: 0,
                created_at: now,
            },
            now,
        )
        .ok(),
    };

    let job = CronJobRecord {
        id: CronJobId::generate().to_string(),
        user_id: ident.user_id.clone(),
        agent_id: req.agent_id,
        name: req.name,
        r#type: req.r#type,
        schedule: req.schedule,
        message: req.message,
        channel: req.channel,
        chat_id: req.chat_id,
        account_id: req.account_id,
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run,
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    match state.store.save_cron_job(&job).await {
        Ok(()) => (StatusCode::CREATED, Json(json!({"job": job}))).into_response(),
        Err(e) => super::err_to_response(e),
    }
}

pub async fn list_cron_for_agent(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    // Verify the caller can access this agent
    if let Ok(agent) = state.store.get_agent(&agent_id).await {
        if agent.user_id != ident.user_id && !ident.is_super_admin() {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"}))).into_response();
        }
    }
    match state.store.list_cron_jobs_by_agent(&agent_id).await {
        Ok(jobs) => (StatusCode::OK, Json(json!({"jobs": jobs}))).into_response(),
        Err(e) => super::err_to_response(e),
    }
}

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub enabled: bool,
}

pub async fn toggle_cron(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<ToggleRequest>,
) -> impl IntoResponse {
    let _ = super::require_auth(&state, &headers).await;
    match state.store.get_cron_job(&id).await {
        Ok(mut job) => {
            job.enabled = req.enabled;
            if req.enabled && job.next_run.is_none() {
                let now = cleanclaw_core::now_utc();
                job.next_run = compute_next_run(&job, now).ok();
            }
            if let Err(e) = state.store.save_cron_job(&job).await {
                return super::err_to_response(e);
            }
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }
        Err(e) => super::err_to_response(e),
    }
}

pub async fn delete_cron(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let _ = super::require_auth(&state, &headers).await;
    match state.store.delete_cron_job(&id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => super::err_to_response(e),
    }
}
