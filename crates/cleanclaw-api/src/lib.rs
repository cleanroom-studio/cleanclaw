//! HTTP API server (axum). Mounted by the setup server on a sub-router
//! and exposed as `/api/*` and `/v1/*`.
//!
//! + the corresponding
//!   handlers in .go`.

#![allow(
    clippy::too_many_arguments,
    dead_code,
    unused_imports,
    unused_variables
)]

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use cleanclaw_auth::{Identity, Resolver};
use cleanclaw_core::{CleanClawError, Result, UserId};
use cleanclaw_provider::Message;
use cleanclaw_store::Store;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

pub mod apikey_endpoints;
pub mod chat;
pub mod chat_ws;
pub mod cron_endpoints;
pub mod openai_compat;
pub mod v1_users;
pub mod websocket;

pub use chat::ChatRequest as AgentChatRequest;
use chat::ChatService;
pub use websocket::WsState;

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<dyn Store>,
    pub auth: Arc<Resolver>,
    pub chat: Arc<ChatService>,
}

impl ApiState {
    pub fn new(store: Arc<dyn Store>, auth: Arc<Resolver>, chat: Arc<ChatService>) -> Self {
        Self { store, auth, chat }
    }

    /// Build a `WsState` for the WebSocket bridge from this state.
    pub fn ws_state(&self) -> WsState {
        WsState::new(self.auth.clone(), self.store.clone())
    }
}

pub fn router(state: ApiState) -> Router {
    let ws = state.ws_state();
    // The W1 (core) surface: status, register, login, logout, me,
    // agents CRUD, per-agent cron, apikeys, agent files, chat
    // streaming, WebSocket bridges, and the v1/* OpenAI-compat /
    // provisioning endpoints. The W2 surface (admin, channels,
    // skills, tools, plugins, projects, scoped config) is mounted
    // alongside by the gateway from `cleanclaw-setup`.
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(get_me))
        .route("/api/agents", get(list_agents).post(create_agent))
        .route("/api/agents/:id", get(get_agent).delete(delete_agent))
        .route(
            "/api/agents/:id/cron",
            get(cron_endpoints::list_cron_for_agent).post(cron_endpoints::create_cron),
        )
        .route(
            "/api/cron/:id",
            axum::routing::delete(cron_endpoints::delete_cron).patch(cron_endpoints::toggle_cron),
        )
        .route(
            "/api/agents/:id/files",
            get(apikey_endpoints::list_agent_files),
        )
        .route(
            "/api/agents/:id/files/:filename",
            get(apikey_endpoints::get_agent_file),
        )
        .route(
            "/api/apikeys",
            get(apikey_endpoints::list_api_keys).post(apikey_endpoints::create_api_key),
        )
        .route("/api/chat/stream", post(chat::stream))
        // Chat history + sessions (matches CleanClaw `/api/chat/*`
        // dashboard surface). The history endpoint re-emits
        // assistant / user / tool messages for a session so the
        // dashboard can hydrate the bubble stack on navigation.
        .route("/api/chat/history", get(chat::get_history))
        .route("/api/chat/sessions", get(chat::list_sessions))
        .route(
            "/api/chat/sessions/:key",
            axum::routing::delete(chat::delete_session).put(chat::rename_session),
        )
        .route(
            "/api/chat/sessions/:key/project",
            axum::routing::patch(chat::move_session_project),
        )
        // NOTE: /api/agents/:id/projects is owned by cleanclaw-setup
        // (handlers/projects.rs).
        .route("/api/ws", get(websocket::ws_handler).with_state(ws))
        .route("/api/ws/chat", get(chat_ws::chat_ws_handler))
        .route(
            "/v1/chat/completions",
            post(openai_compat::chat_completions),
        )
        .route("/v1/agents", get(list_v1_agents))
        .route("/v1/users", post(v1_users::provision_user))
        .with_state(state)
}

// ---- /api/status --------------------------------------------------------

#[derive(Serialize)]
struct StatusResponse {
    configured: bool,
    running: bool,
    port: u16,
    version: String,
    uptime: String,
    user_count: i64,
}

async fn get_status(State(state): State<ApiState>) -> Json<StatusResponse> {
    let user_count = state.store.count_users().await.unwrap_or(0);
    Json(StatusResponse {
        configured: user_count > 0,
        running: true,
        port: 18953,
        version: cleanclaw_core::BUILD_VERSION.to_string(),
        uptime: "0s".into(),
        user_count,
    })
}

// ---- /api/register ------------------------------------------------------

#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    email: String,
    password: String,
    display_name: Option<String>,
}

async fn register(
    State(state): State<ApiState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    match register_inner(&state, &req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => err_to_response(e),
    }
}

async fn register_inner(state: &ApiState, req: &RegisterRequest) -> Result<serde_json::Value> {
    if state.store.count_users().await? > 0 {
        return Err(CleanClawError::Forbidden);
    }
    let hash = cleanclaw_auth::password::hash_password(&req.password)?;
    let user = cleanclaw_store::models::UserRecord {
        id: UserId::generate().to_string(),
        username: req.username.clone(),
        email: req.email.clone(),
        password_hash: hash,
        display_name: req
            .display_name
            .clone()
            .unwrap_or_else(|| req.username.clone()),
        role: "super_admin".into(),
        status: "active".into(),
        apikey_id: String::new(),
        external_id: String::new(),
        avatar_url: String::new(),
        agent_quota: -1,
        created_at: cleanclaw_core::now_utc(),
        updated_at: cleanclaw_core::now_utc(),
    };
    state.store.create_user(&user).await?;
    Ok(json!({ "ok": true, "user_id": user.id }))
}

// ---- /api/login /api/logout ---------------------------------------------

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    ok: bool,
    user_id: String,
    username: String,
    role: String,
}

async fn login(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    match login_inner(&state, &req).await {
        Ok((resp, cookie)) => {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(c) = cookie {
                headers.insert(axum::http::header::SET_COOKIE, c.parse().unwrap());
            }
            (StatusCode::OK, headers, Json(resp)).into_response()
        }
        Err(e) => err_to_response(e),
    }
}

async fn login_inner(
    state: &ApiState,
    req: &LoginRequest,
) -> Result<(LoginResponse, Option<String>)> {
    let user = state
        .store
        .get_user_by_login(&req.username)
        .await
        .map_err(|_| CleanClawError::Unauthorized)?;
    let ok = cleanclaw_auth::password::verify_password(&req.password, &user.password_hash)?;
    if !ok {
        return Err(CleanClawError::Unauthorized);
    }
    let sid = cleanclaw_auth::session::new_token();
    let sess = cleanclaw_store::models::WebSessionRecord {
        sid: sid.clone(),
        user_id: user.id.clone(),
        created_at: cleanclaw_core::now_utc(),
        expires_at: cleanclaw_core::now_utc()
            + chrono::Duration::from_std(cleanclaw_auth::SESSION_TTL).unwrap(),
    };
    state.store.create_web_session(&sess).await?;
    let cookie = format!(
        "{}={}; HttpOnly; Path=/; SameSite=Lax; Max-Age={}",
        cleanclaw_auth::SESSION_COOKIE_NAME,
        sid,
        cleanclaw_auth::SESSION_TTL.as_secs()
    );
    Ok((
        LoginResponse {
            ok: true,
            user_id: user.id,
            username: user.username,
            role: user.role,
        },
        Some(cookie),
    ))
}

async fn logout(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(sid) = extract_cookie(&headers) {
        let _ = state.store.delete_web_session(&sid).await;
    }
    let mut headers = axum::http::HeaderMap::new();
    let cookie = format!(
        "{}=; HttpOnly; Path=/; Max-Age=0",
        cleanclaw_auth::SESSION_COOKIE_NAME
    );
    headers.insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    (StatusCode::OK, headers, Json(json!({"ok": true})))
}

// ---- /api/me ------------------------------------------------------------

async fn get_me(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let bearer = extract_bearer(&headers);
    let cookie = extract_cookie(&headers);
    match state
        .auth
        .resolve(bearer.as_deref(), cookie.as_deref())
        .await
    {
        Ok(Some(ident)) => {
            let u = state.store.get_user(&ident.user_id).await.ok();
            match u {
                Some(u) => (
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "user": {
                            "id": u.id,
                            "username": u.username,
                            "email": u.email,
                            "role": u.role,
                            "is_admin": u.role == "super_admin" || u.role == "admin",
                        }
                    })),
                )
                    .into_response(),
                None => (StatusCode::UNAUTHORIZED, Json(json!({"ok": false}))).into_response(),
            }
        }
        _ => (StatusCode::UNAUTHORIZED, Json(json!({"ok": false}))).into_response(),
    }
}

// ---- /api/agents -------------------------------------------------------

#[derive(Serialize)]
struct AgentSummary {
    id: String,
    name: String,
    user_id: String,
    is_public: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_agents(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let ident = match require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let rows = if ident.is_super_admin() {
        state.store.list_all_agents().await
    } else {
        state.store.list_agents(&ident.user_id).await
    };
    match rows {
        Ok(rows) => {
            let list: Vec<AgentSummary> = rows
                .into_iter()
                .map(|a| AgentSummary {
                    id: a.id,
                    name: a.name,
                    user_id: a.user_id,
                    is_public: a.is_public,
                    created_at: a.created_at,
                })
                .collect();
            (StatusCode::OK, Json(json!({"agents": list}))).into_response()
        }
        Err(e) => err_to_response(e),
    }
}

async fn list_v1_agents(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    list_agents(State(state), headers).await
}

#[derive(Deserialize)]
struct CreateAgentRequest {
    name: String,
    model: String,
    soul: Option<String>,
    identity: Option<String>,
}

async fn create_agent(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateAgentRequest>,
) -> impl IntoResponse {
    let ident = match require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    if !ident.can_create_agent() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "not authorized"})),
        )
            .into_response();
    }
    let agent = cleanclaw_store::models::AgentRecord {
        id: cleanclaw_core::AgentId::generate().to_string(),
        user_id: ident.user_id.clone(),
        name: req.name.clone(),
        config: json!({ "model": req.model }),
        is_public: false,
        created_at: cleanclaw_core::now_utc(),
        updated_at: cleanclaw_core::now_utc(),
    };
    if let Err(e) = state.store.save_agent(&agent).await {
        return err_to_response(e);
    }
    if let Some(soul) = &req.soul {
        if let Err(e) = state
            .store
            .save_workspace_file(&agent.id, "", "SOUL.md", soul.as_bytes())
            .await
        {
            return err_to_response(e);
        }
    }
    if let Some(identity) = &req.identity {
        if let Err(e) = state
            .store
            .save_workspace_file(&agent.id, "", "IDENTITY.md", identity.as_bytes())
            .await
        {
            return err_to_response(e);
        }
    }
    (StatusCode::CREATED, Json(json!({"agent": agent}))).into_response()
}

async fn get_agent(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ident = match require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    match state.store.get_agent(&id).await {
        Ok(a) => {
            if !ident.can_access_agent(&a.id)
                && a.user_id != ident.user_id
                && !ident.is_super_admin()
            {
                return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"})))
                    .into_response();
            }
            (StatusCode::OK, Json(json!({"agent": a}))).into_response()
        }
        Err(e) => err_to_response(e),
    }
}

async fn delete_agent(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ident = match require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    match state.store.get_agent(&id).await {
        Ok(a) => {
            if a.user_id != ident.user_id && !ident.is_super_admin() {
                return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"})))
                    .into_response();
            }
            if let Err(e) = state.store.delete_agent(&id).await {
                return err_to_response(e);
            }
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }
        Err(e) => err_to_response(e),
    }
}

// ---- helpers -------------------------------------------------------------

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

fn extract_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').map(|s| s.trim()).find_map(|c| {
                let (k, v) = c.split_once('=')?;
                if k == cleanclaw_auth::SESSION_COOKIE_NAME {
                    Some(v.to_string())
                } else {
                    None
                }
            })
        })
}

async fn require_auth(
    state: &ApiState,
    headers: &axum::http::HeaderMap,
) -> std::result::Result<Identity, axum::response::Response> {
    let bearer = extract_bearer(headers);
    let cookie = extract_cookie(headers);
    match state
        .auth
        .resolve(bearer.as_deref(), cookie.as_deref())
        .await
    {
        Ok(Some(ident)) => Ok(ident),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()),
    }
}

fn err_to_response(e: CleanClawError) -> axum::response::Response {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(json!({"error": {"message": e.to_string()}}))).into_response()
}

// =====================================================================
// Additional HTTP endpoints (OpenAI-compat + per-resource CRUD).
//
// =====================================================================

/// Rate-limit token bucket. Per-tenant; one refills per second.
pub mod ratelimit {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    #[derive(Debug, Clone)]
    pub struct TokenBucket {
        capacity: u32,
        refill_per_sec: u32,
        state: Arc<Mutex<BucketState>>,
    }

    #[derive(Debug)]
    struct BucketState {
        tokens: f64,
        last_refill: std::time::Instant,
    }

    impl TokenBucket {
        pub fn new(capacity: u32, refill_per_sec: u32) -> Self {
            Self {
                capacity,
                refill_per_sec,
                state: Arc::new(Mutex::new(BucketState {
                    tokens: capacity as f64,
                    last_refill: std::time::Instant::now(),
                })),
            }
        }

        /// Try to consume one token. Returns true if the request is
        /// allowed; false if rate-limited.
        pub async fn try_acquire(&self) -> bool {
            let mut s = self.state.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(s.last_refill).as_secs_f64();
            s.tokens = (s.tokens + elapsed * self.refill_per_sec as f64).min(self.capacity as f64);
            s.last_refill = now;
            if s.tokens >= 1.0 {
                s.tokens -= 1.0;
                true
            } else {
                false
            }
        }
    }

    /// Convenience constructor for a default "10 req/sec" bucket.
    pub fn default_bucket() -> TokenBucket {
        TokenBucket::new(20, 10)
    }

    pub fn per_minute_bucket(per_minute: u32) -> TokenBucket {
        let per_sec = (per_minute as f64 / 60.0).ceil() as u32;
        TokenBucket::new(per_minute, per_sec)
    }
}

#[cfg(test)]
mod ratelimit_tests {
    use super::ratelimit::*;
    use std::time::Duration;

    #[tokio::test]
    async fn bucket_starts_full() {
        let b = TokenBucket::new(5, 1);
        for _ in 0..5 {
            assert!(b.try_acquire().await);
        }
        assert!(!b.try_acquire().await);
    }

    #[tokio::test]
    async fn bucket_refills() {
        let b = TokenBucket::new(5, 10);
        for _ in 0..5 {
            assert!(b.try_acquire().await);
        }
        assert!(!b.try_acquire().await);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(b.try_acquire().await);
    }

    #[tokio::test]
    async fn default_bucket_shape() {
        let b = default_bucket();
        for _ in 0..20 {
            assert!(b.try_acquire().await);
        }
        assert!(!b.try_acquire().await);
    }

    #[tokio::test]
    async fn per_minute_bucket_shape() {
        let b = per_minute_bucket(60);
        for _ in 0..60 {
            assert!(b.try_acquire().await);
        }
    }
}

// =====================================================================
// Attachment upload + WebSocket streaming chat stub.
// and .
// =====================================================================

/// Per-upload attachment record returned by the upload handler.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentRecord {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
}

/// Validate + persist a binary upload. Returns the record to expose back
/// to the caller (id, name, mime, size). The payload itself is written
/// to `dest_dir/<id>` and the caller is expected to wire that path to
/// the chat pipeline.
pub fn persist_attachment(
    dest_dir: &std::path::Path,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> std::result::Result<AttachmentRecord, CleanClawError> {
    if name.is_empty() {
        return Err(CleanClawError::InvalidArgument(
            "attachment name required".into(),
        ));
    }
    if bytes.is_empty() {
        return Err(CleanClawError::InvalidArgument(
            "attachment payload empty".into(),
        ));
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(CleanClawError::InvalidArgument(format!(
            "attachment too large: {} > {}",
            bytes.len(),
            MAX_ATTACHMENT_BYTES
        )));
    }
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| CleanClawError::Internal(format!("mkdir: {e}")))?;
    let id = cleanclaw_core::idgen::IdGen::new().next("att");
    let path = dest_dir.join(&id);
    std::fs::write(&path, bytes).map_err(|e| CleanClawError::Internal(format!("write: {e}")))?;
    Ok(AttachmentRecord {
        id,
        name: name.to_string(),
        mime: mime.to_string(),
        size: bytes.len() as u64,
    })
}

pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// WebSocket-style chunked chat frame. One inbound prompt can produce
/// zero or more `StreamChunk`s plus a final `StreamEnd` marker.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StreamChunk {
    /// Token-style partial (text fragment).
    Token(String),
    /// Reasoning/trace (optional).
    Trace(String),
    /// Final frame with the complete assistant text and usage summary.
    End { full: String, tokens: u32 },
}

/// Trivial chunker used by tests + offline mode. Splits a finalized
/// string into space-delimited tokens and yields them with a final End
/// frame carrying the full payload.
pub fn chunk_assistant_text(full: &str, tokens: u32) -> Vec<StreamChunk> {
    let mut out: Vec<StreamChunk> = Vec::new();
    for tok in full.split_whitespace() {
        out.push(StreamChunk::Token(tok.to_string()));
    }
    out.push(StreamChunk::End {
        full: full.to_string(),
        tokens,
    });
    out
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    #[test]
    fn persist_attachment_writes_file() {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-att-{}-{}",
            std::process::id(),
            cleanclaw_core::idgen::IdGen::new().next("t")
        ));
        let rec =
            persist_attachment(&dir, "hello.txt", "text/plain", b"hi there").expect("persist ok");
        assert!(!rec.id.is_empty());
        assert_eq!(rec.name, "hello.txt");
        assert_eq!(rec.mime, "text/plain");
        assert_eq!(rec.size, 8);
        let read = std::fs::read(dir.join(&rec.id)).expect("read back");
        assert_eq!(read, b"hi there");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_attachment_rejects_empty_payload() {
        let dir = std::env::temp_dir().join(format!("cleanclaw-att-empty-{}", std::process::id()));
        let err = persist_attachment(&dir, "x.txt", "text/plain", b"").unwrap_err();
        matches!(err, CleanClawError::InvalidArgument(_));
    }

    #[test]
    fn persist_attachment_rejects_huge_payload() {
        let dir = std::env::temp_dir().join(format!("cleanclaw-att-huge-{}", std::process::id()));
        let huge = vec![0u8; MAX_ATTACHMENT_BYTES + 1];
        let err = persist_attachment(&dir, "x.bin", "application/octet-stream", &huge).unwrap_err();
        matches!(err, CleanClawError::InvalidArgument(_));
    }

    #[test]
    fn chunk_assistant_text_emits_end() {
        let chunks = chunk_assistant_text("hi there friend", 7);
        assert_eq!(chunks.len(), 4); // 3 tokens + 1 end
        let last = chunks.last().unwrap();
        match last {
            StreamChunk::End { full, tokens } => {
                assert_eq!(full, "hi there friend");
                assert_eq!(*tokens, 7);
            }
            other => panic!("expected End, got {other:?}"),
        }
    }
}
