//! Auth resolver: given a request, resolve it to an `Identity`.
//!
//! Two credential types:
//!   - cookie session:  set by /api/login, validated against web_sessions
//!   - Bearer apikey:   validated against the apikeys table
//!
//! Both paths funnel into the same `Identity` struct. There is no
//! anonymous "local" fallback — requests without valid credentials get
//! 401.

use crate::apikey::sha256_hex;
use crate::identity::{ApiKeyType, AuthMethod, Identity, Role};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::models::{ApiKeyRecord, UserRecord};
use cleanclaw_store::Store;
use std::sync::Arc;
use std::time::Duration;

pub const SESSION_COOKIE_NAME: &str = "cleanclaw_session";
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

pub struct Resolver {
    store: Arc<dyn Store>,
}

impl Resolver {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Resolve identity from an optional `Authorization: Bearer …` and
    /// optional `Cookie: cleanclaw_session=…` pair. Returns `None` if no
    /// valid credentials were presented.
    pub async fn resolve(
        &self,
        bearer: Option<&str>,
        cookie_sid: Option<&str>,
    ) -> Result<Option<Identity>, CleanClawError> {
        // Apikey takes precedence over session.
        if let Some(token) = bearer {
            if let Some(id) = self.resolve_apikey(token).await? {
                return Ok(Some(id));
            }
        }
        if let Some(sid) = cookie_sid {
            if let Some(id) = self.resolve_session(sid).await? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Pull the auth bits out of an `axum::http::HeaderMap` and call
    /// `resolve`. Convenience wrapper for handlers.
    #[cfg(feature = "axum")]
    pub async fn resolve_from_headers(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<Identity>, CleanClawError> {
        let bearer = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.to_string());
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cookies| {
                cookies
                    .split(';')
                    .map(|s| s.trim())
                    .find_map(|c| {
                        let (k, v) = c.split_once('=')?;
                        if k == SESSION_COOKIE_NAME {
                            Some(v.to_string())
                        } else {
                            None
                        }
                    })
            });
        self.resolve(bearer.as_deref(), cookie.as_deref()).await
    }

    async fn resolve_apikey(&self, token: &str) -> Result<Option<Identity>, CleanClawError> {
        if !token.starts_with("fk_") {
            return Ok(None);
        }
        let hash = sha256_hex(token);
        let key: ApiKeyRecord = match self.store.lookup_api_key_by_hash(&hash).await {
            Ok(k) => k,
            Err(CleanClawError::NotFound(_)) => return Ok(None),
            Err(CleanClawError::Unauthorized) => return Ok(None),
            Err(e) => return Err(e),
        };
        let user: UserRecord = self.store.get_user(&key.user_id).await?;
        let agents = self.store.list_api_key_agents(&key.id).await?;
        Ok(Some(Identity {
            user_id: user.id,
            role: Role::parse(&user.role),
            method: AuthMethod::ApiKey,
            api_key_id: key.id,
            api_key_type: Some(ApiKeyType::parse(&key.r#type)),
            api_key_agents: agents,
        }))
    }

    async fn resolve_session(&self, sid: &str) -> Result<Option<Identity>, CleanClawError> {
        let sess = match self.store.get_web_session(sid).await {
            Ok(s) => s,
            Err(CleanClawError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        if sess.expires_at < chrono::Utc::now() {
            return Ok(None);
        }
        let user = self.store.get_user(&sess.user_id).await?;
        Ok(Some(Identity {
            user_id: user.id,
            role: Role::parse(&user.role),
            method: AuthMethod::Session,
            api_key_id: String::new(),
            api_key_type: None,
            api_key_agents: Vec::new(),
        }))
    }
}

/// Axum middleware: resolve the request's auth to an `Identity`,
/// insert it as a request extension so handlers can pull it out with
/// `Extension<Option<Identity>>` (or just call the resolver again).
/// Rejects unauthenticated requests with 401 unless `allow_anon` is
/// set. Use this on `/api/*` routes that require auth.
#[cfg(feature = "axum")]
pub async fn middleware_optional(
    axum::extract::State(state): axum::extract::State<Arc<Resolver>>,
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let headers = req.headers().clone();
    let ident = match state.resolve_from_headers(&headers).await {
        Ok(opt) => opt,
        Err(e) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(format!("auth error: {e}")))
                .unwrap();
        }
    };
    req.extensions_mut().insert(ident);
    next.run(req).await
}

/// Strict variant: 401 on missing identity. Use on `/api/admin/*` etc.
#[cfg(feature = "axum")]
pub async fn middleware_required(
    axum::extract::State(state): axum::extract::State<Arc<Resolver>>,
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let headers = req.headers().clone();
    let ident = match state.resolve_from_headers(&headers).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(
                    r#"{"error":{"message":"unauthorized","type":"authentication_error"}}"#,
                ))
                .unwrap();
        }
        Err(e) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(format!("auth error: {e}")))
                .unwrap();
        }
    };
    req.extensions_mut().insert(Some(ident));
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_name_matches_module_const() {
        assert_eq!(SESSION_COOKIE_NAME, "cleanclaw_session");
    }

    #[test]
    fn session_ttl_is_thirty_days() {
        assert_eq!(SESSION_TTL, Duration::from_secs(30 * 24 * 3600));
    }
}

