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
use serde_json::Value;
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

/// Whitelist of providers we know how to talk to. Any other name
/// the dashboard sends is dropped on the floor so a typo can't
/// silently re-route searches to a dead upstream.
const WEB_SEARCH_PROVIDERS: &[&str] = &[
    "duckduckgo",
    "brave",
    "bing",
    "google",
    "baidu",
    "searxng",
    "exa",
    "none",
];

async fn write_tools(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let value = body
        .get("tools")
        .cloned()
        .unwrap_or_else(|| body.clone());
    // Sanitize web_search.provider against the whitelist so a
    // typo or an untrusted admin can't route searches to a
    // provider we never registered.
    let mut value = value;
    if let Some(ws) = value.get_mut("web_search").and_then(|v| v.as_object_mut()) {
        if let Some(p) = ws.get("provider").and_then(|v| v.as_str()) {
            if !WEB_SEARCH_PROVIDERS.contains(&p) {
                ws.insert(
                    "provider".to_string(),
                    Value::String("duckduckgo".to_string()),
                );
            }
        }
        // Also ensure the category object has the standard shape
        // (enabled + provider + optional api_key + endpoint) so
        // downstream code can rely on it.
        if !ws.contains_key("enabled") {
            ws.insert("enabled".to_string(), Value::Bool(true));
        }
    }
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
        // DuckDuckGo is the default primary because it ships
        // credential-free and works out-of-the-box without an
        // API key — operators who want better relevance add a
        // key-bearing provider (brave / bing / google) which
        // the chain will pick up via the tools.chain() list.
        "web_search": { "enabled": true,  "provider": "duckduckgo" },
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
