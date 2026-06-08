//! Agents CRUD handlers.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::models::AgentRecord;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Server, ServerState, SetupError};

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/agents", get(list_agents).post(create_agent))
        .route(
            "/api/agents/:id",
            get(get_agent).put(update_agent).delete(delete_agent),
        )
        .route("/api/agents/:id/files", get(list_agent_files))
        .route(
            "/api/agents/:id/files/:name",
            get(get_agent_file)
                .put(put_agent_file)
                .delete(delete_agent_file),
        )
}

#[derive(Debug, Serialize)]
pub struct AgentDto {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub is_public: bool,
    pub config: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<AgentRecord> for AgentDto {
    fn from(r: AgentRecord) -> Self {
        Self {
            id: r.id,
            user_id: r.user_id,
            name: r.name,
            is_public: r.is_public,
            config: r.config,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

async fn list_agents(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let uid = header_user(&headers);
    let res = if uid.is_empty() {
        state.store.list_all_agents().await
    } else {
        state.store.list_agents(&uid).await
    };
    match res {
        Ok(rows) => {
            let dtos: Vec<AgentDto> = rows.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(dtos)).into_response()
        }
        Err(e) => err_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentReq {
    pub name: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub is_public: bool,
}

async fn create_agent(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateAgentReq>,
) -> impl IntoResponse {
    let owner = if !req.user_id.is_empty() {
        req.user_id.clone()
    } else {
        header_user(&headers)
    };
    if owner.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "owner required"})),
        )
            .into_response();
    }
    let now = chrono::Utc::now();
    let id = format!("agent_{}", uuid::Uuid::new_v4().simple());
    let rec = AgentRecord {
        id,
        user_id: owner,
        name: req.name,
        config: if req.config.is_null() {
            serde_json::json!({})
        } else {
            req.config
        },
        is_public: req.is_public,
        created_at: now,
        updated_at: now,
    };
    match state.store.save_agent(&rec).await {
        Ok(()) => (StatusCode::CREATED, Json(AgentDto::from(rec))).into_response(),
        Err(e) => err_response(e),
    }
}

async fn get_agent(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_agent(&id).await {
        Ok(rec) => (StatusCode::OK, Json(AgentDto::from(rec))).into_response(),
        Err(e) => err_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub is_public: Option<bool>,
}

async fn update_agent(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentReq>,
) -> impl IntoResponse {
    let mut rec = match state.store.get_agent(&id).await {
        Ok(r) => r,
        Err(e) => return err_response(e),
    };
    if let Some(n) = req.name {
        rec.name = n;
    }
    if let Some(c) = req.config {
        rec.config = c;
    }
    if let Some(p) = req.is_public {
        rec.is_public = p;
    }
    rec.updated_at = chrono::Utc::now();
    match state.store.save_agent(&rec).await {
        Ok(()) => (StatusCode::OK, Json(AgentDto::from(rec))).into_response(),
        Err(e) => err_response(e),
    }
}

async fn delete_agent(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.delete_agent(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_response(e),
    }
}

#[derive(Debug, Serialize)]
pub struct AgentFileDto {
    pub agent_id: String,
    pub user_id: String,
    pub filename: String,
    pub content: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

async fn list_agent_files(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let names = match state.store.list_workspace_files(&id).await {
        Ok(r) => r,
        Err(e) => return err_response(e),
    };
    let dtos: Vec<AgentFileDto> = names
        .into_iter()
        .map(|n| AgentFileDto {
            agent_id: id.clone(),
            user_id: String::new(),
            filename: n,
            content: String::new(),
            updated_at: chrono::Utc::now(),
        })
        .collect();
    (StatusCode::OK, Json(dtos)).into_response()
}

async fn get_agent_file(
    State(state): State<Arc<ServerState>>,
    Path((id, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user_id = String::new(); // user_id="" → shared file
    match state.store.get_workspace_file(&id, &user_id, &name).await {
        Ok((content, _)) => (
            StatusCode::OK,
            Json(AgentFileDto {
                agent_id: id,
                user_id,
                filename: name,
                content: String::from_utf8_lossy(content.as_bytes()).to_string(),
                updated_at: chrono::Utc::now(),
            }),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PutFileReq {
    pub content: String,
}

async fn put_agent_file(
    State(state): State<Arc<ServerState>>,
    Path((id, name)): Path<(String, String)>,
    Json(req): Json<PutFileReq>,
) -> impl IntoResponse {
    match state
        .store
        .save_workspace_file(&id, "", &name, req.content.as_bytes())
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "agent_id": id,
                "user_id": "",
                "filename": name,
                "content": req.content,
                "updated_at": chrono::Utc::now(),
            })),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

async fn delete_agent_file(
    State(_state): State<Arc<ServerState>>,
    Path((_id, _name)): Path<(String, String)>,
) -> impl IntoResponse {
    // The Store trait doesn't expose a delete_workspace_file method
    // (file deletion goes through a separate admin path on the Go
    // side). Return 501 for now until that path lands.
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "file deletion pending Store trait expansion",
        })),
    )
        .into_response()
}

fn header_user(headers: &axum::http::HeaderMap) -> String {
    // The cookie / auth middleware in the gateway decodes the
    // session and stuffs the user_id into a custom header. Until
    // that middleware lands, the handler reads it directly from
    // a header for tests.
    headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn err_response(e: CleanClawError) -> axum::response::Response {
    let status = match &e {
        CleanClawError::NotFound(_) => StatusCode::NOT_FOUND,
        CleanClawError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
}
