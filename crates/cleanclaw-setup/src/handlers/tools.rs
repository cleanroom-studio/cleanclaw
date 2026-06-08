//! Tools handlers.
//!
//! Routes:
//!   * GET  /api/tools   - read the system-wide tool config (admin)
//!   * PUT  /api/tools   - write the system-wide tool config (admin)
//!
//! The dashboard's "tools" tab toggles per-tool on/off flags plus
//! provider keys for the gateway's built-in tools (web_search,
//! image_gen, tts, webfetch). The Go side stores the merged config
//! as a `configs` row with `kind="tools"`; we follow the same shape.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, put},
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/tools", get(read_tools).put(write_tools))
}

async fn read_tools(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    // The "tools" config is a system-scope row: user_id="",
    // agent_id="", kind="tools", name="config".
    let configs = match state.store.list_configs("tools", "", "").await {
        Ok(c) => c,
        Err(e) => return internal(e).into_response(),
    };
    let value = configs
        .into_iter()
        .next()
        .map(|c| c.data)
        .unwrap_or_else(default_tools_config);
    (StatusCode::OK, Json(json!({"tools": value}))).into_response()
}

async fn write_tools(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let value = body
        .get("tools")
        .cloned()
        .unwrap_or_else(|| body.clone());
    let now = chrono::Utc::now();
    let rec = cleanclaw_store::models::ConfigRecord {
        id: format!("cfg_{}", uuid::Uuid::new_v4().simple()),
        kind: "tools".into(),
        scope: "system".into(),
        user_id: String::new(),
        agent_id: String::new(),
        name: "config".into(),
        enabled: true,
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

fn default_tools_config() -> serde_json::Value {
    json!({
        "web_search": { "enabled": true,  "provider": "brave" },
        "image_gen":  { "enabled": false, "provider": "openai" },
        "tts":        { "enabled": false, "provider": "openai" },
        "webfetch":   { "enabled": true,  "provider": "direct" },
    })
}

fn internal(e: CleanClawError) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tools_config_has_all_categories() {
        let v = default_tools_config();
        for k in ["web_search", "image_gen", "tts", "webfetch"] {
            assert!(v.get(k).is_some(), "missing default {k}");
        }
    }
}
