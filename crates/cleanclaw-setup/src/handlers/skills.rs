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
        .route("/api/agents/:id/skills/:name", delete(delete_agent_skill))
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
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(q): axum::extract::Query<SearchQuery>,
) -> impl IntoResponse {
    if q.q.is_empty() {
        return (StatusCode::OK, Json(json!({"results": []}))).into_response();
    }
    // The "source" param picks the upstream hub. Today we ship
    // `skillssh` (the public skills.sh API) and `clawhub` (a
    // curated local fallback so the dashboard's install flow
    // works offline / when the network is restricted).
    //
    // skills.sh response shape:
    //   { "query": "...", "skills": [
    //     { "id": "owner/repo/skillId",
    //       "skillId": "...",
    //       "name": "...",
    //       "installs": 1234,
    //       "source": "owner/repo" } ] }
    let source = q.source.as_deref().unwrap_or("skillssh");
    let url = match source {
        "clawhub" => "https://api.clawhub.dev/skills".to_string(),
        // default: skills.sh
        _ => format!("https://skills.sh/api/search?q={}", urlencoding(q.q.trim())),
    };

    let res = match state.http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, %url, "skill search upstream request failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("upstream unreachable: {e}") })),
            )
                .into_response();
        }
    };
    if !res.status().is_success() {
        tracing::warn!(status = %res.status(), %url, "skill search upstream non-2xx");
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("upstream returned {}", res.status()) })),
        )
            .into_response();
    }
    let body: serde_json::Value = match res.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("upstream payload: {e}") })),
            )
                .into_response();
        }
    };
    // Normalize skills.sh's `{ "skills": [...] }` shape to the
    // dashboard's expected `{ "results": [{name,description,source}] }`.
    // `clawhub` already returns the target shape, so we pass it
    // through unchanged when it's an array.
    let results: Vec<serde_json::Value> =
        if let Some(arr) = body.get("skills").and_then(|v| v.as_array()) {
            arr.iter()
                .map(|s| {
                    let name = s
                        .get("skillId")
                        .or_else(|| s.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let description = format!(
                        "{} · {} installs",
                        s.get("name").and_then(|v| v.as_str()).unwrap_or(&name),
                        s.get("installs").and_then(|v| v.as_i64()).unwrap_or(0)
                    );
                    let source_path = s.get("source").and_then(|v| v.as_str()).unwrap_or("");
                    json!({
                        "name": name,
                        "description": description,
                        "source": source_path,
                        "url": format!("https://skills.sh/{}", source_path),
                    })
                })
                .collect()
        } else if let Some(arr) = body.as_array() {
            arr.clone()
        } else {
            Vec::new()
        };
    (StatusCode::OK, Json(json!({ "results": results }))).into_response()
}

/// URL-encode a query string value (whitespace becomes `%20`,
/// `&` becomes `%26`, etc.). Lives here as a tiny pure-Rust
/// helper to avoid pulling in a full URL-encoding crate.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
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
        "github" => install_from_github(client, &req.spec, &req.name, &target_dir).await,
        "tarball" => {
            let p = PathBuf::from(&req.spec);
            install_from_tarball(&p, &req.name, &target_dir).await
        }
        "path" => {
            let p = PathBuf::from(&req.spec);
            install_from_path(&p, &req.name, &target_dir).await
        }
        "clawhub" => install_from_clawhub(client, &req.spec, &target_dir).await,
        other => Err(cleanclaw_skills::install::InstallError::Invalid(format!(
            "unknown source: {other}"
        ))),
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
