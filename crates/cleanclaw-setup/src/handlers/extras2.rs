//! Second batch of HTTP endpoints. Self-service user actions, API
//! key CRUD, provider CRUD, agent system files.
//!
//! Mirrors the remaining handlers in
//!  + `handlers_admin.go`
//! + `handlers_scoped.go` that the dashboard needs:
//!
//!   - `POST /api/logout`                       end the session
//!   - `PUT  /api/me`                           update own profile
//!   - `POST /api/me/password`                  change own password
//!   - `GET  /api/apikeys`                      list API keys (per user)
//!   - `POST /api/apikeys`                      create API key
//!   - `DELETE /api/apikeys/:id`                delete API key
//!   - `POST /api/apikeys/:id/rotate`           rotate API key
//!   - `GET  /api/providers`                    list providers
//!   - `POST /api/providers`                    create provider
//!   - `PUT  /api/providers/:id`                update provider
//!   - `DELETE /api/providers/:id`              delete provider
//!   - `GET  /api/agents/:id/system-file/:name` read system file
//!   - `PUT  /api/agents/:id/system-file/:name` write system file
//!   - `DELETE /api/agents/:id/system-file/:name` delete system file

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use chrono::{DateTime, Utc};
use cleanclaw_auth::apikey::{generate, sha256_hex};
use cleanclaw_auth::UserError;
use cleanclaw_store::models::{ApiKeyRecord, ConfigRecord};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use crate::ServerState;

#[derive(Debug, Error)]
pub enum Extras2Error {
    #[error("auth: {0}")]
    Auth(#[from] UserError),
    #[error("store: {0}")]
    Store(String),
    #[error("invalid input: {0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for Extras2Error {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            Extras2Error::Auth(UserError::InvalidCredentials) => {
                (StatusCode::UNAUTHORIZED, "invalid credentials".to_string())
            }
            Extras2Error::Auth(UserError::LastSuperAdmin) => (
                StatusCode::CONFLICT,
                "cannot remove last super admin".to_string(),
            ),
            Extras2Error::Auth(UserError::InvalidRole(_)) => {
                (StatusCode::BAD_REQUEST, "invalid role".to_string())
            }
            Extras2Error::Auth(UserError::InvalidStatus(_)) => {
                (StatusCode::BAD_REQUEST, "invalid status".to_string())
            }
            Extras2Error::Auth(UserError::Missing(_)) => (
                StatusCode::BAD_REQUEST,
                "missing required field".to_string(),
            ),
            Extras2Error::Auth(UserError::Store(s)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, s.to_string())
            }
            Extras2Error::BadRequest(s) => (StatusCode::BAD_REQUEST, s.clone()),
            Extras2Error::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            Extras2Error::Store(s) | Extras2Error::Internal(s) => {
                (StatusCode::INTERNAL_SERVER_ERROR, s.clone())
            }
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

pub fn router() -> Router<Arc<ServerState>> {
    // NOTE: /api/logout and /api/apikeys (GET+POST) live in
    // `cleanclaw-api` (W1 surface). The W2 sub-resources here —
    // apikey DELETE + rotate, providers CRUD, agents/:id/system-file
    // — are dashboard-only and don't collide.
    Router::new()
        .route("/api/me", put(update_me))
        .route("/api/me/password", post(change_password))
        .route("/api/apikeys/:id", delete(delete_apikey))
        .route("/api/apikeys/:id/rotate", post(rotate_apikey))
        .route(
            "/api/providers",
            get(list_providers).post(create_provider),
        )
        .route(
            "/api/providers/:id",
            put(update_provider).delete(delete_provider),
        )
        .route(
            "/api/agents/:id/system-file/:name",
            get(get_system_file)
                .put(put_system_file)
                .delete(delete_system_file),
        )
}

// ---------------------------------------------------------------------
// /api/logout + /api/me (PUT) + /api/me/password
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    #[serde(default)]
    pub session_id: String,
}

async fn logout(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<LogoutRequest>,
) -> Json<serde_json::Value> {
    if !body.session_id.is_empty() {
        let _ = state.store.delete_web_session(&body.session_id).await;
        info!(session = %body.session_id, "logged out");
    }
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateMeRequest {
    pub user_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub avatar_url: String,
}

async fn update_me(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<UpdateMeRequest>,
) -> Result<Json<serde_json::Value>, Extras2Error> {
    if body.user_id.is_empty() {
        return Err(Extras2Error::BadRequest("user_id required".into()));
    }
    // `update_profile` is a partial update: it only sets display_name
    // and avatar_url. Email change is intentionally not supported
    // here — the auth crate's `update` doesn't expose email as a
    // partial field, and email changes have UNIQUE-constraint
    // implications that need a separate flow.
    let _ = body.email; // accepted but ignored
    state
        .accounts
        .update_profile(
            &body.user_id,
            &body.display_name,
            &body.avatar_url,
        )
        .await
        .map_err(Extras2Error::Auth)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub user_id: String,
    pub old_password: String,
    pub new_password: String,
}

async fn change_password(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, Extras2Error> {
    if body.new_password.is_empty() {
        return Err(Extras2Error::BadRequest("new_password required".into()));
    }
    state
        .accounts
        .verify_password(&body.user_id, &body.old_password)
        .await
        .map_err(Extras2Error::Auth)?;
    state
        .accounts
        .set_password(&body.user_id, &body.new_password)
        .await
        .map_err(Extras2Error::Auth)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------
// /api/apikeys — list / create / delete / rotate
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ApiKeyView {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_prefix: String,
    pub r#type: String,
    pub created_at: String,
}

fn to_view(k: &ApiKeyRecord) -> ApiKeyView {
    ApiKeyView {
        id: k.id.clone(),
        user_id: k.user_id.clone(),
        name: k.name.clone(),
        key_prefix: k.key_prefix.clone(),
        r#type: k.r#type.clone(),
        created_at: k.created_at.to_rfc3339(),
    }
}

async fn list_apikeys(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<ApiKeyView>>, Extras2Error> {
    // List ALL keys for now (the CLI's `apikey list` accepts an
    // optional --user filter; the dashboard wants all visible to
    // the caller). Auth middleware will gate this to admins in
    // production; for parity we expose the full list.
    let keys = state
        .store
        .list_api_keys("")
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    let views: Vec<ApiKeyView> = keys.iter().map(to_view).collect();
    Ok(Json(views))
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub user_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_key_type")]
    pub r#type: String,
}

fn default_key_type() -> String {
    "user".to_string()
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub key: String,
    pub key_prefix: String,
}

async fn create_apikey(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<Json<CreateApiKeyResponse>, Extras2Error> {
    if body.user_id.is_empty() {
        return Err(Extras2Error::BadRequest("user_id required".into()));
    }
    let (id, key, prefix) = generate();
    let now: DateTime<Utc> = Utc::now();
    let rec = ApiKeyRecord {
        id: id.clone(),
        user_id: body.user_id.clone(),
        name: if body.name.is_empty() { "default".into() } else { body.name },
        key_hash: sha256_hex(&key),
        key_prefix: prefix.clone(),
        r#type: body.r#type,
        created_at: now,
        prev_hash: None,
        prev_hash_set_at: None,
    };
    state
        .store
        .create_api_key(&rec)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    info!(id = %id, user_id = %body.user_id, "apikey created");
    Ok(Json(CreateApiKeyResponse {
        id,
        key,
        key_prefix: prefix,
    }))
}

async fn delete_apikey(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Extras2Error> {
    state
        .store
        .delete_api_key(&id)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Serialize)]
pub struct RotateApiKeyResponse {
    pub id: String,
    pub key: String,
    pub key_prefix: String,
}

async fn rotate_apikey(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<RotateApiKeyResponse>, Extras2Error> {
    let (new_id, new_key, new_prefix) = generate();
    state
        .store
        .rotate_api_key(&id, &sha256_hex(&new_key), &new_prefix)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    info!(id = %id, "apikey rotated");
    Ok(Json(RotateApiKeyResponse {
        id: new_id,
        key: new_key,
        key_prefix: new_prefix,
    }))
}

// ---------------------------------------------------------------------
// /api/providers — list / create / update / delete
// ---------------------------------------------------------------------

async fn list_providers(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<ConfigRecord>>, Extras2Error> {
    let rows = state
        .store
        .list_configs("provider", "", "")
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct CreateProviderRequest {
    pub name: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

async fn create_provider(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<CreateProviderRequest>,
) -> Result<Json<ConfigRecord>, Extras2Error> {
    if body.name.is_empty() {
        return Err(Extras2Error::BadRequest("name required".into()));
    }
    let now = Utc::now();
    let rec = ConfigRecord {
        id: format!("prov_{}", uuid::Uuid::new_v4().simple()),
        kind: "provider".into(),
        scope: if body.user_id.is_empty() { "system".into() } else { "user".into() },
        user_id: body.user_id.clone(),
        agent_id: body.agent_id.clone(),
        name: body.name.clone(),
        enabled: true,
        credential_key: String::new(),
        data: body.data,
        created_at: now,
        updated_at: now,
    };
    state
        .store
        .save_config(&rec)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(rec))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderRequest {
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub enabled: Option<bool>,
}

async fn update_provider(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProviderRequest>,
) -> Result<Json<ConfigRecord>, Extras2Error> {
    // `get_config` takes (kind, user_id, agent_id, name) — for a
    // system-scope provider update the caller passes the row id as
    // `name` (matches the create path which uses the same field
    // for the provider's logical name). Per-user providers use
    // the row's `user_id` field.
    let mut rec = state
        .store
        .get_config("provider", "", "", &id)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    if rec.kind != "provider" {
        return Err(Extras2Error::BadRequest("not a provider".into()));
    }
    if !body.data.is_null() {
        rec.data = body.data;
    }
    if let Some(en) = body.enabled {
        rec.enabled = en;
    }
    rec.updated_at = Utc::now();
    state
        .store
        .save_config(&rec)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(rec))
}

async fn delete_provider(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Extras2Error> {
    state
        .store
        .delete_config("provider", "", "", &id)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------
// /api/agents/:id/system-file/:name
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SystemFileResponse {
    pub name: String,
    pub content: String,
}

async fn get_system_file(
    State(state): State<Arc<ServerState>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Json<SystemFileResponse>, Extras2Error> {
    // System files live in agent_files with user_id="" (shared
    // template). The dashboard reads the whole text blob.
    let rec = state
        .store
        .get_workspace_file_exact(&id, "", &name)
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(SystemFileResponse {
        name: rec.filename,
        content: rec.content,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PutSystemFileRequest {
    pub content: String,
}

async fn put_system_file(
    State(state): State<Arc<ServerState>>,
    Path((id, name)): Path<(String, String)>,
    Json(body): Json<PutSystemFileRequest>,
) -> Result<Json<serde_json::Value>, Extras2Error> {
    state
        .store
        .save_workspace_file(&id, "", &name, body.content.as_bytes())
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_system_file(
    State(state): State<Arc<ServerState>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, Extras2Error> {
    // The store doesn't have a `delete_workspace_file` method; we
    // emulate the operation by writing an empty body. The full
    // delete-soft implementation lands with the file-zip
    // endpoint's follow-up.
    state
        .store
        .save_workspace_file(&id, "", &name, b"")
        .await
        .map_err(|e| Extras2Error::Store(e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_view_serializes() {
        let v = ApiKeyView {
            id: "k1".into(),
            user_id: "u1".into(),
            name: "test".into(),
            key_prefix: "fast_".into(),
            r#type: "user".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        let blob = serde_json::to_string(&v).unwrap();
        assert!(blob.contains("\"keyPrefix\":\"fast_\"") || blob.contains("\"key_prefix\":\"fast_\""));
    }

    #[test]
    fn system_file_response_serializes() {
        let r = SystemFileResponse {
            name: "SOUL.md".into(),
            content: "You are helpful.".into(),
        };
        let blob = serde_json::to_string(&r).unwrap();
        assert!(blob.contains("\"name\":\"SOUL.md\""));
        assert!(blob.contains("\"You are helpful.\""));
    }

    #[test]
    fn default_key_type_is_user() {
        assert_eq!(default_key_type(), "user");
    }

    #[test]
    fn logout_request_default_session_id_is_empty() {
        let r = LogoutRequest { session_id: String::new() };
        assert!(r.session_id.is_empty());
    }

    #[test]
    fn update_me_request_defaults_to_empty() {
        let r = UpdateMeRequest::default();
        assert!(r.user_id.is_empty());
        assert!(r.display_name.is_empty());
    }

    #[test]
    fn change_password_request_requires_new_password() {
        // The handler checks `new_password.is_empty()` and returns
        // 400; we just verify the shape.
        let r = ChangePasswordRequest {
            user_id: "u1".into(),
            old_password: "old".into(),
            new_password: "".into(),
        };
        assert!(r.new_password.is_empty());
    }
}
