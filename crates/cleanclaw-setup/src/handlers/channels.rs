//! Channel + scoped-channel handlers. Mirrors
//!  and the
//! `scoped-channels` family from `handlers_scoped.go`.
//!
//! Routes:
//!   * GET    /api/channels            - per-user channels (returns the
//!                                       static channel type list — the
//!                                       dashboard's resources tab
//!                                       doesn't filter by config yet)
//!   * GET    /api/scoped-channels     - per-(scope,scopeId) channel rows
//!   * POST   /api/scoped-channels     - create a scoped channel
//!   * PUT    /api/scoped-channels/:id
//!   * DELETE /api/scoped-channels/:id
//!
//! The Go side stores scoped-channels as `configs` rows with
//! `kind="channel"` and (scope, scopeID) = (user, <userID>). The
//! full write path lives in the gateway's config layer; the routes
//! here are stubs that surface the right JSON shape so the dashboard
//! can talk to them.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use cleanclaw_core::CleanClawError;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/channels", get(list_channels))
        .route("/api/scoped-channels", get(list_scoped).post(create_scoped))
        .route(
            "/api/scoped-channels/:id",
            put(update_scoped).delete(delete_scoped),
        )
}

#[derive(Serialize)]
struct ChannelRow {
    r#type: String,
    enabled: bool,
    status: String,
}

async fn list_channels(State(_state): State<Arc<ServerState>>) -> impl IntoResponse {
    let rows = vec![
        ChannelRow {
            r#type: "telegram".into(),
            enabled: false,
            status: "disconnected".into(),
        },
        ChannelRow {
            r#type: "discord".into(),
            enabled: false,
            status: "disconnected".into(),
        },
        ChannelRow {
            r#type: "slack".into(),
            enabled: false,
            status: "disconnected".into(),
        },
        ChannelRow {
            r#type: "feishu".into(),
            enabled: false,
            status: "disconnected".into(),
        },
        ChannelRow {
            r#type: "wechat".into(),
            enabled: false,
            status: "disconnected".into(),
        },
        ChannelRow {
            r#type: "line".into(),
            enabled: false,
            status: "disconnected".into(),
        },
        ChannelRow {
            r#type: "web".into(),
            enabled: true,
            status: "connected".into(),
        },
    ];
    Json(rows).into_response()
}

#[derive(Serialize)]
struct ScopedChannelRow {
    id: String,
    scope: String,
    scope_id: String,
    r#type: String,
    enabled: bool,
    config: serde_json::Value,
}

#[derive(Deserialize)]
struct ScopedQuery {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scope_id: Option<String>,
}

async fn list_scoped(
    State(_state): State<Arc<ServerState>>,
    axum::extract::Query(_q): axum::extract::Query<ScopedQuery>,
) -> impl IntoResponse {
    // The store has `list_configs(kind, user_id, agent_id)` which
    // is the canonical source. The gateway's config layer is the
    // one that translates the row to ScopedChannelRow. We return
    // an empty list here — the real impl reads from configs.
    let rows: Vec<ScopedChannelRow> = Vec::new();
    (StatusCode::OK, Json(rows)).into_response()
}

#[derive(Deserialize)]
struct CreateScopedReq {
    scope: String,
    scope_id: String,
    r#type: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    config: serde_json::Value,
}

async fn create_scoped(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<CreateScopedReq>,
) -> impl IntoResponse {
    if req.r#type.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "type required"})),
        )
            .into_response();
    }
    let id = format!("sc_{}", uuid::Uuid::new_v4().simple());
    (StatusCode::CREATED, Json(json!({"id": id, "ok": true}))).into_response()
}

async fn update_scoped(
    State(_state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(_req): Json<CreateScopedReq>,
) -> impl IntoResponse {
    if id.is_empty() {
        return bad("id required");
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

async fn delete_scoped(
    State(_state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if id.is_empty() {
        return bad("id required");
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

fn bad(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}

fn _silence_unused(_: CleanClawError) {}
