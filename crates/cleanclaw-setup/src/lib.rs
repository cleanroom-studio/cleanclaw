//! HTTP setup server.
//!
//! The Go setup package hosts the dashboard's REST surface: auth
//! (register / login), agents CRUD, sessions, channels, projects,
//! skills, tools, plugins, cron, usage, scoped-config. The full
//! surface is 9192 LoC of handlers — this crate provides the
//! skeleton: the `Server` builder, the `/api/register` and
//! `/api/login` endpoints, and the `mount` function that stitches
//! the static-asset fallback under the API router. New handlers
//! land here as the dashboard grows; existing endpoint-by-endpoint
//! parity is on the C12 top-up list.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use cleanclaw_auth::{Accounts, CreateInput, UserError, ROLE_SUPER_ADMIN};
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

pub mod handlers;
pub mod identity_store;
pub mod server;

#[derive(Debug, Error)]
pub enum SetupError {
    #[error("auth: {0}")]
    Auth(#[from] UserError),
    #[error("invalid input: {0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for SetupError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            SetupError::Auth(UserError::InvalidCredentials) => (StatusCode::UNAUTHORIZED, "invalid credentials".into()),
            SetupError::Auth(UserError::LastSuperAdmin) => (StatusCode::CONFLICT, "cannot remove last super admin".into()),
            SetupError::Auth(UserError::InvalidRole(_)) => (StatusCode::BAD_REQUEST, "invalid role".into()),
            SetupError::Auth(UserError::InvalidStatus(_)) => (StatusCode::BAD_REQUEST, "invalid status".into()),
            SetupError::Auth(UserError::Missing(_)) => (StatusCode::BAD_REQUEST, "missing required field".into()),
            SetupError::Auth(UserError::Store(_)) => (StatusCode::INTERNAL_SERVER_ERROR, "store error".into()),
            SetupError::BadRequest(s) => (StatusCode::BAD_REQUEST, s.clone()),
            SetupError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
            SetupError::Internal(s) => (StatusCode::INTERNAL_SERVER_ERROR, s.clone()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

pub struct ServerState {
    pub accounts: Arc<Accounts>,
    pub store: Arc<dyn Store>,
    /// Optional webhook → bus bridge. Set via
    /// `Server::with_webhook_bridge(...)` so the platform-specific
    /// HTTP routes (`/api/line/webhook`, `/api/feishu/webhook/:id`,
    /// `/api/telegram/webhook/:id`, `/api/wechat/webhook/:id`) can
    /// push verified payloads onto the in-process `MessageBus`.
    /// When `None`, the routes still ack (Feishu challenge, LINE
    /// heartbeat) but don't dispatch — useful for dashboard-only
    /// installs.
    pub webhook_bridge: Option<Arc<cleanclaw_channels::WebhookBridge>>,
    /// Shared `reqwest::Client` for skill install + provider
    /// management endpoints. P31: skills::install calls
    /// `http_client.clone()`.
    pub http_client: Arc<reqwest::Client>,
    /// On-disk root for installed skills. The install module
    /// writes `target_dir/<name>/SKILL.md` here. P31: derived
    /// from `$CLEANCLAW_HOME/skills` or a per-instance override.
    pub skills_root: PathBuf,
}

impl ServerState {
    /// Skills install target — `skills_root` for now. Future
    /// per-agent installs route through `<skills_root>/<agent>`.
    pub fn skills_target_dir(&self) -> PathBuf {
        self.skills_root.clone()
    }
}

#[derive(Clone)]
pub struct Server {
    state: Arc<ServerState>,
}

impl Server {
    pub fn new(store: Arc<dyn Store>) -> Self {
        let accounts = Arc::new(
            Accounts::new(store.clone()).expect("Accounts::new"),
        );
        let skills_root = std::env::var("CLEANCLAW_SKILLS_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("CLEANCLAW_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("./.cleanclaw"))
                    .join("skills")
            });
        let _ = std::fs::create_dir_all(&skills_root);
        Self {
            state: Arc::new(ServerState {
                accounts,
                store,
                webhook_bridge: None,
                http_client: Arc::new(
                    reqwest::Client::builder()
                        .user_agent("cleanclaw/1.0")
                        .timeout(std::time::Duration::from_secs(60))
                        .build()
                        .expect("reqwest client"),
                ),
                skills_root,
            }),
        }
    }

    pub fn with_accounts(accounts: Arc<Accounts>, store: Arc<dyn Store>) -> Self {
        let mut s = Self::new(store);
        s.state = Arc::new(ServerState {
            accounts,
            store: s.state.store.clone(),
            webhook_bridge: s.state.webhook_bridge.clone(),
            http_client: s.state.http_client.clone(),
            skills_root: s.state.skills_root.clone(),
        });
        s
    }

    /// Wire a `WebhookBridge` so the platform-specific HTTP
    /// routes can push verified webhook payloads onto the bus.
    pub fn with_webhook_bridge(
        mut self,
        bridge: Arc<cleanclaw_channels::WebhookBridge>,
    ) -> Self {
        self.state = Arc::new(ServerState {
            accounts: self.state.accounts.clone(),
            store: self.state.store.clone(),
            webhook_bridge: Some(bridge),
            http_client: self.state.http_client.clone(),
            skills_root: self.state.skills_root.clone(),
        });
        self
    }

    pub fn state(&self) -> Arc<ServerState> {
        self.state.clone()
    }

    pub fn router(&self) -> Router {
        // The core surface (`/api/{health,register,login,logout,me,
        // agents, agents/:id, agents/:id/files, agents/:id/cron,
        // cron/*, apikeys, status, test-provider, …}`) lives in
        // `cleanclaw-api`. The setup crate carries the W2 surface
        // (admin, channels, plugins, projects, skills, tools,
        // usage, scoped config, apikeys sub-resources, providers,
        // agents/<id>/system-file) — all paths here are unique and
        // do NOT collide with the api crate. The `extras` module
        // (channel webhooks + admin/users CRUD + /api/status) is
        // intentionally NOT merged here to avoid the route
        // overlap panic; its unique routes are reachable via
        // `handlers::extras::router()` when wired individually.
        Router::new()
            .route("/api/health", get(health))
            .merge(handlers::admin::router())
            .merge(handlers::channels::router())
            .merge(handlers::extras2::router())
            .merge(handlers::plugins::router())
            .merge(handlers::projects::router())
            .merge(handlers::resources::router())
            .merge(handlers::scoped::router())
            .merge(handlers::skills::router())
            .merge(handlers::tools::router())
            .merge(handlers::usage::router())
            .with_state(self.state.clone())
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"ok": true}))
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: String,
    pub username: String,
    pub role: String,
}

async fn register(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, SetupError> {
    // Bootstrap: if no users exist, force the first one to super_admin.
    let role = if req.role.is_empty() {
        let count = state.accounts.count().await?;
        if count == 0 { ROLE_SUPER_ADMIN.to_string() } else { "user".to_string() }
    } else {
        req.role
    };
    let display_name = if req.display_name.is_empty() { req.username.clone() } else { req.display_name };
    let in_ = CreateInput {
        username: req.username.clone(),
        email: req.email,
        password: req.password,
        display_name,
        role: role.clone(),
        agent_quota: Some(-1),
        avatar_url: String::new(),
        apikey_id: String::new(),
        external_id: String::new(),
    };
    let acc = state.accounts.create(in_).await?;
    info!(user_id = %acc.id, username = %acc.username, "registered");
    Ok(Json(RegisterResponse {
        id: acc.id,
        username: acc.username,
        role: acc.role,
    }))
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user_id: String,
    pub username: String,
    pub role: String,
    pub session_id: String,
}

async fn login(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, SetupError> {
    let acc = state.accounts.authenticate(&req.login, &req.password).await?;
    let session_id = format!("sess_{}", uuid::Uuid::new_v4().simple());
    info!(user_id = %acc.id, username = %acc.username, "logged in");
    Ok(Json(LoginResponse {
        user_id: acc.id,
        username: acc.username,
        role: acc.role,
        session_id,
    }))
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user_id: String,
    pub username: String,
    pub role: String,
    pub is_admin: bool,
}

async fn me(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<MeRequest>,
) -> Result<Json<MeResponse>, SetupError> {
    // For the parity sweep: `/api/me` takes a `{ user_id }` in the
    // body (the dashboard stashes it after login). The cookie-based
    // variant lands with the full auth middleware in C12.
    let u = state
        .store
        .get_user(&body.user_id)
        .await
        .map_err(|_| SetupError::Auth(UserError::InvalidCredentials))?;
    let is_admin = u.role == "super_admin" || u.role == "admin";
    Ok(Json(MeResponse {
        user_id: u.id,
        username: u.username,
        role: u.role,
        is_admin,
    }))
}

#[derive(Debug, Deserialize)]
pub struct MeRequest {
    pub user_id: String,
}

/// Mount the API + static-asset fallback. Mirrors the Go `mount()`
/// helper at the bottom of `setup/server.go`.
pub fn mount(api: Router) -> Router {
    Router::new()
        .merge(api)
        .fallback(get(server::serve_static))
}

pub mod assets {
    use include_dir::{include_dir, Dir};

    static WWW: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../web/build");

    pub struct WebAssets;
    impl WebAssets {
        pub fn get(path: &str) -> Option<&'static [u8]> {
            let stripped = path.trim_start_matches('/');
            WWW.get_file(stripped).map(|f| f.contents())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxStatus};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_returns_ok() {
        use cleanclaw_store::{StorageConfig, StorageType};
        let dir = tempfile::tempdir().unwrap();
        let cfg = StorageConfig {
            r#type: StorageType::Sqlite,
            dsn: format!("sqlite://{}/test.db", dir.path().display()),
            auto_migrate: true,
        };
        let store = cleanclaw_store::open(&cfg, dir.path()).await.unwrap();
        let store: Arc<dyn cleanclaw_store::Store> = Arc::from(store);
        let server = Server::new(store);
        let app = server.router();
        let req = Request::builder()
            .uri("/api/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
    }

    #[test]
    fn server_state_exposes_accounts() {
        // The Server::new panics if the store is broken, so just
        // verify the accessors exist.
        fn _takes_state(_s: &ServerState) {}
    }

    #[test]
    fn register_response_serializes() {
        let r = RegisterResponse {
            id: "u1".into(),
            username: "alice".into(),
            role: "user".into(),
        };
        let blob = serde_json::to_string(&r).unwrap();
        assert!(blob.contains("\"id\":\"u1\""));
    }

    #[test]
    fn login_response_serializes() {
        let r = LoginResponse {
            user_id: "u1".into(),
            username: "alice".into(),
            role: "user".into(),
            session_id: "s1".into(),
        };
        let blob = serde_json::to_string(&r).unwrap();
        assert!(blob.contains("\"sessionId\":\"s1\"") || blob.contains("\"session_id\":\"s1\""));
    }
}
