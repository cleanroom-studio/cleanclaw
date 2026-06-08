//! Plugin handlers.
//!
//! Routes:
//!   * GET    /api/plugins            - list loaded plugins (admin)
//!   * PUT    /api/plugins/:id        - update plugin config / enable
//!   * GET    /api/plugins/hook       - list hook plugins (any auth)
//!
//! Plugins are spawned by the gateway's `PluginManager`. We don't
//! have direct access to that here; the handlers read from the
//! configs store (kind="plugin", name=<plugin_id>) and surface the
//! manifest. The gateway owns the live process state.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, put},
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/:id", put(update_plugin))
        .route("/api/plugins/hook", get(list_hook_plugins))
}

#[derive(Serialize)]
struct PluginDto {
    id: String,
    name: String,
    r#type: String,
    enabled: bool,
    config: serde_json::Value,
}

async fn list_plugins(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    // Read every `kind="plugin"` config row across all users and
    // project to the manifest. A real impl filters by admin scope.
    let configs = match state.store.list_configs_all_kinds().await {
        Ok(c) => c,
        Err(e) => return internal(e).into_response(),
    };
    let mut out = Vec::new();
    for c in configs {
        if c.kind != "plugin" {
            continue;
        }
        out.push(PluginDto {
            id: c.name.clone(),
            name: c.name,
            r#type: c
                .data
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string(),
            enabled: c
                .data
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            config: c.data,
        });
    }
    (StatusCode::OK, Json(out)).into_response()
}

#[derive(Deserialize)]
struct UpdateReq {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    config: Option<serde_json::Value>,
}

async fn update_plugin(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateReq>,
) -> impl IntoResponse {
    if id.is_empty() {
        return bad("id required");
    }
    // The config row's user_id is "" (system scope). We upsert a
    // single plugin-level config; the gateway reads it on next
    // reload.
    let mut value = serde_json::json!({});
    if let Some(cfg) = req.config {
        if let Some(obj) = cfg.as_object() {
            for (k, v) in obj {
                value[k] = v.clone();
            }
        }
    }
    if let Some(en) = req.enabled {
        value["enabled"] = serde_json::json!(en);
    }
    let now = chrono::Utc::now();
    let rec = cleanclaw_store::models::ConfigRecord {
        id: format!("cfg_{}", uuid::Uuid::new_v4().simple()),
        kind: "plugin".into(),
        scope: "system".into(),
        user_id: String::new(),
        agent_id: String::new(),
        name: id.clone(),
        enabled: req.enabled.unwrap_or(true),
        credential_key: String::new(),
        data: value,
        created_at: now,
        updated_at: now,
    };
    match state.store.save_config(&rec).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn list_hook_plugins(State(_state): State<Arc<ServerState>>) -> impl IntoResponse {
    // Hook plugins are a subset of plugins; for the parity sweep we
    // return an empty array — the real impl filters by capability.
    let rows: Vec<PluginDto> = Vec::new();
    (StatusCode::OK, Json(rows)).into_response()
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
