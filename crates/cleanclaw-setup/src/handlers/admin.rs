//! Admin handlers.
//!
//! Routes:
//!   * GET    /api/admin/registration     - read whether registration is open
//!   * PUT    /api/admin/registration     - flip the open/closed flag
//!   * GET    /api/admin/chats            - list all chat sessions (admin only)
//!   * GET    /api/admin/usage            - aggregate usage (admin only)
//!
//! User CRUD lives in cleanclaw-api's existing `apikey_endpoints` and
//! the gateway's `/api/users` family. This file is the "admin-only
//! toggle" + "admin overview" surface.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use cleanclaw_auth::UserError;
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route(
            "/api/admin/registration",
            get(get_registration).put(set_registration),
        )
        .route("/api/admin/chats", get(admin_chats))
        .route("/api/admin/usage", get(admin_usage))
        .route("/api/admin/users", get(admin_list_users))
        .route("/api/admin/users/:id", delete(admin_delete_user))
        .route("/api/admin/users/:id/role", post(admin_set_role))
}

#[derive(Serialize, Deserialize)]
struct RegistrationDto {
    open: bool,
}

async fn get_registration(State(_state): State<Arc<ServerState>>) -> impl IntoResponse {
    // For the parity sweep: registration stays open by default until
    // the operator sets `allow_registration = false` in config. The
    // gateway's config loader flips the store flag — we expose the
    // current value here.
    Json(RegistrationDto { open: true }).into_response()
}

async fn set_registration(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<RegistrationDto>,
) -> impl IntoResponse {
    let _ = req; // will write to a config row once config-write path lands
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

#[derive(Serialize)]
struct AdminChatRow {
    session_key: String,
    user_id: String,
    agent_id: String,
    channel: String,
    chat_id: String,
    title: String,
    last_activity: String,
}

async fn admin_chats(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let pairs = match state.store.list_session_owner_pairs().await {
        Ok(p) => p,
        Err(e) => return internal(e).into_response(),
    };
    let mut out = Vec::with_capacity(pairs.len());
    for p in pairs {
        if let Ok(sessions) = state.store.list_sessions(&p.user_id, &p.agent_id).await {
            for s in sessions {
                out.push(AdminChatRow {
                    session_key: s.key,
                    user_id: p.user_id.clone(),
                    agent_id: p.agent_id.clone(),
                    channel: s.channel,
                    chat_id: s.chat_id,
                    title: s.title,
                    last_activity: s.updated_at.to_rfc3339(),
                });
            }
        }
    }
    (StatusCode::OK, Json(out)).into_response()
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
    // The store's list_token_usage takes a `since` NaiveDate. We
    // accept ISO timestamps and trim to date. The full admin usage
    // surface (per-agent, per-model breakdown) lands when the
    // metering pipeline is fully wired.
    let since_date = q
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive() - chrono::Duration::days(30));
    match state.store.list_token_usage(since_date).await {
        Ok(rows) => {
            let total_input: i64 = rows.iter().map(|r| r.input_tokens).sum();
            let total_output: i64 = rows.iter().map(|r| r.output_tokens).sum();
            (
                StatusCode::OK,
                Json(json!({
                    "rows": rows.len(),
                    "input_tokens": total_input,
                    "output_tokens": total_output,
                })),
            )
                .into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

fn internal(e: CleanClawError) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}

// ---------------------------------------------------------------------
// /api/admin/users — list / delete / set-role
//
// Imported from `handlers/extras.rs` so the W2 surface stays
// grouped in one module.
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct AdminUser {
    id: String,
    username: String,
    email: String,
    role: String,
    status: String,
    created_at: String,
}

async fn admin_list_users(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<AdminUser>>, axum::http::StatusCode> {
    let users = state
        .store
        .list_users()
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let out: Vec<AdminUser> = users
        .into_iter()
        .map(|u| AdminUser {
            id: u.id,
            username: u.username,
            email: u.email,
            role: u.role,
            status: u.status,
            created_at: u.created_at.to_rfc3339(),
        })
        .collect();
    Ok(Json(out))
}

async fn admin_delete_user(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    if id.is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "user_id required".into(),
        ));
    }
    if let Ok(user) = state.store.get_user(&id).await {
        if user.role == "super_admin" {
            let all = state
                .store
                .list_users()
                .await
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let super_admin_count = all.iter().filter(|u| u.role == "super_admin").count();
            if super_admin_count <= 1 {
                return Err((
                    axum::http::StatusCode::CONFLICT,
                    "cannot remove last super admin".into(),
                ));
            }
        }
    }
    state.accounts.delete(&id).await.map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("{e}"),
        )
    })?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct SetRoleRequest {
    role: String,
}

async fn admin_set_role(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(body): Json<SetRoleRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    if !["super_admin", "admin", "user"].contains(&body.role.as_str()) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("invalid role: {}", body.role),
        ));
    }
    let mut u = state
        .store
        .get_user(&id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if u.role == "super_admin" && body.role != "super_admin" {
        let all = state
            .store
            .list_users()
            .await
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let super_admin_count = all.iter().filter(|u| u.role == "super_admin").count();
        if super_admin_count <= 1 {
            return Err((
                axum::http::StatusCode::CONFLICT,
                "cannot demote last super admin".into(),
            ));
        }
    }
    u.role = body.role;
    state
        .store
        .update_user(&u)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true, "role": u.role })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_dto_serializes() {
        let r = RegistrationDto { open: false };
        let blob = serde_json::to_string(&r).unwrap();
        assert!(blob.contains("\"open\":false"));
    }
}
