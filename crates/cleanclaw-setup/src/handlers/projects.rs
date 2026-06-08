//! Project handlers. Mirrors
//! .
//!
//! Routes:
//!   * GET    /api/agents/:id/projects     - list an agent's projects
//!   * POST   /api/agents/:id/projects     - create a project
//!   * PATCH  /api/agents/:id/projects/:pid - rename / move
//!   * DELETE /api/agents/:id/projects/:pid
//!
//! A "project" is a per-(user, agent) folder of related sessions,
//! so the agent can swap workspace context per chat. The Go side
//! stores projects as a `projects` table; the Rust store has
//! `list_projects` / `get_project` / `save_project` / `delete_project`.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, patch, post},
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route(
            "/api/agents/:id/projects",
            get(list_projects).post(create_project),
        )
        .route(
            "/api/agents/:id/projects/:pid",
            patch(update_project).delete(delete_project),
        )
}

/// Resolve the owner of `agent_id` from the store when the
/// caller didn't pass an explicit `?user_id=` query. Used by
/// list/create/update/delete — all four need an owner key.
async fn resolve_owner(
    state: &Arc<ServerState>,
    agent_id: &str,
    explicit: &str,
) -> Result<String, CleanClawError> {
    if !explicit.is_empty() {
        return Ok(explicit.to_string());
    }
    // Look up the agent row to find its `user_id` (owner). Falls
    // back to the first user we can see — good enough for the
    // single-tenant dashboard, multi-tenant callers should pass
    // `?user_id=` explicitly or use the API layer's auth.
    if let Ok(agent) = state.store.get_agent(agent_id).await {
        if !agent.user_id.is_empty() {
            return Ok(agent.user_id);
        }
    }
    if let Ok(agents) = state.store.list_all_agents().await {
        if let Some(a) = agents.iter().find(|a| a.id == agent_id) {
            return Ok(a.user_id.clone());
        }
    }
    Ok(String::new())
}

#[derive(Serialize)]
struct ProjectDto {
    id: String,
    name: String,
    description: String,
    created_at: String,
}

#[derive(Deserialize, Default)]
struct OwnerQuery {
    #[serde(default)]
    user_id: String,
}

#[derive(Deserialize)]
struct CreateReq {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    user_id: String,
}

async fn list_projects(
    State(state): State<Arc<ServerState>>,
    Path(agent_id): Path<String>,
    Query(q): Query<OwnerQuery>,
) -> impl IntoResponse {
    let owner = match resolve_owner(&state, &agent_id, &q.user_id).await {
        Ok(o) => o,
        Err(e) => return internal(e).into_response(),
    };
    if owner.is_empty() {
        return (StatusCode::OK, Json(json!({"projects": []}))).into_response();
    }
    match state.store.list_projects(&owner, &agent_id).await {
        Ok(rows) => {
            let dtos: Vec<ProjectDto> = rows
                .into_iter()
                .map(|p| ProjectDto {
                    id: p.project_id,
                    name: p.name,
                    description: p.description,
                    created_at: p.created_at.to_rfc3339(),
                })
                .collect();
            (StatusCode::OK, Json(json!({"projects": dtos}))).into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

async fn create_project(
    State(state): State<Arc<ServerState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateReq>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return bad("name required");
    }
    let user_id = match resolve_owner(&state, &agent_id, &req.user_id).await {
        Ok(o) => o,
        Err(e) => return internal(e).into_response(),
    };
    if user_id.is_empty() {
        return bad("could not determine owner");
    }
    let now = chrono::Utc::now();
    let rec = cleanclaw_store::models::ProjectRecord {
        user_id,
        agent_id,
        project_id: format!("proj_{}", uuid::Uuid::new_v4().simple()),
        name: req.name,
        description: req.description,
        created_at: now,
        updated_at: now,
    };
    let id = rec.project_id.clone();
    match state.store.save_project(&rec).await {
        Ok(()) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

#[derive(Deserialize)]
struct UpdateReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

async fn update_project(
    State(state): State<Arc<ServerState>>,
    Path((agent_id, pid)): Path<(String, String)>,
    Query(q): Query<OwnerQuery>,
    Json(req): Json<UpdateReq>,
) -> impl IntoResponse {
    let owner = match resolve_owner(&state, &agent_id, &q.user_id).await {
        Ok(o) => o,
        Err(e) => return internal(e).into_response(),
    };
    if owner.is_empty() {
        return bad("agent not found");
    }
    let mut p = match state.store.get_project(&owner, &agent_id, &pid).await {
        Ok(p) => p,
        Err(e) => return internal(e).into_response(),
    };
    if let Some(n) = req.name {
        p.name = n;
    }
    if let Some(d) = req.description {
        p.description = d;
    }
    match state.store.save_project(&p).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn delete_project(
    State(state): State<Arc<ServerState>>,
    Path((agent_id, pid)): Path<(String, String)>,
    Query(q): Query<OwnerQuery>,
) -> impl IntoResponse {
    let owner = match resolve_owner(&state, &agent_id, &q.user_id).await {
        Ok(o) => o,
        Err(e) => return internal(e).into_response(),
    };
    if owner.is_empty() {
        return bad("agent not found");
    }
    match state.store.delete_project(&owner, &agent_id, &pid).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => internal(e).into_response(),
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
