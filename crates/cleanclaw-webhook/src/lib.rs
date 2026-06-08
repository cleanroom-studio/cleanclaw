//! Webhook HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use cleanclaw_bus::InboundMessage;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

/// Agent-facing handler invoked for each authenticated webhook. Mirrors
/// the Go `AgentHandler` interface.
#[async_trait::async_trait]
pub trait AgentHandler: Send + Sync + 'static {
    async fn handle_message(
        &self,
        agent_id: &str,
        msg: InboundMessage,
    ) -> Result<String, WebhookError>;
}

#[async_trait::async_trait]
impl<F, Fut> AgentHandler for F
where
    F: Fn(String, InboundMessage) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, WebhookError>> + Send,
{
    async fn handle_message(
        &self,
        agent_id: &str,
        msg: InboundMessage,
    ) -> Result<String, WebhookError> {
        (self)(agent_id.to_string(), msg).await
    }
}

/// Resolves a bearer token to a user ID (cloud mode). Optional.
#[async_trait::async_trait]
pub trait UserLookup: Send + Sync + 'static {
    async fn lookup_by_token(&self, token: &str) -> Option<String>;
}

#[async_trait::async_trait]
impl<F, Fut> UserLookup for F
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Option<String>> + Send,
{
    async fn lookup_by_token(&self, token: &str) -> Option<String> {
        (self)(token.to_string()).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("handler error: {0}")]
    Handler(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookRequest {
    #[serde(rename = "agentId", default)]
    pub agent_id: String,
    #[serde(rename = "userId", default)]
    pub user_id: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub channel: String,
    #[serde(rename = "chatId", default)]
    pub chat_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

impl WebhookResponse {
    pub fn ok(reply: impl Into<String>) -> Self {
        Self {
            ok: true,
            reply: Some(reply.into()),
            error: None,
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            reply: None,
            error: Some(msg.into()),
        }
    }
}

struct ServerState {
    token: String,
    handler: Arc<dyn AgentHandler>,
    user_lookup: Option<Arc<dyn UserLookup>>,
}

#[derive(Clone)]
pub struct Server {
    path: String,
    state: Arc<ServerState>,
}

impl Server {
    pub fn new(
        token: impl Into<String>,
        path: impl Into<String>,
        handler: Arc<dyn AgentHandler>,
    ) -> Self {
        Self::with_user_lookup(token, path, handler, None)
    }

    pub fn with_user_lookup(
        token: impl Into<String>,
        path: impl Into<String>,
        handler: Arc<dyn AgentHandler>,
        user_lookup: Option<Arc<dyn UserLookup>>,
    ) -> Self {
        let path = path.into();
        let path = if path.is_empty() {
            "/hooks".to_string()
        } else {
            path
        };
        Self {
            path,
            state: Arc::new(ServerState {
                token: token.into(),
                handler,
                user_lookup,
            }),
        }
    }

    /// Replace the agent handler (used by gateway.New to break a
    /// chicken-and-egg cycle between gateway and webhook).
    pub fn set_handler(&mut self, handler: Arc<dyn AgentHandler>) {
        // ServerState is in an Arc; rebuild with a new state to swap.
        let new_state = ServerState {
            token: self.state.token.clone(),
            handler,
            user_lookup: self.state.user_lookup.clone(),
        };
        self.state = Arc::new(new_state);
    }

    /// axum Router mounted at the configured path. Use this in your
    /// top-level axum app via `Router::nest` or by calling
    /// `axum::serve` directly.
    pub fn router(&self) -> Router {
        let path = self.path.clone();
        let state = self.state.clone();
        Router::new()
            .route(&path, post(handle_webhook))
            .with_state(state)
    }

    /// Bind and serve on the given address. Blocks until ctx is
    /// cancelled or the server crashes. Returns Ok(()) on graceful
    /// shutdown, error otherwise.
    pub async fn listen_and_serve(&self, addr: SocketAddr) -> Result<(), std::io::Error> {
        let app = self.router();
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!(addr = %addr, path = %self.path, "webhook server started");
        axum::serve(listener, app).await
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

async fn handle_webhook(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(req): Json<WebhookRequest>,
) -> impl IntoResponse {
    // Token validation. If `state.token` is empty, the server is open
    // (matches Go's "no token configured = no auth required" mode).
    let mut owner_user_id = String::new();
    if !state.token.is_empty() {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or("").trim();
        if token == state.token {
            // Admin / local-mode token matches; leave owner_user_id empty.
        } else if let Some(lookup) = &state.user_lookup {
            match lookup.lookup_by_token(token).await {
                Some(uid) => owner_user_id = uid,
                None => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(WebhookResponse::err("unauthorized")),
                    )
                }
            }
        } else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(WebhookResponse::err("unauthorized")),
            );
        }
    }

    if req.agent_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(WebhookResponse::err("agentId is required")),
        );
    }
    if req.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(WebhookResponse::err("message is required")),
        );
    }

    let channel = if req.channel.is_empty() {
        "webhook".to_string()
    } else {
        req.channel
    };
    let chat_id = if req.chat_id.is_empty() {
        "webhook-default".to_string()
    } else {
        req.chat_id
    };

    // Prefer explicit userId in body, then token-derived.
    if !req.user_id.is_empty() {
        owner_user_id = req.user_id.clone();
    }

    let msg = InboundMessage {
        channel: channel.clone(),
        account_id: String::new(),
        chat_id: chat_id.clone(),
        project_id: String::new(),
        user_id: "webhook".to_string(),
        owner_user_id: owner_user_id.clone(),
        agent_id: req.agent_id.clone(),
        message_id: format!("webhook:{}", chrono_now()),
        text: req.message.clone(),
        peer_kind: "dm".to_string(),
        sender_name: "webhook".to_string(),
        sender_avatar_url: String::new(),
        mentions: vec![],
        is_bot_message: false,
        photo_url: String::new(),
        photo_urls: vec![],
        reply_to_msg_id: String::new(),
        params: Default::default(),
        source: String::new(),
    };

    info!(
        agent = %req.agent_id,
        channel = %channel,
        chat_id = %chat_id,
        "webhook received"
    );

    match state.handler.handle_message(&req.agent_id, msg).await {
        Ok(reply) => (StatusCode::OK, Json(WebhookResponse::ok(reply))),
        Err(e) => {
            error!(agent = %req.agent_id, error = %e, "webhook handler error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(WebhookResponse::err(e.to_string())),
            )
        }
    }
}

fn chrono_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as AxStatus};
    use serde_json::json;
    use tower::ServiceExt;

    fn ok_handler() -> Arc<dyn AgentHandler> {
        Arc::new(|_agent: String, msg: InboundMessage| async move {
            Ok::<String, WebhookError>(format!("echo:{}", msg.text))
        })
    }

    fn err_handler() -> Arc<dyn AgentHandler> {
        Arc::new(|_: String, _: InboundMessage| async move {
            Err::<String, _>(WebhookError::Handler("boom".into()))
        })
    }

    fn failing_lookup() -> Arc<dyn UserLookup> {
        Arc::new(|_token: String| async move { None::<String> })
    }

    fn granting_lookup(uid: &str) -> Arc<dyn UserLookup> {
        let uid = uid.to_string();
        Arc::new(move |_token: String| {
            let uid = uid.clone();
            async move { Some(uid) }
        })
    }

    #[tokio::test]
    async fn post_returns_echo_with_token() {
        let server = Server::new("secret123", "/hooks", ok_handler());
        let app = server.router();
        let body = json!({
            "agentId": "a1",
            "message": "hello",
            "channel": "custom",
            "chatId": "c42"
        })
        .to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer secret123")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: WebhookResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(json.ok);
        assert_eq!(json.reply.as_deref(), Some("echo:hello"));
    }

    #[tokio::test]
    async fn missing_token_returns_unauthorized() {
        let server = Server::new("secret123", "/hooks", ok_handler());
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bearer_token_via_user_lookup() {
        let server = Server::with_user_lookup(
            "admin-tok",
            "/hooks",
            ok_handler(),
            Some(granting_lookup("user-1")),
        );
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer user-token")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: WebhookResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json.reply.as_deref(), Some("echo:hi"));
    }

    #[tokio::test]
    async fn invalid_token_returns_unauthorized() {
        let server =
            Server::with_user_lookup("admin-tok", "/hooks", ok_handler(), Some(failing_lookup()));
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer wrong")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_agent_id_returns_400() {
        let server = Server::new("tok", "/hooks", ok_handler());
        let app = server.router();
        let body = json!({"message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer tok")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::BAD_REQUEST);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: WebhookResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(!json.ok);
        assert!(json.error.unwrap().contains("agentId"));
    }

    #[tokio::test]
    async fn missing_message_returns_400() {
        let server = Server::new("tok", "/hooks", ok_handler());
        let app = server.router();
        let body = json!({"agentId": "a1"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer tok")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_error_returns_500() {
        let server = Server::new("tok", "/hooks", err_handler());
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer tok")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn explicit_user_id_overrides_token_user() {
        let server = Server::with_user_lookup(
            "admin-tok",
            "/hooks",
            ok_handler(),
            Some(granting_lookup("from-token")),
        );
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi", "userId": "from-body"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer user-token")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
    }

    #[tokio::test]
    async fn open_mode_when_token_blank() {
        // No token configured = no auth required.
        let server = Server::new("", "/hooks", ok_handler());
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
    }

    #[tokio::test]
    async fn default_channel_and_chat_id() {
        let server = Server::new("tok", "/hooks", ok_handler());
        let app = server.router();
        let body = json!({"agentId": "a1", "message": "hi"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/hooks")
            .header("authorization", "Bearer tok")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        // We don't have direct access to msg, but the 200 + echo:hi
        // confirms the defaults don't blow up.
    }
}
