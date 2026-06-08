//! API key HTTP endpoints.
//! `HandleCreateAPIKey` / `HandleListAPIKeys`.

use super::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use cleanclaw_auth::apikey;
use cleanclaw_core::{ApiKeyId, Result};
use cleanclaw_store::models::ApiKeyRecord;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct CreateApiKeyRequest {
    pub r#type: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn list_api_keys(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let _ = super::require_auth(&state, &headers).await;
    // The Store layer doesn't yet expose a "list all keys" call; for
    // now, we only return the caller's own keys. The CLI can still
    // reach every key via `cleanclaw apikey ls`.
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    match state.store.list_api_keys(&ident.user_id).await {
        Ok(keys) => {
            let projected: Vec<serde_json::Value> = keys
                .into_iter()
                .map(|k| {
                    json!({
                        "id": k.id,
                        "type": k.r#type,
                        "key_prefix": k.key_prefix,
                        "name": k.name,
                    })
                })
                .collect();
            (StatusCode::OK, Json(json!({"keys": projected}))).into_response()
        }
        Err(e) => super::err_to_response(e),
    }
}

pub async fn create_api_key(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let (token, hash, prefix) = apikey::generate();
    let rec = ApiKeyRecord {
        id: ApiKeyId::generate().to_string(),
        user_id: ident.user_id.clone(),
        name: req.name.unwrap_or_else(|| "default".into()),
        key_hash: hash,
        key_prefix: prefix,
        r#type: req.r#type,
        created_at: cleanclaw_core::now_utc(),
        prev_hash: None,
        prev_hash_set_at: None,
    };
    if let Err(e) = state.store.create_api_key(&rec).await {
        return super::err_to_response(e);
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "id": rec.id,
            "type": rec.r#type,
            "key_prefix": rec.key_prefix,
            "name": rec.name,
            "token": token,
        })),
    )
        .into_response()
}

// ---- agent files ----------------------------------------------------------

pub async fn list_agent_files(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let _ = super::require_auth(&state, &headers).await;
    match state.store.list_workspace_files(&agent_id).await {
        Ok(files) => (StatusCode::OK, Json(json!({"files": files}))).into_response(),
        Err(e) => super::err_to_response(e),
    }
}

pub async fn get_agent_file(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path((agent_id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let _ = super::require_auth(&state, &headers).await;
    match state
        .store
        .get_workspace_file(&agent_id, "", &filename)
        .await
    {
        Ok((_user, bytes)) => {
            let content = String::from_utf8_lossy(&bytes).to_string();
            (
                StatusCode::OK,
                Json(json!({"filename": filename, "content": content})),
            )
                .into_response()
        }
        Err(e) => super::err_to_response(e),
    }
}
