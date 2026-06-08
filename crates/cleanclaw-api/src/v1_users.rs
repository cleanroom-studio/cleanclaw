//! `POST /v1/users` — provision (or fetch) an app_user for the calling
//! api_key.
//!
//! Authenticated by api_key only. The endpoint is idempotent: repeated
//! calls with the same `(apikey_id, external_id)` pair return the same
//! CleanClaw user_id; the `created` field tells the caller whether a
//! new row was inserted or whether the existing one was reused.
//!
//! Request body: `{ "external_id": "…", "display_name": "…" (optional) }`
//! Response:     `{ "user_id": "u_…", "external_id": "…", "created": bool }`

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use cleanclaw_auth::{users::Accounts, AuthMethod, Identity};
use cleanclaw_core::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::ApiState;

#[derive(Debug, Clone, Deserialize)]
pub struct ProvisionRequest {
    pub external_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvisionResponse {
    pub user_id: String,
    pub external_id: String,
    pub created: bool,
}

pub async fn provision_user(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ProvisionRequest>,
) -> impl IntoResponse {
    // The caller must be an apikey caller; sessions get 401 here.
    let ident = match require_apikey(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let external_id = req.external_id.trim().to_string();
    if external_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "external_id is required",
                    "type": "invalid_request_error",
                }
            })),
        )
            .into_response();
    }
    let accounts = match Accounts::new(state.store.clone()) {
        Ok(a) => a,
        Err(e) => return user_err_to_response(e),
    };
    let display = req.display_name.clone().unwrap_or_default();
    let created = match accounts
        .ensure_app_user(&ident.api_key_id, &external_id, &display)
        .await
    {
        Ok(acc) => acc,
        Err(e) => return user_err_to_response(e),
    };
    let response = ProvisionResponse {
        user_id: created.id,
        external_id,
        created: true,
    };
    (StatusCode::OK, Json(response)).into_response()
}

fn user_err_to_response(e: cleanclaw_auth::users::UserError) -> axum::response::Response {
    use cleanclaw_auth::users::UserError;
    let (status, msg) = match &e {
        UserError::Missing(field) => (StatusCode::BAD_REQUEST, format!("missing: {field}")),
        UserError::InvalidRole(r) => (StatusCode::BAD_REQUEST, format!("invalid role: {r}")),
        UserError::InvalidStatus(s) => (StatusCode::BAD_REQUEST, format!("invalid status: {s}")),
        UserError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid credentials".into()),
        UserError::LastSuperAdmin => (StatusCode::CONFLICT, "last super admin".into()),
        UserError::Store(err) => {
            let s = err.http_status();
            return (
                StatusCode::from_u16(s).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(json!({ "error": { "message": err.to_string() } })),
            )
                .into_response();
        }
    };
    (status, Json(json!({ "error": { "message": msg } }))).into_response()
}

fn err_to_response(e: cleanclaw_core::CleanClawError) -> axum::response::Response {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        Json(json!({ "error": { "message": e.to_string() } })),
    )
        .into_response()
}

async fn require_apikey(
    state: &ApiState,
    headers: &axum::http::HeaderMap,
) -> std::result::Result<Identity, axum::response::Response> {
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let ident = state
        .auth
        .resolve(bearer.as_deref(), None)
        .await
        .map_err(err_to_response)?;
    match ident {
        Some(i) if i.method == AuthMethod::ApiKey && !i.api_key_id.is_empty() => Ok(i),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "api_key required",
                    "type": "authentication_error",
                }
            })),
        )
            .into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provision_request_deserialize() {
        let raw = json!({ "external_id": "u_42", "display_name": "Alice" });
        let r: ProvisionRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(r.external_id, "u_42");
        assert_eq!(r.display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn provision_request_optional_display_name() {
        let raw = json!({ "external_id": "u_42" });
        let r: ProvisionRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(r.external_id, "u_42");
        assert!(r.display_name.is_none());
    }

    #[test]
    fn provision_response_serialize() {
        let r = ProvisionResponse {
            user_id: "u_abc".into(),
            external_id: "ext_1".into(),
            created: true,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"user_id\":\"u_abc\""));
        assert!(s.contains("\"external_id\":\"ext_1\""));
        assert!(s.contains("\"created\":true"));
    }
}
