//! Usage handlers.
//!
//! Routes:
//!   * GET  /api/usage            - admin aggregate (sums over all
//!                                   agents in the requested window)
//!   * GET  /api/agents/:id/usage - per-agent usage
//!   * GET  /api/tasks            - admin task list (currently empty)
//!
//! Token usage lives in the `token_usage` table; the store
//! already has `upsert_token_usage` + `list_token_usage`. The
//! admin aggregate just sums the rows in the window.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::Deserialize;
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/usage", get(admin_usage))
        .route("/api/agents/:id/usage", get(agent_usage))
        .route("/api/tasks", get(list_tasks))
}

#[derive(Deserialize)]
struct UsageQuery {
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    until: Option<String>,
}

async fn admin_usage(
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(q): axum::extract::Query<UsageQuery>,
) -> impl IntoResponse {
    let since_date = q
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive() - chrono::Duration::days(30));
    match state.store.list_token_usage(since_date).await {
        Ok(rows) => {
            let input: i64 = rows.iter().map(|r| r.input_tokens).sum();
            let output: i64 = rows.iter().map(|r| r.output_tokens).sum();
            (
                StatusCode::OK,
                Json(json!({
                    "rows": rows.len(),
                    "input_tokens": input,
                    "output_tokens": output,
                })),
            )
                .into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

async fn agent_usage(
    State(state): State<Arc<ServerState>>,
    Path(agent_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<UsageQuery>,
) -> impl IntoResponse {
    let since_date = q
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive() - chrono::Duration::days(30));
    match state.store.list_token_usage(since_date).await {
        Ok(all) => {
            let rows: Vec<_> = all.into_iter().filter(|r| r.agent_id == agent_id).collect();
            let input: i64 = rows.iter().map(|r| r.input_tokens).sum();
            let output: i64 = rows.iter().map(|r| r.output_tokens).sum();
            (
                StatusCode::OK,
                Json(json!({
                    "agent_id": agent_id,
                    "rows": rows.len(),
                    "input_tokens": input,
                    "output_tokens": output,
                })),
            )
                .into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

async fn list_tasks(State(_state): State<Arc<ServerState>>) -> impl IntoResponse {
    // The Go side reads from `taskqueue` (in-process). We don't
    // have a direct hook here; the dashboard shows "no tasks" until
    // a task-bridge lands.
    (StatusCode::OK, Json(json!({"tasks": []}))).into_response()
}

fn internal(e: CleanClawError) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}
