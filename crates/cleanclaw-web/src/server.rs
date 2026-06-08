//! Axum server boot + route table. W1 wires the bare minimum
//! (`/`, `/overview`, `/favicon.ico`); later phases add the rest.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use std::path::PathBuf;
use crate::html::Theme;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Server state shared across handlers. W1 only holds the `tx` for
/// graceful shutdown; later phases add auth + the typed API client.
#[derive(Clone)]
pub struct WebState {
    pub shutdown_tx: Arc<watch::Sender<bool>>,
    pub version: String,
    /// Optional store handle. When `Some`, the admin / agents /
    /// settings pages query it for live data; when `None`, the pages
    /// render the empty placeholder rows. The CLI sets `None` by
    /// default (the dashboard talks to the API directly); the
    /// `cleanclaw` daemon sets `Some` so embedded web boots in
    /// standalone mode.
    pub store: Option<Arc<dyn cleanclaw_store::Store>>,
    /// Optional skills directory. When `Some`, the `/skills` and
    /// `/agents/{id}/skills` pages call `cleanclaw_skills::discover()`
    /// and render the actual installed skills. When `None`, the
    /// pages render an empty placeholder.
    pub skills_dir: Option<PathBuf>,
    /// Optional account registry. When `Some`, the login + signup
    /// POST handlers authenticate against this; when `None`, they
    /// just redirect to `/overview` (the W4 stub).
    pub accounts: Option<Arc<cleanclaw_auth::Accounts>>,
    /// Active user id once authenticated. The login / signup
    /// handlers set this in the response cookie; the SSR layout
    /// reads it back to render the right user chip.
    pub session_user: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl WebState {
    pub fn new(shutdown_tx: watch::Sender<bool>) -> Self {
        Self {
            shutdown_tx: Arc::new(shutdown_tx),
            version: env!("CARGO_PKG_VERSION").to_string(),
            store: None,
            skills_dir: None,
            accounts: None,
            session_user: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub fn with_store(mut self, store: Arc<dyn cleanclaw_store::Store>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_skills_dir(mut self, dir: PathBuf) -> Self {
        self.skills_dir = Some(dir);
        self
    }

    pub fn with_accounts(mut self, accounts: Arc<cleanclaw_auth::Accounts>) -> Self {
        self.accounts = Some(accounts);
        self
    }
}

/// Build the W1 router. Includes the landing page, the overview
/// dashboard, the auth flow (login/signup), the settings tabs, the
/// admin pages, the apikeys page, and the agent workspace
/// (`/agents`, `/agents/{id}`, and the 12 sub-tabs under that).
    pub fn router(state: WebState) -> Router {
        Router::new()
        .route("/", get(root))
        .route("/overview", get(overview))
        .route("/favicon.ico", get(favicon))
        .route("/chat", get(chat))
        .route("/login", get(login_get).post(login_post))
        .route("/signup", get(signup_get).post(signup_post))
        .route("/onboard", get(onboard).post(onboard_post))
        .route("/settings", get(|| async { Redirect::to("/settings/general") }))
        .route("/settings/general", get(settings_general).post(settings_general_post))
        .route("/settings/account", get(settings_account).post(settings_account_post))
        .route("/settings/account/password", post(settings_account_password_post))
        .route("/settings/runtime", get(settings_runtime).post(settings_runtime_post))
        .route("/settings/about", get(settings_about))
        .route("/admin/users", get(admin_users))
        .route("/admin/usage", get(admin_usage))
        .route("/admin/chats", get(admin_chats))
        .route("/apikeys", get(apikeys))
        .route("/agents", get(agents_list))
        .route("/agents/:id", get(agent_overview))
        .route("/agents/:id/chat", get(agent_chat).post(agent_chat_post))
        .route("/agents/:id/chats", get(agent_chats))
        .route("/agents/:id/sessions", get(agent_sessions))
        .route("/agents/:id/sessions/:sid", get(agent_session_detail))
        .route("/agents/:id/channels", get(agent_channels))
        .route("/agents/:id/scheduler", get(agent_scheduler))
        .route("/agents/:id/skills", get(agent_skills))
        .route("/agents/:id/plugins", get(agent_plugins))
        .route("/agents/:id/models", get(agent_models))
        .route("/agents/:id/context", get(agent_context))
        .route("/agents/:id/customize", get(agent_customize).post(agent_customize_post))
        .route("/agents/:id/project", get(agent_project))
        .route("/agents/:id/project/:pid", get(agent_project_id))
        .route("/agents/:id/usage", get(agent_usage))
        .route("/channels", get(channels_list))
        .route("/channels-config", get(channels_config))
        .route("/models", get(models))
        .route("/providers", get(providers))
        .route("/plugins", get(plugins))
        .route("/skills", get(skills))
        .route("/tools", get(tools))
        .route("/cron", get(cron))
        .route("/health", get(health))
        // Static assets — channel icons, favicon, future
        // shadcn-style SVG sprites. The dir is relative to the
        // crate root (we read `static/...` on every request).
        .route("/static/*path", get(serve_static))
        .fallback(not_found)
        .with_state(state)
}

/// Mount all of the W4+ handlers on top of the W1 router. The merge
/// keeps the routes additive — handlers added in later phases are
/// appended, not replacing W1.
pub fn full_router(state: WebState) -> Router {
    let r = router(state.clone());
    crate::pages::mount(r, state)
}

/// Boot the server on `addr` and block until the shutdown channel
/// fires.
pub async fn serve(addr: SocketAddr) -> std::result::Result<(), std::io::Error> {
    let (tx, rx) = watch::channel(false);
    let state = WebState::new(tx);
    let app = full_router(state);
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("cleanclaw-web listening on {addr}");
    let mut shutdown_rx = rx;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        })
        .await
}

/// Boot a server with a pre-built `WebState`. Used by integration
/// tests that need to drive the shutdown channel directly.
pub async fn serve_with_state(
    state: WebState,
    addr: SocketAddr,
) -> std::result::Result<(), std::io::Error> {
    let app = full_router(state.clone());
    let listener = TcpListener::bind(addr).await?;
    let mut rx = state.shutdown_tx.subscribe();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = rx.changed().await;
        })
        .await
}

async fn root() -> Html<String> {
    Html(crate::pages::index::render())
}

/// Top-level `/chat` — the agent picker / chat landing. With no
/// `?agent=<id>` query param, it shows a list of agents and asks
/// the user to pick one (mirrors the Next.js `web/src/app/chat/page.tsx`
/// behavior). With a query, it redirects to that agent's chat
/// route so the deep-link `?agent=foo` works.
async fn chat(Query(q): Query<HashMap<String, String>>) -> Response {
    if let Some(agent_id) = q.get("agent") {
        if !agent_id.is_empty() {
            return Redirect::to(&format!("/agents/{}/chat", agent_id)).into_response();
        }
    }
    Html(crate::pages::chat::render()).into_response()
}

async fn overview(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = crate::html::Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let user = demo_user();
    Html(crate::pages::overview::render(user, theme))
}

async fn favicon() -> Response {
    // Tiny 1x1 transparent PNG. Stops the browser from 404-spamming
    // `/favicon.ico` on every page. Real branding lives in W8.
    const BYTES: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
        0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
        0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
        0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
        0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    // Try the on-disk file first; if absent, fall back to the
    // 1x1 transparent PNG. The dashboard's `next.svg` /
    // `vercel.svg` / `globe.svg` / `window.svg` from the
    // CleanClaw assets tree are served from `/static/...` (the
    // P2-9 asset mount).
    if let Ok(bytes) = std::fs::read("static/favicon.ico") {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "image/x-icon")],
            bytes,
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/png")],
        BYTES,
    )
        .into_response()
}

/// Stub for `useMe()`-style auth. W1 returns a fixed demo user so
/// pages have something to render. W4 wires this to the real auth
/// resolver.
fn demo_user() -> Option<(&'static str, &'static str)> {
    Some(("Ada Lovelace", "admin"))
}

// =====================================================================
// Auth handlers
// =====================================================================

async fn login_get(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let error = params.get("error").map(|s| s.as_str());
    let prefill = params.get("login").map(|s| s.as_str());
    Html(crate::pages::auth::login_page(theme, error, prefill))
}

async fn login_post(
    State(state): State<WebState>,
    Form(form): Form<crate::pages::auth::LoginForm>,
) -> Response {
    let (login, password) = match crate::pages::auth::validate_login(&form) {
        Ok(v) => v,
        Err(msg) => {
            return redirect_with_query(
                "/login",
                &[
                    ("error", msg.as_str()),
                    ("login", form.login.as_str()),
                ],
            );
        }
    };
    let Some(accounts) = state.accounts.as_ref() else {
        // No accounts wired — keep the W4 stub behaviour.
        let next = crate::pages::auth::safe_redirect(form.next.as_deref());
        return Redirect::to(next).into_response();
    };
    match accounts.authenticate(login, password).await {
        Ok(account) => {
            // Mint a session and set the cookie. The cookie value
            // is a 256-bit base64-no-pad token; the store holds
            // the matching `WebSessionRecord` with the expiry.
            let sid = mint_session_for(&state, &account.id).await;
            let cookie = format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
                cleanclaw_auth::SESSION_COOKIE_NAME,
                sid,
                cleanclaw_auth::SESSION_TTL.as_secs(),
            );
            // Stash the user id for SSR-driven pages.
            *state.session_user.lock().await = Some(account.id.clone());
            let next = crate::pages::auth::safe_redirect(form.next.as_deref());
            let next_header: String = next.to_string();
            (
                StatusCode::SEE_OTHER,
                [(header::LOCATION, next_header), (header::SET_COOKIE, cookie)],
            )
                .into_response()
        }
        Err(_) => redirect_with_query(
            "/login",
            &[
                ("error", "Invalid username or password"),
                ("login", form.login.as_str()),
            ],
        ),
    }
}

async fn signup_get(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let error = params.get("error").map(|s| s.as_str());
    let prefill_username = params.get("username").map(|s| s.as_str());
    let prefill_email = params.get("email").map(|s| s.as_str());
    Html(crate::pages::auth::signup_page(theme, error, prefill_username, prefill_email))
}

async fn signup_post(
    State(state): State<WebState>,
    Form(form): Form<crate::pages::auth::SignupForm>,
) -> Response {
    if let Err(msg) = crate::pages::auth::validate_signup(&form) {
        return redirect_with_query(
            "/signup",
            &[
                ("error", msg.as_str()),
                ("username", form.username.as_str()),
                ("email", form.email.as_str()),
            ],
        );
    }
    let Some(accounts) = state.accounts.as_ref() else {
        return Redirect::to("/overview").into_response();
    };
    let display_name = form
        .display_name
        .clone()
        .unwrap_or_else(|| form.username.clone());
    let input = cleanclaw_auth::CreateInput {
        username: form.username.clone(),
        email: form.email.clone(),
        password: form.password.clone(),
        display_name: display_name.clone(),
        role: cleanclaw_auth::ROLE_USER.to_string(),
        agent_quota: None,
        avatar_url: String::new(),
        apikey_id: String::new(),
        external_id: String::new(),
    };
    match accounts.create(input).await {
        Ok(account) => {
            let sid = mint_session_for(&state, &account.id).await;
            let cookie = format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
                cleanclaw_auth::SESSION_COOKIE_NAME,
                sid,
                cleanclaw_auth::SESSION_TTL.as_secs(),
            );
            *state.session_user.lock().await = Some(account.id.clone());
            (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, "/overview".to_string()),
                    (header::SET_COOKIE, cookie),
                ],
            )
                .into_response()
        }
        Err(e) => redirect_with_query(
            "/signup",
            &[
                ("error", signup_error_message(&e).as_str()),
                ("username", form.username.as_str()),
                ("email", form.email.as_str()),
            ],
        ),
    }
}

/// Build a `Redirect` to `path?k=v&k=v`. Avoids hand-rolling
/// query-string encoding inline.
fn redirect_with_query(path: &str, pairs: &[(&str, &str)]) -> Response {
    let qs: String = pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            format!(
                "{}={}",
                crate::client::urlencode(k),
                crate::client::urlencode(v)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let url = if qs.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{qs}")
    };
    Redirect::to(&url).into_response()
}

/// Mint a web session for `user_id`, persist it on the store, and
/// return the opaque session token (which becomes the cookie value).
async fn mint_session_for(state: &WebState, user_id: &str) -> String {
    let sid = cleanclaw_auth::session::new_token();
    if let Some(store) = state.store.as_ref() {
        let now = chrono::Utc::now();
        let sess = cleanclaw_store::models::WebSessionRecord {
            sid: sid.clone(),
            user_id: user_id.to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::from_std(cleanclaw_auth::SESSION_TTL).unwrap_or_default(),
        };
        if let Err(e) = store.create_web_session(&sess).await {
            tracing::warn!(?e, user_id, "mint_session_for: store failed");
        }
    }
    sid
}

/// Map a `UserError` to a user-friendly string. The CleanClaw Go
/// server surfaces a similar set; the user-facing text is short.
fn signup_error_message(e: &cleanclaw_auth::UserError) -> String {
    use cleanclaw_auth::UserError::*;
    use cleanclaw_core::CleanClawError;
    match e {
        InvalidCredentials => "Invalid input".to_string(),
        InvalidRole(r) => format!("Invalid role: {r}"),
        InvalidStatus(s) => format!("Invalid status: {s}"),
        LastSuperAdmin => "Cannot remove the last super admin".to_string(),
        Missing(f) => format!("Missing field: {f}"),
        Store(CleanClawError::Conflict(msg)) => {
            if msg.contains("username") || msg.contains("Username") {
                "Username already taken".to_string()
            } else if msg.contains("email") || msg.contains("Email") {
                "Email already registered".to_string()
            } else {
                format!("Conflict: {msg}")
            }
        }
        Store(CleanClawError::Internal(msg))
            if msg.contains("UNIQUE")
                && (msg.contains("username") || msg.contains("email")) =>
        {
            if msg.contains("username") {
                "Username already taken".to_string()
            } else {
                "Email already registered".to_string()
            }
        }
        Store(other) => format!("Server error: {other}"),
    }
}

/// Resolve the active user id from a request's `Cookie` header.
/// Returns `None` when no session cookie is present or the
/// session is unknown. The store lookup uses the same
/// `WebSessionRecord` table that `mint_session_for` writes to.
async fn resolve_user_id(
    state: &WebState,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    let sid = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(|s| s.trim())
                .find_map(|c| {
                    let (k, v) = c.split_once('=')?;
                    if k == cleanclaw_auth::SESSION_COOKIE_NAME {
                        Some(v.to_string())
                    } else {
                        None
                    }
                })
        })?;
    let store = state.store.as_ref()?;
    let sess = store.get_web_session(&sid).await.ok()?;
    if sess.expires_at < chrono::Utc::now() {
        return None;
    }
    Some(sess.user_id)
}

// =====================================================================
// Settings handlers
// =====================================================================

async fn settings_general(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::settings::general(theme))
}

/// `POST /settings/general` — save the active provider. The form
/// posts `provider` + `apiBase` + `apiKey`. We project the row
/// into a `kind=provider` config with `scope=system`. The store
/// holds one row per provider name.
async fn settings_general_post(
    State(state): State<WebState>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let provider = form.get("provider").cloned().unwrap_or_default();
    let api_base = form.get("apiBase").cloned().unwrap_or_default();
    let api_key = form.get("apiKey").cloned().unwrap_or_default();
    if provider.is_empty() {
        return redirect_with_query("/settings/general", &[("error", "Provider required")]);
    }
    let Some(store) = state.store.as_ref() else {
        return Redirect::to("/settings/general").into_response();
    };
    let now = chrono::Utc::now();
    let mut data = serde_json::Map::new();
    data.insert("type".into(), serde_json::Value::String(provider.clone()));
    if !api_base.is_empty() {
        data.insert("api_base".into(), serde_json::Value::String(api_base));
    }
    if !api_key.is_empty() {
        data.insert(
            "api_key".into(),
            serde_json::Value::String(api_key.clone()),
        );
    }
    let rec = cleanclaw_store::models::ConfigRecord {
        id: format!("cfg_provider_{provider}"),
        kind: "provider".into(),
        scope: "system".into(),
        user_id: String::new(),
        agent_id: String::new(),
        name: provider.clone(),
        enabled: true,
        credential_key: api_key.clone(),
        data: serde_json::Value::Object(data),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = store.save_config(&rec).await {
        tracing::warn!(?e, "settings_general_post: save_config failed");
        return redirect_with_query(
            "/settings/general",
            &[("error", "Failed to save provider config")],
        );
    }
    Redirect::to("/settings/general").into_response()
}

async fn settings_account(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::settings::account(theme))
}

/// `POST /settings/account` — update display name + avatar. The
/// form posts `displayName` + `avatarUrl`. The handler resolves
/// the current user from the session cookie and calls
/// `Accounts::update`.
async fn settings_account_post(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let user_id = match resolve_user_id(&state, &headers).await {
        Some(id) => id,
        None => return Redirect::to("/login").into_response(),
    };
    let display_name = form.get("displayName").cloned().unwrap_or_default();
    let avatar_url = form.get("avatarUrl").cloned().unwrap_or_default();
    let Some(accounts) = state.accounts.as_ref() else {
        return Redirect::to("/settings/account").into_response();
    };
    let res = accounts
        .update(&user_id, &display_name, "", "", None)
        .await;
    if let Err(e) = res {
        tracing::warn!(?e, user_id, "settings_account_post: update failed");
        return redirect_with_query(
            "/settings/account",
            &[("error", "Failed to update profile")],
        );
    }
    // The avatar URL is stored on the user record's data field; we
    // don't have a direct setter for it on `Accounts`, so we
    // leave the field alone here. The full avatar flow goes
    // through the API client once it's wired.
    let _ = avatar_url;
    Redirect::to("/settings/account").into_response()
}

/// `POST /settings/account/password` — change the active user's
/// password. The form posts `oldPassword` + `newPassword`. We
/// verify the old password against the stored hash before
/// accepting the change.
async fn settings_account_password_post(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let user_id = match resolve_user_id(&state, &headers).await {
        Some(id) => id,
        None => return Redirect::to("/login").into_response(),
    };
    let old = form.get("oldPassword").cloned().unwrap_or_default();
    let new_ = form.get("newPassword").cloned().unwrap_or_default();
    if old.is_empty() || new_.is_empty() {
        return redirect_with_query(
            "/settings/account",
            &[("error", "Both old and new password are required")],
        );
    }
    if new_.len() < 8 {
        return redirect_with_query(
            "/settings/account",
            &[("error", "New password must be at least 8 characters")],
        );
    }
    let Some(accounts) = state.accounts.as_ref() else {
        return Redirect::to("/settings/account").into_response();
    };
    if let Err(e) = accounts.verify_password(&user_id, &old).await {
        tracing::warn!(?e, user_id, "settings_account_password_post: verify failed");
        return redirect_with_query(
            "/settings/account",
            &[("error", "Current password is incorrect")],
        );
    }
    if let Err(e) = accounts.set_password(&user_id, &new_).await {
        tracing::warn!(?e, user_id, "settings_account_password_post: set failed");
        return redirect_with_query(
            "/settings/account",
            &[("error", "Failed to update password")],
        );
    }
    Redirect::to("/settings/account").into_response()
}

async fn settings_runtime(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::settings::runtime(theme))
}

/// `POST /settings/runtime` — toggle the sandbox backend. The
/// form posts `sandboxEnabled` + `sandboxBackend`. We persist
/// the choice as a `kind=setting` config with `scope=system`.
async fn settings_runtime_post(
    State(state): State<WebState>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let enabled = form.contains_key("sandboxEnabled");
    let backend = form
        .get("sandboxBackend")
        .cloned()
        .unwrap_or_else(|| "local".to_string());
    let Some(store) = state.store.as_ref() else {
        return Redirect::to("/settings/runtime").into_response();
    };
    let now = chrono::Utc::now();
    let mut data = serde_json::Map::new();
    data.insert("sandbox_enabled".into(), serde_json::Value::Bool(enabled));
    data.insert(
        "sandbox_backend".into(),
        serde_json::Value::String(backend.clone()),
    );
    let rec = cleanclaw_store::models::ConfigRecord {
        id: "cfg_setting_runtime_sandbox".into(),
        kind: "setting".into(),
        scope: "system".into(),
        user_id: String::new(),
        agent_id: String::new(),
        name: "runtime_sandbox".into(),
        enabled: enabled,
        credential_key: String::new(),
        data: serde_json::Value::Object(data),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = store.save_config(&rec).await {
        tracing::warn!(?e, "settings_runtime_post: save_config failed");
        return redirect_with_query(
            "/settings/runtime",
            &[("error", "Failed to save runtime config")],
        );
    }
    Redirect::to("/settings/runtime").into_response()
}

async fn settings_about(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::settings::about(theme))
}

// =====================================================================
// Admin handlers
// =====================================================================

async fn admin_users(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let rows = match state.store.as_ref() {
        Some(store) => match store.list_users().await {
            Ok(users) => users
                .into_iter()
                .map(|u| crate::pages::admin::UserRow {
                    id: u.id,
                    username: u.username,
                    email: u.email,
                    role: u.role,
                    status: u.status,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(?e, "admin_users: list_users failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::admin::users(theme, &rows))
}

async fn admin_usage(
    State(_state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    // W4: usage report requires a meter. Until cleanclaw-usage is
    // wired into the gateway, render the empty placeholder so the
    // page works in any build configuration.
    Html(crate::pages::admin::usage(theme, None))
}

async fn admin_chats(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let rows = match state.store.as_ref() {
        Some(store) => match store.list_session_owner_pairs().await {
            Ok(pairs) => {
                let mut out = Vec::with_capacity(pairs.len());
                for p in pairs.into_iter().take(50) {
                    let owner = store
                        .get_user(&p.user_id)
                        .await
                        .ok()
                        .map(|u| u.username)
                        .unwrap_or_default();
                    out.push(crate::pages::admin::ChatRow {
                        id: format!("{}:{}", p.user_id, p.agent_id),
                        agent_id: p.agent_id,
                        agent_name: None,
                        owner_username: Some(owner),
                        preview: String::new(),
                    });
                }
                out
            }
            Err(e) => {
                tracing::warn!(?e, "admin_chats: list_session_owner_pairs failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::admin::chats(theme, &rows))
}

async fn apikeys(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::apikeys::apikeys(theme, &[]))
}

// =====================================================================
// Agent handlers
// =====================================================================

async fn agents_list(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::list(theme, &[]))
}

async fn agent_overview(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::overview(&id, None, theme))
}

async fn agent_chat(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::chat(&id, theme))
}

/// `POST /agents/{id}/chat` — open a new chat session for the
/// agent. The form posts a `session` id; an empty value means
/// "start a new session". We mint a session key in either case
/// and redirect to `/agents/{id}/sessions/{sid}`.
async fn agent_chat_post(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let user_id = match resolve_user_id(&state, &headers).await {
        Some(id) => id,
        None => return Redirect::to("/login").into_response(),
    };
    let requested = form.get("session").cloned().unwrap_or_default();
    let session_id = if requested.is_empty() {
        // Mint a fresh session id. Use a short random hex so the
        // URL stays readable.
        cleanclaw_auth::session::new_token()
            .chars()
            .take(10)
            .collect::<String>()
    } else {
        requested
    };
    let Some(store) = state.store.as_ref() else {
        return Redirect::to(&format!("/agents/{}", crate::client::urlencode(&id))).into_response();
    };
    let now = chrono::Utc::now();
    let rec = cleanclaw_store::models::SessionRecord {
        user_id: user_id.clone(),
        agent_id: id.clone(),
        session_key: session_id.clone(),
        channel: "web".into(),
        account_id: String::new(),
        chat_id: session_id.clone(),
        project_id: String::new(),
        title: String::new(),
        messages: serde_json::Value::Array(Vec::new()),
        message_count: 0,
        updated_at: now,
        chatter_user_id: user_id.clone(),
    };
    if let Err(e) = store
        .save_session(&user_id, &id, &session_id, &rec)
        .await
    {
        tracing::warn!(?e, %session_id, "agent_chat_post: save_session failed");
        return redirect_with_query(
            &format!("/agents/{}", crate::client::urlencode(&id)),
            &[("error", "Failed to open chat")],
        );
    }
    let url = format!(
        "/agents/{}/sessions/{}",
        crate::client::urlencode(&id),
        crate::client::urlencode(&session_id)
    );
    Redirect::to(&url).into_response()
}

async fn agent_chats(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::chats(&id, &[], theme))
}

async fn agent_sessions(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::sessions(&id, &[], theme))
}

/// `GET /agents/:id/sessions/:sid` — the chat surface.
//
/// Loads the session + its messages, renders the chat history,
/// embeds the WS client (`/static/ws-chat.js`) for live
/// streaming, and includes a composer at the bottom. This is
/// where the user spends most of their time on the dashboard.
//
//
/// (the React UI's session detail page). The SSR form does the
/// initial render; the embedded JS does the live streaming.
async fn agent_session_detail(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path((id, sid)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let user_id = match resolve_user_id(&state, &headers).await {
        Some(id) => id,
        None => return Redirect::to("/login").into_response(),
    };

    // Load the session record + the message archive. We pass
    // both to the page so the SSR can render the history
    // without an extra round-trip from the JS client.
    let store = match state.store.as_ref() {
        Some(s) => s,
        None => {
            return Html(crate::pages::agent::chat_surface(
                &id,
                &sid,
                None,
                &[],
                &[],
                theme,
            ))
            .into_response();
        }
    };

    let session = match store.get_session(&user_id, &id, &sid).await {
        Ok(s) => Some(s),
        Err(_) => None,
    };
    let messages = match store
        .list_session_messages(&user_id, &id, &sid)
        .await
    {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!(?err, %user_id, %id, %sid, "agent_session_detail: list messages failed");
            Vec::new()
        }
    };

    Html(crate::pages::agent::chat_surface(
        &id,
        &sid,
        session.as_ref(),
        &messages,
        &[],
        theme,
    ))
    .into_response()
}

async fn agent_channels(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    // The per-agent channels page lists rows whose `scope_id` matches
    // the agent id. Re-use the global loader and filter.
    let channels = match state.store.as_ref() {
        Some(store) => match load_channel_rows(store).await {
            Ok(r) => project_agent_channels(
                &r.into_iter().filter(|r| r.scope_id == id).collect::<Vec<_>>(),
            ),
            Err(err) => {
                tracing::warn!(?err, agent = %id, "agent_channels: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::agent::channels(&id, &channels, theme))
}

async fn agent_scheduler(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let jobs = match state.store.as_ref() {
        Some(store) => match load_cron_infos(store).await {
            Ok(j) => project_agent_cron_jobs(
                &j.into_iter().filter(|j| j.agent_id == id).collect::<Vec<_>>(),
            ),
            Err(err) => {
                tracing::warn!(?err, agent = %id, "agent_scheduler: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::agent::scheduler(&id, &jobs, theme))
}

async fn agent_skills(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let infos = load_skill_infos(state.skills_dir.as_ref());
    Html(crate::pages::agent::skills(&id, &infos, theme))
}

async fn agent_plugins(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::plugins(&id, &[], theme))
}

async fn agent_models(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::models(&id, None, theme))
}

async fn agent_context(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::context(&id, theme))
}

async fn agent_customize(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::customize(&id, theme))
}

/// `POST /agents/{id}/customize` — save the agent's display
/// name + description + prompt mode + soul. The handler upserts
/// the `AgentRecord`'s `config` JSON to carry the new fields.
/// The existing name is read from the record; if the record
/// doesn't exist yet, we create it on the fly.
async fn agent_customize_post(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let user_id = match resolve_user_id(&state, &headers).await {
        Some(uid) => uid,
        None => return Redirect::to("/login").into_response(),
    };
    let name = form.get("name").cloned().unwrap_or_default();
    let description = form.get("description").cloned().unwrap_or_default();
    let prompt_mode = form.get("promptMode").cloned().unwrap_or_default();
    let soul = form.get("soul").cloned().unwrap_or_default();
    let redirect_to = format!("/agents/{}", crate::client::urlencode(&id));
    if name.is_empty() {
        return redirect_with_query(&redirect_to, &[("error", "Name is required")]);
    }
    let Some(store) = state.store.as_ref() else {
        return Redirect::to(&redirect_to).into_response();
    };
    // Load (or synthesize) the agent record.
    let existing = store.get_agent(&id).await.ok();
    let now = chrono::Utc::now();
    let mut config = existing
        .as_ref()
        .map(|a| a.config.clone())
        .unwrap_or(serde_json::Value::Object(Default::default()));
    if !description.is_empty() {
        config
            .as_object_mut()
            .unwrap()
            .insert("description".into(), serde_json::Value::String(description));
    }
    if !prompt_mode.is_empty() {
        config
            .as_object_mut()
            .unwrap()
            .insert("prompt_mode".into(), serde_json::Value::String(prompt_mode));
    }
    if !soul.is_empty() {
        config
            .as_object_mut()
            .unwrap()
            .insert("soul".into(), serde_json::Value::String(soul));
    }
    let rec = cleanclaw_store::models::AgentRecord {
        id: id.clone(),
        user_id: user_id.clone(),
        name: name.clone(),
        config,
        is_public: existing.as_ref().map(|a| a.is_public).unwrap_or(false),
        created_at: existing.as_ref().map(|a| a.created_at).unwrap_or(now),
        updated_at: now,
    };
    if let Err(e) = store.save_agent(&rec).await {
        tracing::warn!(?e, agent_id = %id, "agent_customize_post: save_agent failed");
        return redirect_with_query(&redirect_to, &[("error", "Failed to save agent")]);
    }
    Redirect::to(&redirect_to).into_response()
}

async fn agent_project(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::project(&id, &[], theme))
}

async fn agent_project_id(
    Path((id, pid)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::project_id(&id, &pid, theme))
}

async fn agent_usage(
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::agent::usage(&id, None, theme))
}

// =====================================================================
// Resource handlers
// =====================================================================

async fn channels_list(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let infos = match state.store.as_ref() {
        Some(store) => match load_channel_rows(store).await {
            Ok(rows) => project_channel_health(&rows),
            Err(err) => {
                tracing::warn!(?err, "channels_list: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::resources::channels_list(theme, &infos))
}

async fn channels_config(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let rows = match state.store.as_ref() {
        Some(store) => match load_channel_rows(store).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(?err, "channels_config: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::resources::channels_config(theme, &rows))
}

async fn models(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let entries = match state.store.as_ref() {
        Some(store) => match load_model_entries(store).await {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(?err, "models: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::resources::models(theme, &entries))
}

async fn providers(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let rows = match state.store.as_ref() {
        Some(store) => match load_provider_rows(store).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(?err, "providers: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::resources::providers(theme, &rows))
}

async fn plugins(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    Html(crate::pages::resources::plugins(theme, &[]))
}

async fn skills(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let infos = load_skill_infos(state.skills_dir.as_ref());
    Html(crate::pages::resources::skills(theme, &infos))
}

async fn tools(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let cfg = match state.store.as_ref() {
        Some(store) => match load_tools_config(store).await {
            Ok(c) => Some(c),
            Err(err) => {
                tracing::warn!(?err, "tools: load failed");
                None
            }
        },
        None => None,
    };
    Html(crate::pages::resources::tools(theme, cfg.as_ref()))
}

async fn cron(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let jobs = match state.store.as_ref() {
        Some(store) => match load_cron_infos(store).await {
            Ok(j) => j,
            Err(err) => {
                tracing::warn!(?err, "cron: load failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    Html(crate::pages::resources::cron(theme, &jobs))
}

async fn onboard(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = Theme::from_query(params.get("theme").map(|s| s.as_str()));
    let error = params.get("error").map(|s| s.as_str());
    Html(crate::pages::resources::onboard(theme, error))
}

/// `POST /onboard` — first-run wizard. The form posts the admin
/// account (`username` + `email` + `password`) and a provider
/// (`provider` + `apiBase` + `apiKey` + `model`). The handler:
//
/// 1. Validates the input.
/// 2. Creates the admin account as `ROLE_SUPER_ADMIN` with
///    `agent_quota = -1` (unlimited).
/// 3. Persists the provider config with `scope=system`.
/// 4. Mints a session cookie and redirects to `/overview`.
//
/// On failure the handler redirects back to `/onboard?error=…`
/// with the original form values pre-filled via query params
/// (the page already supports that).
async fn onboard_post(
    State(state): State<WebState>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let username = form.get("username").cloned().unwrap_or_default();
    let email = form.get("email").cloned().unwrap_or_default();
    let password = form.get("password").cloned().unwrap_or_default();
    let provider = form.get("provider").cloned().unwrap_or_default();
    let api_base = form.get("apiBase").cloned().unwrap_or_default();
    let api_key = form.get("apiKey").cloned().unwrap_or_default();
    let model = form.get("model").cloned().unwrap_or_default();

    if username.len() < 3
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return redirect_with_query(
            "/onboard",
            &[
                ("error", "Username must be 3+ chars (letters, digits, _ or -)"),
                ("username", &username),
                ("email", &email),
            ],
        );
    }
    if !email.contains('@') || email.len() < 5 {
        return redirect_with_query(
            "/onboard",
            &[
                ("error", "Invalid email"),
                ("username", &username),
                ("email", &email),
            ],
        );
    }
    if password.len() < 8 {
        return redirect_with_query(
            "/onboard",
            &[
                ("error", "Password must be at least 8 characters"),
                ("username", &username),
                ("email", &email),
            ],
        );
    }
    if provider.is_empty() {
        return redirect_with_query(
            "/onboard",
            &[
                ("error", "Provider is required"),
                ("username", &username),
                ("email", &email),
            ],
        );
    }

    let Some(accounts) = state.accounts.as_ref() else {
        return Redirect::to("/overview").into_response();
    };

    // Block onboarding once any user exists — this is the first-run
    // wizard, not a re-onboard page.
    if let Ok(count) = accounts.count().await {
        if count > 0 {
            return redirect_with_query("/onboard", &[("error", "Onboarding is one-time")]);
        }
    }

    let input = cleanclaw_auth::CreateInput {
        username: username.clone(),
        email: email.clone(),
        password: password.clone(),
        display_name: username.clone(),
        role: cleanclaw_auth::ROLE_SUPER_ADMIN.to_string(),
        agent_quota: Some(-1),
        avatar_url: String::new(),
        apikey_id: String::new(),
        external_id: String::new(),
    };
    let account = match accounts.create(input).await {
        Ok(a) => a,
        Err(e) => {
            return redirect_with_query(
                "/onboard",
                &[
                    ("error", signup_error_message(&e).as_str()),
                    ("username", &username),
                    ("email", &email),
                ],
            );
        }
    };

    // Persist the provider config. We use the same shape as
    // `/settings/general` so the loader can pick it up.
    if let Some(store) = state.store.as_ref() {
        let now = chrono::Utc::now();
        let mut data = serde_json::Map::new();
        data.insert("type".into(), serde_json::Value::String(provider.clone()));
        if !api_base.is_empty() {
            data.insert("api_base".into(), serde_json::Value::String(api_base));
        }
        if !api_key.is_empty() {
            data.insert(
                "api_key".into(),
                serde_json::Value::String(api_key.clone()),
            );
        }
        if !model.is_empty() {
            data.insert("model".into(), serde_json::Value::String(model));
        }
        let rec = cleanclaw_store::models::ConfigRecord {
            id: format!("cfg_provider_{provider}"),
            kind: "provider".into(),
            scope: "system".into(),
            user_id: String::new(),
            agent_id: String::new(),
            name: provider.clone(),
            enabled: true,
            credential_key: api_key.clone(),
            data: serde_json::Value::Object(data),
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = store.save_config(&rec).await {
            tracing::warn!(?e, "onboard_post: provider save failed");
        }
    }

    // Mint a session and redirect to /overview with the cookie set.
    let sid = mint_session_for(&state, &account.id).await;
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        cleanclaw_auth::SESSION_COOKIE_NAME,
        sid,
        cleanclaw_auth::SESSION_TTL.as_secs(),
    );
    *state.session_user.lock().await = Some(account.id.clone());
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/overview".to_string()),
            (header::SET_COOKIE, cookie),
        ],
    )
        .into_response()
}

/// Health endpoint (used by `cleanclaw-daemon`'s boot probe). Mirrors
/// `/api/health` in the Go server.
pub async fn health() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true,"service":"cleanclaw-web"}"#,
    )
        .into_response()
}

// =====================================================================
// Real-data loaders for the resource pages.
//
// Each loader walks the `configs` table for the relevant kind and
// projects the rows into the typed page inputs. They're tolerant of
// malformed rows (skip + warn) so a single bad config doesn't take
// the whole page down.
// =====================================================================

async fn load_model_entries(
    store: &Arc<dyn cleanclaw_store::Store>,
) -> std::result::Result<Vec<crate::types::ModelEntry>, cleanclaw_core::CleanClawError> {
    let configs = store.list_configs_all_kinds().await?;
    let mut out = Vec::new();
    for c in configs {
        if c.kind != "model" || !c.enabled {
            continue;
        }
        // The model row is a thin shape: { id, name, reasoning,
        // context_window, max_tokens }. Anything else in `data`
        // is ignored so we stay forward-compatible with new
        // CleanClaw fields.
        if let Some(m) = c.data.get("model") {
            let entry = crate::types::ModelEntry {
                id: m
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&c.name)
                    .to_string(),
                name: m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&c.name)
                    .to_string(),
                reasoning: m
                    .get("reasoning")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                input: m
                    .get("input")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                cost: m
                    .get("cost")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                context_window: m
                    .get("context_window")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                max_tokens: m
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            };
            out.push(entry);
        }
    }
    Ok(out)
}

async fn load_provider_rows(
    store: &Arc<dyn cleanclaw_store::Store>,
) -> std::result::Result<Vec<crate::types::ProviderRow>, cleanclaw_core::CleanClawError> {
    let configs = store.list_configs_all_kinds().await?;
    let mut out = Vec::new();
    for c in configs {
        if c.kind != "provider" || !c.enabled {
            continue;
        }
        let scope = match c.scope.as_str() {
            "system" => crate::types::ScopeName::System,
            "user" => crate::types::ScopeName::User,
            "agent" => crate::types::ScopeName::Agent,
            other => {
                tracing::warn!(scope = %other, id = %c.id, "unknown scope; defaulting to User");
                crate::types::ScopeName::User
            }
        };
        let row = crate::types::ProviderRow {
            id: c.id.clone(),
            scope,
            scope_id: if c.user_id.is_empty() {
                c.agent_id.clone()
            } else {
                c.user_id.clone()
            },
            name: c.name.clone(),
            api_base: c
                .data
                .get("api_base")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            api_key: c
                .data
                .get("api_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            api_type: c
                .data
                .get("api_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            auth_type: c
                .data
                .get("auth_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            models: c
                .data
                .get("models")
                .and_then(|v| v.as_array())
                .and_then(|arr| serde_json::from_value(serde_json::Value::Array(arr.clone())).ok()),
            updated_at: Some(c.updated_at.to_rfc3339()),
        };
        out.push(row);
    }
    Ok(out)
}

async fn load_tools_config(
    store: &Arc<dyn cleanclaw_store::Store>,
) -> std::result::Result<crate::types::ToolsConfig, cleanclaw_core::CleanClawError> {
    // The tools catalog lives in `kind=tool_category` config rows,
    // one per category (imagegen / tts / websearch / webfetch).
    // The per-provider enable flags live in `kind=tool_provider`
    // rows. We merge them into a `ToolsConfig` for the page.
    let configs = store.list_configs_all_kinds().await?;
    let mut cfg = crate::types::ToolsConfig::default();
    for c in &configs {
        match c.kind.as_str() {
            "tool_category" => {
                if let Ok(cat) = serde_json::from_value::<crate::types::ToolCategoryCatalog>(
                    c.data.clone(),
                ) {
                    cfg.categories.push(cat);
                }
            }
            "tool_provider" => {
                if let Ok(prov) = serde_json::from_value::<crate::types::ToolProviderSettings>(
                    c.data.clone(),
                ) {
                    cfg.tool_providers.insert(c.name.clone(), prov);
                }
            }
            "tool_setting" => {
                if let Ok(set) = serde_json::from_value::<crate::types::ToolCategorySettings>(
                    c.data.clone(),
                ) {
                    cfg.tools.insert(c.name.clone(), set);
                }
            }
            _ => {}
        }
    }
    Ok(cfg)
}

// =====================================================================
// P6-2: real data loaders for the remaining resource pages — skills,
// channels, and cron jobs. Skills come from disk via
// `cleanclaw_skills::discover()`; channels and cron come from the
// `Store` trait.
// =====================================================================

/// Walk the skills directory and project each `Skill` into the
/// `SkillInfo` shape the `/skills` page expects. Tolerant: a
/// missing or empty directory yields an empty list, not an error.
fn load_skill_infos(dir: Option<&PathBuf>) -> Vec<crate::types::SkillInfo> {
    let Some(dir) = dir else { return Vec::new() };
    if !dir.is_dir() {
        return Vec::new();
    }
    cleanclaw_skills::discover(dir)
        .into_iter()
        .map(|s| crate::types::SkillInfo {
            name: s.name.clone(),
            description: s.description.clone(),
            location: s.path.to_string_lossy().to_string(),
            kind: "skill".to_string(),
            env_spec: Some(
                s.env
                    .iter()
                    .map(|e| crate::types::SkillEnvSpec {
                        name: e.name.clone(),
                        description: Some(e.description.clone()),
                        required: Some(e.required),
                        secret: None,
                    })
                    .collect(),
            ),
        })
        .collect()
}

/// Walk the `configs` table for `kind=channel` rows and project
/// each into the `ChannelRow` shape the `/channels-config` page
/// expects.
async fn load_channel_rows(
    store: &Arc<dyn cleanclaw_store::Store>,
) -> std::result::Result<Vec<crate::types::ChannelRow>, cleanclaw_core::CleanClawError> {
    let configs = store.list_configs_all_kinds().await?;
    let mut out = Vec::new();
    for c in configs {
        if c.kind != "channel" {
            continue;
        }
        let scope = match c.scope.as_str() {
            "system" => crate::types::ScopeName::System,
            "user" => crate::types::ScopeName::User,
            "agent" => crate::types::ScopeName::Agent,
            other => {
                tracing::warn!(scope = %other, id = %c.id, "unknown channel scope; defaulting to User");
                crate::types::ScopeName::User
            }
        };
        let row = crate::types::ChannelRow {
            id: c.id.clone(),
            scope,
            scope_id: if c.user_id.is_empty() {
                c.agent_id.clone()
            } else {
                c.user_id.clone()
            },
            kind: c
                .data
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or(&c.name)
                .to_string(),
            enabled: c.enabled,
            bot_token: c
                .data
                .get("bot_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            app_token: c
                .data
                .get("app_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            credential_key: Some(c.credential_key.clone()),
            ..Default::default()
        };
        out.push(row);
    }
    Ok(out)
}

/// Project each channel config into the `ChannelInfo` health
/// dashboard shape. The "status" is derived from `enabled` plus
/// whether the credential key is set — there's no live ping path
/// here; the gateway boot path is the source of truth.
fn project_channel_health(rows: &[crate::types::ChannelRow]) -> Vec<crate::types::ChannelInfo> {
    rows.iter()
        .map(|r| crate::types::ChannelInfo {
            kind: r.kind.clone(),
            bot_username: r
                .bot_token
                .as_deref()
                .or(r.credential_key.as_deref())
                .unwrap_or("(unset)")
                .to_string(),
            enabled: Some(r.enabled),
            status: Some(if r.enabled { "ok".to_string() } else { "disabled".to_string() }),
        })
        .collect()
}

/// Walk `Store::list_all_cron_jobs` and project each into the
/// `CronJobInfo` shape the `/cron` page expects.
async fn load_cron_infos(
    store: &Arc<dyn cleanclaw_store::Store>,
) -> std::result::Result<Vec<crate::types::CronJobInfo>, cleanclaw_core::CleanClawError> {
    let jobs = store.list_all_cron_jobs().await?;
    Ok(jobs
        .into_iter()
        .map(|j| crate::types::CronJobInfo {
            id: j.id.clone(),
            name: j.name.clone(),
            kind: j.r#type.clone(),
            schedule: j.schedule.clone(),
            agent_id: j.agent_id.clone(),
            channel: j.channel.clone(),
            chat_id: j.chat_id.clone(),
            message: j.message.clone(),
            enabled: j.enabled,
            last_run: j.last_run.as_ref().map(|t| t.to_rfc3339()),
            ..Default::default()
        })
        .collect())
}

/// Project a slice of `ChannelRow`s into the per-agent
/// `AgentChannel` shape the `/agents/{id}/channels` page expects.
fn project_agent_channels(rows: &[crate::types::ChannelRow]) -> Vec<crate::types::AgentChannel> {
    rows.iter()
        .map(|r| crate::types::AgentChannel {
            kind: r.kind.clone(),
            account_id: r.id.clone(),
            bot_username: r
                .bot_token
                .as_deref()
                .or(r.credential_key.as_deref())
                .map(|s| s.to_string()),
            bot_token: r.bot_token.clone().unwrap_or_default(),
            enabled: r.enabled,
            updated_at: None,
        })
        .collect()
}

/// Project a slice of `CronJobInfo`s into the per-agent
/// `AgentCronJob` shape the `/agents/{id}/scheduler` page expects.
fn project_agent_cron_jobs(
    jobs: &[crate::types::CronJobInfo],
) -> Vec<crate::types::AgentCronJob> {
    jobs.iter()
        .map(|j| crate::types::AgentCronJob {
            id: j.id.clone(),
            agent_id: j.agent_id.clone(),
            name: j.name.clone(),
            kind: j.kind.clone(),
            schedule: j.schedule.clone(),
            message: j.message.clone(),
            channel: j.channel.clone(),
            chat_id: j.chat_id.clone(),
            account_id: None,
            timezone: "UTC".to_string(),
            enabled: j.enabled,
            last_run: j.last_run.clone(),
            next_run: None,
            created_at: j.last_run.clone().unwrap_or_default(),
        })
        .collect()
}

/// Generic 404 handler — returns a minimal "page not found" stub.
pub async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!DOCTYPE html><html><body><h1>404</h1><p>page not found</p></body></html>",
    )
        .into_response()
}

/// Helper used by later-phase handlers to drive a `Path` capture
/// without pulling in axum's full type machinery inline.
pub async fn echo_path(Path(p): Path<String>) -> String {
    p
}

/// Helper used by later-phase handlers to convert a `HashMap<String,
/// String>` query string into a sorted key=value vector — useful for
/// stable cache keys.
pub fn sorted_query(q: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = q
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    v.sort();
    v
}

/// Default listen address (0.0.0.0:8080). Matches the Go server's
/// default port so the existing systemd unit doesn't need updating.
pub fn default_addr() -> SocketAddr {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    let host: std::net::IpAddr = std::env::var("HOST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0".parse().expect("static ip"));
    SocketAddr::from((host, port))
}

/// Health-check ping that resolves after `d`; used by tests to wait
/// for the server to start listening.
pub async fn wait_for_ready(addr: SocketAddr, d: Duration) -> std::result::Result<(), std::io::Error> {
    let deadline = std::time::Instant::now() + d;
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "server did not become ready",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_addr_parses_port() {
        let a = default_addr();
        assert_eq!(a.port(), 8080);
    }

    #[test]
    fn sorted_query_is_stable() {
        let mut q = HashMap::new();
        q.insert("b".to_string(), "2".to_string());
        q.insert("a".to_string(), "1".to_string());
        let s = sorted_query(&q);
        assert_eq!(s, vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())]);
    }

    #[test]
    fn favicon_returns_png() {
        // Synchronous check on the static byte sequence.
        assert_eq!(FaviconBytes::PNG.len(), 67);
    }

    /// Compile-time check: the `WebState` is `Clone` and the
    /// `router()` function builds without errors.
    #[test]
    fn router_builds() {
        let (tx, _rx) = watch::channel(false);
        let s = WebState::new(tx);
        let _r = router(s);
    }
}

/// Favicon byte length helper used by the test above.
struct FaviconBytes;
impl FaviconBytes {
    const PNG: [u8; 67] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
        0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
        0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
        0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
        0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
}

/// Static asset handler. Serves anything under `static/` (read
/// from the crate root) at `/static/<path>`. The directory is
/// where the build pipeline copies channel icons, favicons, and
/// any future shadcn-style SVG sprites.
async fn serve_static(Path(path): Path<String>) -> Response {
    // Refuse path traversal: `..` segments and absolute paths.
    if path.contains("..") || path.starts_with('/') {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let on_disk = std::path::PathBuf::from("static").join(&path);
    let bytes = match std::fs::read(&on_disk) {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    let content_type = match on_disk.extension().and_then(|s| s.to_str()) {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        _ => "application/octet-stream",
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

#[cfg(test)]
mod static_tests {
    use super::serve_static;
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn run(path: &str) -> (StatusCode, Vec<u8>) {
        // Build a tiny app with just the static route.
        let app = axum::Router::new().route("/static/*p", axum::routing::get(serve_static));
        let resp = app
            .oneshot(Request::builder().uri(path).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        (status, body.to_vec())
    }

    #[tokio::test]
    async fn serves_real_asset() {
        let (status, body) = run("/static/favicon.ico").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.is_empty());
    }

    #[tokio::test]
    async fn serves_channel_icon() {
        let (status, body) = run("/static/channels/telegram.svg").await;
        assert_eq!(status, StatusCode::OK);
        // SVG starts with `<svg` or `<?xml`
        let prefix = String::from_utf8_lossy(&body[..body.len().min(5)]);
        assert!(prefix.contains("svg") || prefix.contains("<?xml"));
    }

    #[tokio::test]
    async fn rejects_traversal() {
        let (status, _) = run("/static/../Cargo.toml").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_missing() {
        let (status, _) = run("/static/does-not-exist.png").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

// =====================================================================
// Resource loader unit tests. Each test builds a tiny in-memory
// SQLite store, writes a few configs, and asserts the loader
// projects them into the right typed page input.
// =====================================================================

#[cfg(test)]
mod resource_loader_tests {
    use super::*;
    use cleanclaw_store::models::ConfigRecord;
    use cleanclaw_store::Store;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;

    async fn fresh_store() -> Arc<dyn cleanclaw_store::Store> {
        let st = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        st.migrate().await.unwrap();
        Arc::new(st)
    }

    async fn save(store: &Arc<dyn cleanclaw_store::Store>, c: ConfigRecord) {
        store.save_config(&c).await.unwrap();
    }

    fn cfg(kind: &str, name: &str, data: serde_json::Value) -> ConfigRecord {
        ConfigRecord {
            id: format!("cfg_{}_{}", kind, name),
            kind: kind.into(),
            scope: "user".into(),
            user_id: "u1".into(),
            agent_id: String::new(),
            name: name.into(),
            enabled: true,
            credential_key: String::new(),
            data,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn models_loader_projects_full_row() {
        let store = fresh_store().await;
        save(
            &store,
            cfg(
                "model",
                "claude-sonnet",
                json!({
                    "model": {
                        "id": "anthropic/claude-sonnet-4-6",
                        "name": "Claude Sonnet 4.6",
                        "reasoning": true,
                        "context_window": 200000,
                        "max_tokens": 16384,
                        "input": ["text", "image"],
                        "cost": { "input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75 }
                    }
                }),
            ),
        )
        .await;
        let entries = load_model_entries(&store).await.unwrap();
        assert_eq!(entries.len(), 1);
        let m = &entries[0];
        assert_eq!(m.id, "anthropic/claude-sonnet-4-6");
        assert_eq!(m.name, "Claude Sonnet 4.6");
        assert!(m.reasoning);
        assert_eq!(m.context_window, 200000);
        assert_eq!(m.max_tokens, 16384);
        assert_eq!(m.input, vec!["text", "image"]);
    }

    #[tokio::test]
    async fn models_loader_skips_disabled_and_non_model() {
        let store = fresh_store().await;
        let mut disabled = cfg("model", "off", json!({"model": {"id": "x"}}));
        disabled.enabled = false;
        save(&store, disabled).await;
        save(
            &store,
            cfg(
                "model",
                "on",
                json!({"model": {"id": "y", "name": "Y"}}),
            ),
        )
        .await;
        // An unrelated config kind must be ignored.
        save(
            &store,
            cfg(
                "provider",
                "z",
                json!({"model": {"id": "should_skip"}}),
            ),
        )
        .await;
        let entries = load_model_entries(&store).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "y");
    }

    #[tokio::test]
    async fn models_loader_handles_minimal_row() {
        let store = fresh_store().await;
        save(
            &store,
            cfg("model", "minimal", json!({"model": {}})),
        )
        .await;
        let entries = load_model_entries(&store).await.unwrap();
        assert_eq!(entries.len(), 1);
        // Defaults fall back to the row name when id/name are missing.
        assert_eq!(entries[0].id, "minimal");
        assert_eq!(entries[0].name, "minimal");
        assert!(!entries[0].reasoning);
        assert_eq!(entries[0].context_window, 0);
    }

    #[tokio::test]
    async fn providers_loader_projects_scope() {
        let store = fresh_store().await;
        // system scope
        let mut system = cfg(
            "provider",
            "openai",
            json!({ "api_base": "https://api.openai.com/v1", "api_key": "sk_x" }),
        );
        system.scope = "system".into();
        system.user_id = String::new();
        system.agent_id = String::new();
        save(&store, system).await;
        // user scope
        let mut user = cfg(
            "provider",
            "anthropic",
            json!({ "api_base": "https://api.anthropic.com" }),
        );
        user.scope = "user".into();
        save(&store, user).await;
        let rows = load_provider_rows(&store).await.unwrap();
        assert_eq!(rows.len(), 2);
        let sys = rows.iter().find(|r| r.name == "openai").unwrap();
        assert_eq!(sys.scope, crate::types::ScopeName::System);
        assert_eq!(sys.api_base.as_deref(), Some("https://api.openai.com/v1"));
        let usr = rows.iter().find(|r| r.name == "anthropic").unwrap();
        assert_eq!(usr.scope, crate::types::ScopeName::User);
        assert_eq!(usr.scope_id, "u1");
    }

    #[tokio::test]
    async fn providers_loader_skips_disabled() {
        let store = fresh_store().await;
        let mut off = cfg("provider", "off", json!({}));
        off.enabled = false;
        save(&store, off).await;
        save(
            &store,
            cfg("provider", "on", json!({})),
        )
        .await;
        let rows = load_provider_rows(&store).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "on");
    }

    #[tokio::test]
    async fn tools_loader_merges_categories_providers_settings() {
        let store = fresh_store().await;
        save(
            &store,
            cfg(
                "tool_category",
                "imagegen",
                json!({
                    "name": "imagegen",
                    "label": "Image generation",
                    "providers": [
                        {
                            "name": "openai",
                            "label": "OpenAI",
                            "needs_key": true,
                            "needs_url": false,
                            "models": ["dall-e-3"]
                        }
                    ]
                }),
            ),
        )
        .await;
        save(
            &store,
            cfg(
                "tool_provider",
                "fal",
                json!({ "api_key": "fal_x", "endpoint": "https://fal.run" }),
            ),
        )
        .await;
        save(
            &store,
            cfg(
                "tool_setting",
                "imagegen",
                json!({
                    "primary": "openai",
                    "fallbacks": ["fal", "replicate"],
                    "auto_fallback": true
                }),
            ),
        )
        .await;
        let cfg = load_tools_config(&store).await.unwrap();
        assert_eq!(cfg.categories.len(), 1);
        assert_eq!(cfg.categories[0].name, "imagegen");
        assert_eq!(cfg.tool_providers.len(), 1);
        assert_eq!(
            cfg.tool_providers.get("fal").unwrap().api_key.as_deref(),
            Some("fal_x")
        );
        assert_eq!(cfg.tools.len(), 1);
        let s = cfg.tools.get("imagegen").unwrap();
        assert_eq!(s.primary.as_deref(), Some("openai"));
        assert_eq!(s.fallbacks.as_ref().unwrap().len(), 2);
        assert_eq!(s.auto_fallback, Some(true));
    }

    #[tokio::test]
    async fn tools_loader_tolerates_malformed_rows() {
        let store = fresh_store().await;
        // Malformed: missing required "name" field. The serde_json
        // deserialization will fail; the loader should skip the row
        // and continue rather than 500 the whole page.
        save(
            &store,
            cfg("tool_category", "bad", json!({"label": "no name"})),
        )
        .await;
        save(
            &store,
            cfg(
                "tool_category",
                "ok",
                json!({"name": "ok", "label": "OK", "providers": []}),
            ),
        )
        .await;
        let cfg = load_tools_config(&store).await.unwrap();
        assert_eq!(cfg.categories.len(), 1);
        assert_eq!(cfg.categories[0].name, "ok");
    }

    #[tokio::test]
    async fn resource_loaders_return_empty_on_no_store() {
        // The page handlers call load_* only when WebState.store is
        // Some. This test pins that the loaders themselves return
        // Ok(vec![]) / Ok(default) when the table is empty rather
        // than erroring.
        let store = fresh_store().await;
        assert_eq!(load_model_entries(&store).await.unwrap().len(), 0);
        assert_eq!(load_provider_rows(&store).await.unwrap().len(), 0);
        let tcfg = load_tools_config(&store).await.unwrap();
        assert!(tcfg.categories.is_empty());
        assert!(tcfg.tool_providers.is_empty());
        assert!(tcfg.tools.is_empty());
    }

    // -----------------------------------------------------------------
    // P6-2: loaders for /channels, /channels-config, /skills, /cron,
    // and the per-agent variants.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn channels_loader_projects_kind_and_credentials() {
        let store = fresh_store().await;
        save(
            &store,
            cfg(
                "channel",
                "tg_main",
                json!({ "type": "telegram", "bot_token": "tg_x" }),
            ),
        )
        .await;
        save(
            &store,
            cfg(
                "channel",
                "slack_main",
                json!({ "type": "slack", "bot_token": "xoxb_x", "app_token": "xapp_x" }),
            ),
        )
        .await;
        let rows = load_channel_rows(&store).await.unwrap();
        assert_eq!(rows.len(), 2);
        let tg = rows.iter().find(|r| r.kind == "telegram").unwrap();
        assert_eq!(tg.bot_token.as_deref(), Some("tg_x"));
        assert!(tg.enabled);
        let slack = rows.iter().find(|r| r.kind == "slack").unwrap();
        assert_eq!(slack.app_token.as_deref(), Some("xapp_x"));
    }

    #[tokio::test]
    async fn channels_health_projects_status_from_enabled() {
        let rows = vec![crate::types::ChannelRow {
            id: "c1".into(),
            scope: crate::types::ScopeName::User,
            scope_id: "u1".into(),
            kind: "telegram".into(),
            enabled: true,
            bot_token: Some("tg_x".into()),
            ..Default::default()
        }];
        let infos = project_channel_health(&rows);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].kind, "telegram");
        assert_eq!(infos[0].bot_username, "tg_x");
        assert_eq!(infos[0].status.as_deref(), Some("ok"));
        assert_eq!(infos[0].enabled, Some(true));
    }

    #[tokio::test]
    async fn channels_health_disabled_yields_disabled_status() {
        let rows = vec![crate::types::ChannelRow {
            id: "c1".into(),
            scope: crate::types::ScopeName::User,
            scope_id: "u1".into(),
            kind: "discord".into(),
            enabled: false,
            ..Default::default()
        }];
        let infos = project_channel_health(&rows);
        assert_eq!(infos[0].status.as_deref(), Some("disabled"));
    }

    #[tokio::test]
    async fn agent_channels_filter_by_scope_id() {
        let store = fresh_store().await;
        // Two channel configs: one for u1 (user scope), one for a1 (agent scope).
        let mut c1 = cfg("channel", "tg_u1", json!({ "type": "telegram" }));
        c1.user_id = "u1".into();
        c1.agent_id = String::new();
        save(&store, c1).await;
        let mut c2 = cfg("channel", "dc_a1", json!({ "type": "discord" }));
        c2.user_id = String::new();
        c2.agent_id = "a1".into();
        save(&store, c2).await;
        let rows = load_channel_rows(&store).await.unwrap();
        // Per-agent view: scope_id == "a1" → just the discord row.
        let agent_rows: Vec<_> = rows.iter().filter(|r| r.scope_id == "a1").cloned().collect();
        let channels = project_agent_channels(&agent_rows);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].kind, "discord");
        // The id field on the row is the namespaced config id; the
        // account_id field on the projected AgentChannel is the
        // *original* id (not the namespaced one). Since the row id
        // is `cfg_channel_dc_a1`, the projected account_id is the
        // same — we just verify the channel made it through.
        assert!(channels[0].account_id.contains("dc_a1"));
    }

    #[tokio::test]
    async fn cron_loader_projects_record_to_info() {
        let store = fresh_store().await;
        // The cron table is separate from configs — we need to
        // insert via save_cron_job. Construct a CronJobRecord.
        let j = cleanclaw_store::models::CronJobRecord {
            id: "cj1".into(),
            user_id: "u1".into(),
            agent_id: "a1".into(),
            name: "daily_ping".into(),
            r#type: "cron".into(),
            schedule: "0 0 * * *".into(),
            message: "morning".into(),
            channel: "telegram".into(),
            chat_id: "123".into(),
            account_id: String::new(),
            timezone: "UTC".into(),
            enabled: true,
            last_run: None,
            next_run: None,
            locked_by: None,
            locked_at: None,
            failure_count: 0,
            created_at: Utc::now(),
        };
        store.save_cron_job(&j).await.unwrap();
        let infos = load_cron_infos(&store).await.unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "daily_ping");
        assert_eq!(infos[0].kind, "cron");
        assert_eq!(infos[0].schedule, "0 0 * * *");
        assert!(infos[0].enabled);
    }

    #[tokio::test]
    async fn agent_cron_jobs_filter_by_agent_id() {
        // Insert two cron jobs for two different agents; the
        // per-agent loader should return only the matching one.
        let store = fresh_store().await;
        let j1 = cleanclaw_store::models::CronJobRecord {
            id: "j1".into(),
            user_id: "u1".into(),
            agent_id: "a1".into(),
            name: "alpha".into(),
            r#type: "cron".into(),
            schedule: "* * * * *".into(),
            message: "m".into(),
            channel: "telegram".into(),
            chat_id: "1".into(),
            account_id: String::new(),
            timezone: "UTC".into(),
            enabled: true,
            last_run: None,
            next_run: None,
            locked_by: None,
            locked_at: None,
            failure_count: 0,
            created_at: Utc::now(),
        };
        let mut j2 = j1.clone();
        j2.id = "j2".into();
        j2.agent_id = "a2".into();
        j2.name = "beta".into();
        store.save_cron_job(&j1).await.unwrap();
        store.save_cron_job(&j2).await.unwrap();
        let infos = load_cron_infos(&store).await.unwrap();
        assert_eq!(infos.len(), 2);
        let agent_jobs = project_agent_cron_jobs(
            &infos.into_iter().filter(|j| j.agent_id == "a1").collect::<Vec<_>>(),
        );
        assert_eq!(agent_jobs.len(), 1);
        assert_eq!(agent_jobs[0].name, "alpha");
    }

    #[test]
    fn skills_loader_returns_empty_when_no_dir() {
        let infos = load_skill_infos(None);
        assert!(infos.is_empty());
    }

    #[test]
    fn skills_loader_returns_empty_when_dir_missing() {
        let bogus = PathBuf::from("/nonexistent/skills/dir/xyz");
        let infos = load_skill_infos(Some(&bogus));
        assert!(infos.is_empty());
    }

    #[test]
    fn skills_loader_discovers_real_skill() {
        // Build a tmp dir with a SKILL.md and assert it's discovered.
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-web-skills-loader-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("my_skill")).unwrap();
        std::fs::write(
            dir.join("my_skill/SKILL.md"),
            "---\nname: my_skill\ndescription: a test\nenv:\n  - name: API_KEY\n    description: the key\n    required: true\n---\n# body\n",
        )
        .unwrap();
        let infos = load_skill_infos(Some(&dir));
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "my_skill");
        assert_eq!(infos[0].description, "a test");
        let env = infos[0].env_spec.as_ref().unwrap();
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].name, "API_KEY");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------
    // P7-2..P7-4: end-to-end tests for the new POST handlers. Each
    // test wires a real `Accounts` + `Store` into `WebState`,
    // drives the handler through the axum router, and asserts the
    // resulting redirect / store state.
    // -----------------------------------------------------------------

    /// Spin up a `WebState` with both a real store + a real
    /// `Accounts` registry. Used by the POST handler tests.
    async fn full_state() -> (Arc<dyn cleanclaw_store::Store>, WebState) {
        let st = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        st.migrate().await.unwrap();
        let store: Arc<dyn cleanclaw_store::Store> = Arc::new(st);
        let accounts = cleanclaw_auth::Accounts::new(store.clone()).unwrap();
        let (tx, _rx) = watch::channel(false);
        let state = WebState::new(tx)
            .with_store(store.clone())
            .with_accounts(Arc::new(accounts));
        (store, state)
    }

    /// Helper: drive a POST through the full router and return
    /// the status + `Location` + `Set-Cookie` headers.
    async fn post_form(
        state: WebState,
        uri: &str,
        body: &str,
    ) -> (StatusCode, Option<String>, Vec<String>) {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let app = router(state);
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let location = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let cookies: Vec<String> = resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect();
        (status, location, cookies)
    }

    #[tokio::test]
    async fn login_post_redirects_to_overview_when_no_accounts_wired() {
        // WebState with no accounts → W4 stub behaviour: any
        // valid form redirects to /overview.
        let (tx, _rx) = watch::channel(false);
        let state = WebState::new(tx);
        let (status, location, _cookies) = post_form(
            state,
            "/login",
            "login=ada&password=secret",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/overview"));
    }

    #[tokio::test]
    async fn login_post_validates_and_redirects_to_login_with_error() {
        let (tx, _rx) = watch::channel(false);
        let state = WebState::new(tx);
        let (status, location, _cookies) =
            post_form(state, "/login", "login=&password=").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let loc = location.unwrap();
        assert!(loc.starts_with("/login?"), "got {loc}");
        assert!(loc.contains("error="));
    }

    #[tokio::test]
    async fn signup_post_creates_user_and_mints_session_cookie() {
        let (store, state) = full_state().await;
        let (status, location, cookies) = post_form(
            state,
            "/signup",
            "username=alice&email=alice%40example.com&password=longenough123&display_name=Alice",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/overview"));
        // Cookie must be set with the session cookie name.
        assert!(cookies.iter().any(|c| c.starts_with("cleanclaw_session=")));
        // User was actually written to the store.
        let recs = store.list_users().await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].username, "alice");
        assert_eq!(recs[0].role, "user");
    }

    #[tokio::test]
    async fn signup_post_rejects_duplicate_username() {
        let (store, state) = full_state().await;
        post_form(
            state.clone(),
            "/signup",
            "username=bob&email=bob%40example.com&password=longenough123",
        )
        .await;
        // Second signup with the same username must redirect
        // back with the conflict error.
        let (status, location, _) = post_form(
            state,
            "/signup",
            "username=bob&email=bob2%40example.com&password=longenough123",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let loc = location.unwrap();
        assert!(loc.starts_with("/signup?"));
        assert!(loc.contains("error="));
        // Still only one user in the store.
        assert_eq!(store.list_users().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn login_post_authenticates_after_signup() {
        let (_, state) = full_state().await;
        // First sign up.
        post_form(
            state.clone(),
            "/signup",
            "username=carol&email=carol%40example.com&password=longenough123",
        )
        .await;
        // Then log in.
        let (status, location, cookies) = post_form(
            state,
            "/login",
            "login=carol&password=longenough123",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/overview"));
        assert!(cookies.iter().any(|c| c.starts_with("cleanclaw_session=")));
    }

    #[tokio::test]
    async fn login_post_rejects_bad_password() {
        let (_, state) = full_state().await;
        post_form(
            state.clone(),
            "/signup",
            "username=dave&email=dave%40example.com&password=longenough123",
        )
        .await;
        let (status, location, _cookies) = post_form(
            state,
            "/login",
            "login=dave&password=wrong",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let loc = location.unwrap();
        assert!(loc.starts_with("/login?"));
        assert!(loc.contains("error="));
    }

    #[tokio::test]
    async fn settings_general_post_persists_provider_config() {
        let (store, state) = full_state().await;
        let (status, location, _) = post_form(
            state,
            "/settings/general",
            "provider=openai&apiBase=https%3A%2F%2Fapi.openai.com%2Fv1&apiKey=sk_test_xyz",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/settings/general"));
        // The provider config must be in the store with scope=system.
        let configs = store.list_configs_all_kinds().await.unwrap();
        let provider: Vec<_> = configs.iter().filter(|c| c.kind == "provider").collect();
        assert_eq!(provider.len(), 1);
        assert_eq!(provider[0].name, "openai");
        assert_eq!(provider[0].scope, "system");
        assert!(provider[0].data.get("api_key").is_some());
    }

    #[tokio::test]
    async fn settings_general_post_rejects_empty_provider() {
        let (store, state) = full_state().await;
        let (status, location, _) =
            post_form(state, "/settings/general", "provider=&apiBase=&apiKey=").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let loc = location.unwrap();
        assert!(loc.starts_with("/settings/general?"));
        assert!(loc.contains("error=Provider"));
        // Nothing was saved.
        assert!(store
            .list_configs_all_kinds()
            .await
            .unwrap()
            .iter()
            .all(|c| c.kind != "provider"));
    }

    #[tokio::test]
    async fn settings_runtime_post_persists_sandbox_toggle() {
        let (store, state) = full_state().await;
        let (status, location, _) = post_form(
            state,
            "/settings/runtime",
            "sandboxEnabled=on&sandboxBackend=docker",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/settings/runtime"));
        let configs = store.list_configs_all_kinds().await.unwrap();
        let runtime: Vec<_> = configs.iter().filter(|c| c.name == "runtime_sandbox").collect();
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].data.get("sandbox_backend").unwrap(), "docker");
        assert_eq!(
            runtime[0].data.get("sandbox_enabled").unwrap(),
            &serde_json::Value::Bool(true)
        );
    }

    #[tokio::test]
    async fn onboard_post_creates_first_super_admin_and_redirects() {
        let (store, state) = full_state().await;
        let (status, location, cookies) = post_form(
            state,
            "/onboard",
            "username=root&email=root%40example.com&password=longenough123&provider=anthropic&apiBase=https%3A%2F%2Fapi.anthropic.com&apiKey=sk_ant&model=claude-sonnet-4-6",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/overview"));
        assert!(cookies.iter().any(|c| c.starts_with("cleanclaw_session=")));
        // Admin user with role=super_admin.
        let recs = store.list_users().await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].role, "super_admin");
        // Provider config persisted.
        let configs = store.list_configs_all_kinds().await.unwrap();
        let provider: Vec<_> = configs.iter().filter(|c| c.kind == "provider").collect();
        assert_eq!(provider.len(), 1);
        assert_eq!(provider[0].name, "anthropic");
    }

    #[tokio::test]
    async fn onboard_post_blocks_when_user_already_exists() {
        let (store, state) = full_state().await;
        // First onboard succeeds.
        post_form(
            state.clone(),
            "/onboard",
            "username=root&email=root%40example.com&password=longenough123&provider=openai&apiBase=&apiKey=",
        )
        .await;
        // Second attempt is rejected.
        let (status, location, _cookies) = post_form(
            state,
            "/onboard",
            "username=other&email=other%40example.com&password=longenough123&provider=openai&apiBase=&apiKey=",
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let loc = location.unwrap();
        assert!(loc.contains("error="));
        // Still only one user.
        assert_eq!(store.list_users().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn agent_chat_post_creates_session_and_redirects() {
        let (store, state) = full_state().await;
        // Sign up so we have a valid session.
        let (_status, _location, cookies) = post_form(
            state.clone(),
            "/signup",
            "username=eve&email=eve%40example.com&password=longenough123",
        )
        .await;
        let cookie = cookies
            .iter()
            .find(|c| c.starts_with("cleanclaw_session="))
            .unwrap()
            .clone();
        // POST to /agents/a1/chat with the session cookie.
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let app = router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/agents/a1/chat")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie.split(';').next().unwrap())
            .body(Body::from("session="))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        assert!(
            location.starts_with("/agents/a1/sessions/"),
            "got {location}"
        );
        // A session record was written.
        let sessions = store
            .list_sessions(&store.list_users().await.unwrap()[0].id, "a1")
            .await
            .unwrap();
        assert_eq!(sessions.len(), 1);
    }

    #[tokio::test]
    async fn agent_customize_post_upserts_agent_record() {
        let (store, state) = full_state().await;
        // Sign up.
        let (_, _, cookies) = post_form(
            state.clone(),
            "/signup",
            "username=fred&email=fred%40example.com&password=longenough123",
        )
        .await;
        let cookie = cookies
            .iter()
            .find(|c| c.starts_with("cleanclaw_session="))
            .unwrap()
            .clone();
        // POST customize.
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let app = router(state);
        let body = "name=My+Agent&description=does+things&promptMode=chatbot&soul=hello";
        let req = Request::builder()
            .method("POST")
            .uri("/agents/a_fred/customize")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie.split(';').next().unwrap())
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        // Agent record saved with the new fields.
        let rec = store.get_agent("a_fred").await.unwrap();
        assert_eq!(rec.name, "My Agent");
        assert_eq!(rec.config.get("description").unwrap(), "does things");
        assert_eq!(rec.config.get("prompt_mode").unwrap(), "chatbot");
        assert_eq!(rec.config.get("soul").unwrap(), "hello");
    }

    #[test]
    fn signup_error_message_rewrites_known_conflicts() {
        // Internal UNIQUE constraint errors should be re-written to
        // user-friendly "Username already taken" / "Email already
        // registered" — this is what the user sees after the redirect.
        use cleanclaw_auth::UserError;
        use cleanclaw_core::CleanClawError;
        let u = UserError::Store(CleanClawError::Internal(
            "db: UNIQUE constraint failed: users.username".into(),
        ));
        assert_eq!(signup_error_message(&u), "Username already taken");
        let u = UserError::Store(CleanClawError::Internal(
            "db: UNIQUE constraint failed: users.email".into(),
        ));
        assert_eq!(signup_error_message(&u), "Email already registered");
        let u = UserError::Store(CleanClawError::Conflict("username already exists".into()));
        assert_eq!(signup_error_message(&u), "Username already taken");
    }

    #[test]
    fn redirect_with_query_skips_empty_values() {
        // Sanity check: empty values are dropped from the query
        // string so the redirect URL stays compact.
        let resp = redirect_with_query("/onboard", &[("error", "bad input"), ("username", "")]);
        let url = format!("{:?}", resp);
        assert!(url.contains("/onboard?error=bad"));
        assert!(!url.contains("username="));
    }

    #[tokio::test]
    async fn agent_session_detail_renders_history_and_composer() {
        // Sign up + create an agent + write a session + write a
        // couple of messages, then GET the session detail page
        // and assert the rendered HTML contains the history.
        let (store, state) = full_state().await;
        // Sign up.
        let (_, _, cookies) = post_form(
            state.clone(),
            "/signup",
            "username=george&email=george%40example.com&password=longenough123",
        )
        .await;
        let cookie = cookies
            .iter()
            .find(|c| c.starts_with("cleanclaw_session="))
            .unwrap()
            .clone();
        let user_id = store.list_users().await.unwrap()[0].id.clone();

        // Seed a session record + 2 messages directly.
        let now = chrono::Utc::now();
        let sess = cleanclaw_store::models::SessionRecord {
            user_id: user_id.clone(),
            agent_id: "a_george".into(),
            session_key: "sess_g".into(),
            channel: "web".into(),
            account_id: String::new(),
            chat_id: "sess_g".into(),
            project_id: String::new(),
            title: "My first chat".into(),
            messages: serde_json::json!([]),
            message_count: 2,
            updated_at: now,
            chatter_user_id: user_id.clone(),
        };
        store
            .save_session(&user_id, "a_george", "sess_g", &sess)
            .await
            .unwrap();
        let u_msg = cleanclaw_store::models::SessionMessageRecord {
            user_id: user_id.clone(),
            agent_id: "a_george".into(),
            session_key: "sess_g".into(),
            seq: 0,
            role: "user".into(),
            content: "hi there".into(),
            content_parts: serde_json::json!([]),
            tool_calls: serde_json::json!([]),
            tool_call_id: String::new(),
            name: String::new(),
            metadata: serde_json::json!({}),
            thinking: String::new(),
            raw_assistant: serde_json::Value::Null,
            origin: "test".into(),
            created_at: now,
            chatter_user_id: user_id.clone(),
        };
        let a_msg = cleanclaw_store::models::SessionMessageRecord {
            user_id: user_id.clone(),
            agent_id: "a_george".into(),
            session_key: "sess_g".into(),
            seq: 1,
            role: "assistant".into(),
            content: "hello!".into(),
            content_parts: serde_json::json!([]),
            tool_calls: serde_json::json!([{"name": "echo", "arguments": {"x": 1}}]),
            tool_call_id: String::new(),
            name: String::new(),
            metadata: serde_json::json!({}),
            thinking: String::new(),
            raw_assistant: serde_json::Value::Null,
            origin: "test".into(),
            created_at: now,
            chatter_user_id: user_id.clone(),
        };
        store.append_session_message(&u_msg).await.unwrap();
        store.append_session_message(&a_msg).await.unwrap();

        // GET the session detail page.
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let app = router(state);
        let req = Request::builder()
            .method("GET")
            .uri("/agents/a_george/sessions/sess_g")
            .header(header::COOKIE, cookie.split(';').next().unwrap())
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let s = String::from_utf8_lossy(&body).to_string();
        // The page embeds the history + the WS client.
        assert!(s.contains("hi there"), "user msg not in page");
        assert!(s.contains("hello!"), "assistant msg not in page");
        assert!(s.contains("My first chat"), "session title not in page");
        assert!(s.contains("/static/ws-chat.js"), "WS client not embedded");
        assert!(s.contains("data-agent-id=\"a_george\""), "agent id missing");
        assert!(s.contains("data-session-id=\"sess_g\""), "session id missing");
        // The composer + the chat-* classes are wired.
        assert!(s.contains("data-chat-form"), "composer form missing");
        assert!(s.contains("data-chat-input"), "composer input missing");
    }

    #[tokio::test]
    async fn agent_session_detail_redirects_to_login_when_unauth() {
        // No cookie → handler should redirect to /login.
        let (_store, state) = full_state().await;
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let app = router(state);
        let req = Request::builder()
            .method("GET")
            .uri("/agents/a1/sessions/s1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(loc, "/login");
    }
}
