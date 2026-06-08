//! Scoped-config handlers. Mirrors
//! .
//!
//! Routes:
//!   * GET  /api/config          - read the merged effective config
//!   * POST /api/config          - write a config value at (scope, scope_id, name)
//!   * DELETE /api/config?scope=...&scope_id=...&name=... - delete a row
//!
//! The dashboard's "scoped config" tab is the one place where it
//! can write settings that override per-(user, agent) without
//! touching the global default. The Go side stores these as
//! `configs` rows; we follow the same schema.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::Deserialize;
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/config", get(read_config).post(write_config))
        .route("/api/config/delete", delete(delete_config))
}

#[derive(Deserialize)]
struct ReadQuery {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scope_id: Option<String>,
}

async fn read_config(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<ReadQuery>,
) -> impl IntoResponse {
    // The store has `list_configs(kind, user_id, agent_id)` plus
    // `list_configs_all_kinds`. For the parity sweep we read all
    // configs and project; the gateway's `internal/scope` package
    // is the one that does the proper (system → user → agent)
    // override walk. The dashboard's "config" tab only needs the
    // shape, not the resolution.
    let (user_id, agent_id) = scope_to_ids(&q.scope, &q.scope_id);
    match state.store.list_configs_all_kinds().await {
        Ok(rows) => {
            let filtered: Vec<_> = rows
                .into_iter()
                .filter(|r| r.user_id == user_id && r.agent_id == agent_id)
                .collect();
            let projected: Vec<_> = filtered
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "kind": r.kind,
                        "name": r.name,
                        "value": r.data,
                    })
                })
                .collect();
            (StatusCode::OK, Json(json!({"configs": projected}))).into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

#[derive(Deserialize)]
struct WriteReq {
    scope: String,
    scope_id: String,
    name: String,
    value: serde_json::Value,
}

async fn write_config(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<WriteReq>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return bad("name required");
    }
    let (user_id, agent_id) = scope_to_ids(&Some(req.scope.clone()), &Some(req.scope_id.clone()));
    let now = chrono::Utc::now();
    let rec = cleanclaw_store::models::ConfigRecord {
        id: format!("cfg_{}", uuid::Uuid::new_v4().simple()),
        kind: "setting".into(),
        scope: req.scope.clone(),
        user_id,
        agent_id,
        name: req.name,
        enabled: true,
        credential_key: String::new(),
        data: req.value,
        created_at: now,
        updated_at: now,
    };
    match state.store.save_config(&rec).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

#[derive(Deserialize)]
struct DeleteQuery {
    scope: Option<String>,
    scope_id: Option<String>,
    name: String,
    #[serde(default)]
    kind: Option<String>,
}

async fn delete_config(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<DeleteQuery>,
) -> impl IntoResponse {
    let (user_id, agent_id) = scope_to_ids(&q.scope, &q.scope_id);
    let kind = q.kind.unwrap_or_else(|| "setting".into());
    match state
        .store
        .delete_config(&kind, &user_id, &agent_id, &q.name)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

fn scope_to_ids(scope: &Option<String>, scope_id: &Option<String>) -> (String, String) {
    match scope.as_deref() {
        Some("user") => (scope_id.clone().unwrap_or_default(), String::new()),
        Some("agent") => (String::new(), scope_id.clone().unwrap_or_default()),
        _ => (String::new(), String::new()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::scoped;

    #[test]
    fn scope_to_ids_system() {
        let (u, a) = scoped::scope_to_ids(&None, &None);
        assert_eq!(u, "");
        assert_eq!(a, "");
    }

    #[test]
    fn scope_to_ids_user() {
        let (u, a) = scoped::scope_to_ids(&Some("user".into()), &Some("u1".into()));
        assert_eq!(u, "u1");
        assert_eq!(a, "");
    }

    #[test]
    fn scope_to_ids_agent() {
        let (u, a) = scoped::scope_to_ids(&Some("agent".into()), &Some("a1".into()));
        assert_eq!(u, "");
        assert_eq!(a, "a1");
    }
}
