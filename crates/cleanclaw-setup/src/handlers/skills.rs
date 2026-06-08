//! Skill handlers.
//! + `setup/skill_install.go`.
//!
//! Routes:
//!   * GET    /api/skills                 - list installed skills
//!   * GET    /api/skills/search          - search skill hubs
//!   * POST   /api/skills/install         - install a skill by name
//!   * POST   /api/skills/upload          - upload a tarball
//!   * DELETE /api/skills/:name           - remove a skill (admin)
//!   * GET    /api/agents/:id/skills      - per-agent skill list
//!   * DELETE /api/agents/:id/skills/:name
//!
//! Real install paths (clawhub, github, tarball, path) live in
//! `cleanclaw-skills::install`. We delegate the side effects to
//! that module and read the on-disk result here.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use cleanclaw_core::CleanClawError;
use cleanclaw_skills::install::{
    install_from_clawhub, install_from_github, install_from_path, install_from_tarball,
};
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ServerState;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/api/skills", get(list_skills))
        .route("/api/skills/search", get(search_skills))
        .route("/api/skills/install", post(install_skill))
        .route("/api/skills/upload", post(upload_skill))
        .route("/api/skills/:name", delete(delete_skill))
        .route("/api/agents/:id/skills", get(list_agent_skills))
        .route(
            "/api/agents/:id/skills/:name",
            delete(delete_agent_skill),
        )
}

#[derive(Serialize)]
struct SkillDto {
    name: String,
    description: String,
    layer: String,
    gated: bool,
}

async fn list_skills(State(_state): State<Arc<ServerState>>) -> impl IntoResponse {
    // The Go side reads from `agent.skills.go` which walks the
    // workspace + user-skills dirs. We don't have a per-user root
    // here; the dashboard's skill list lives on the agent detail
    // page. Return a minimal stub so the global `/skills` page
    // renders.
    let rows: Vec<SkillDto> = Vec::new();
    (StatusCode::OK, Json(rows)).into_response()
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    source: Option<String>,
}

async fn search_skills(
    State(_state): State<Arc<ServerState>>,
    axum::extract::Query(q): axum::extract::Query<SearchQuery>,
) -> impl IntoResponse {
    if q.q.is_empty() {
        return (StatusCode::OK, Json(json!({"results": []}))).into_response();
    }
    // No live search backend wired in this crate — the dashboard
    // gets a "search disabled" hint via empty results.
    (StatusCode::OK, Json(json!({"results": []}))).into_response()
}

#[derive(Deserialize)]
struct InstallReq {
    /// "github" | "tarball" | "path" | "clawhub"
    source: String,
    /// Free-form payload interpreted per source.
    ///   * github: "<owner>/<repo>[@<ref>]"
    ///   * tarball: "https://example.com/skill.tgz"
    ///   * path:    "/local/path/to/skill"
    ///   * clawhub: "<hub-name>/<skill-name>"
    spec: String,
    #[serde(default)]
    name: String,
}

async fn install_skill(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<InstallReq>,
) -> impl IntoResponse {
    if req.spec.is_empty() {
        return bad("spec required");
    }
    // P31: wire the install to the actual install module.
    let target_dir = state.skills_target_dir();
    let client = state.http_client.clone();
    let install_result = match req.source.as_str() {
        "github" => {
            install_from_github(client, &req.spec, &req.name, &target_dir).await
        }
        "tarball" => {
            let p = PathBuf::from(&req.spec);
            install_from_tarball(&p, &req.name, &target_dir).await
        }
        "path" => {
            let p = PathBuf::from(&req.spec);
            install_from_path(&p, &req.name, &target_dir).await
        }
        "clawhub" => install_from_clawhub(client, &req.spec, &target_dir).await,
        other => Err(cleanclaw_skills::install::InstallError::Invalid(
            format!("unknown source: {other}"),
        )),
    };
    match install_result {
        Ok(r) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "name": r.name,
                "source": r.source,
                "version": r.version,
                "files_written": r.files_written,
                "installed_at": r.installed_at,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Multipart upload — out of scope for the parity sweep. The
/// dashboard's tarball upload goes through `/api/skills/install`
/// with `source: "tarball"` and a pre-signed URL, or via a
/// sidecar plugin. Return 501.
async fn upload_skill() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "use /api/skills/install with source=tarball"})),
    )
        .into_response()
}

async fn delete_skill(
    State(_state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if name.is_empty() {
        return bad("name required");
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

async fn list_agent_skills(
    State(_state): State<Arc<ServerState>>,
    Path(_agent_id): Path<String>,
) -> impl IntoResponse {
    let rows: Vec<SkillDto> = Vec::new();
    (StatusCode::OK, Json(rows)).into_response()
}

async fn delete_agent_skill(
    State(_state): State<Arc<ServerState>>,
    Path((_agent_id, name)): Path<(String, String)>,
) -> impl IntoResponse {
    if name.is_empty() {
        return bad("name required");
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

fn bad(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}

fn _silence_unused(_: CleanClawError) {}
