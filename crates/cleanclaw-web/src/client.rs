//! Typed API client. Mirrors the 151 `export async function` helpers
//! in . The client is a thin
//! `reqwest` wrapper that:
//!
//! 1. Holds a base URL and an optional bearer token.
//! 2. Exposes generic `get` / `post` / `put` / `patch` / `delete`.
//! 3. Exposes a strongly-typed wrapper per endpoint, named exactly
//!    like the TypeScript original (e.g. `login`, `getMe`,
//!    `updateAgent`, `sendChat`, `adminCreateUser`).
//!
//! Errors bubble up as `ApiError`. Non-2xx responses carry the status
//! code; JSON error envelopes (`{ "error": "..." }`) get unwrapped
//! and exposed via the `error` field.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

use crate::types::*;

// =====================================================================
// Client + error
// =====================================================================

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("http {status}: {message}")]
    Http { status: u16, message: String },
    #[error("decode: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
}

impl ApiError {
    /// Server returned a non-2xx response. `message` is the body
    /// (truncated to 4 KB) or the status text.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// `ApiClient` is the typed HTTP wrapper used by the SSR handlers
/// and (in W3+) by any external script that needs to talk to
/// `cleanclaw-api` / `cleanclaw-setup`.
#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    bearer: Option<String>,
    client: reqwest::Client,
    /// Optional `actAs=<userId>` mirror — appended to every request
    /// when set. Mirrors `apiFetch`'s page-level `actAs` injection.
    act_as: Option<String>,
}

impl ApiClient {
    /// Build a client pointing at the same-origin `/api` path (the
    /// SSR handlers use this so the server talks to itself without
    /// going over a real socket).
    pub fn same_origin() -> Self {
        Self::new("/api".to_string())
    }

    /// Build a client with an explicit base URL.
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        Self {
            base_url,
            bearer: None,
            client,
            act_as: None,
        }
    }

    /// Override the bearer token (programmatic auth; cookie sessions
    /// are preferred for the web UI).
    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(token.into());
        self
    }

    /// Mirror the URL-level `actAs=<userId>` flag from the page into
    /// every outbound request. See `apiFetch` in `lib/api.ts`.
    pub fn with_act_as(mut self, user_id: impl Into<String>) -> Self {
        self.act_as = Some(user_id.into());
        self
    }

    /// Read-only access to the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build a full URL from a path. Appends `actAs=` automatically
    /// when the client has it set and the path doesn't already
    /// include it.
    pub fn url(&self, path: &str) -> String {
        if let Some(a) = &self.act_as {
            if !path.contains("actAs=") {
                let sep = if path.contains('?') { '&' } else { '?' };
                return format!("{path}{sep}actAs={}", urlencode(a));
            }
        }
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// Issue a `GET`. Returns the parsed JSON body, or `None` for
    /// empty 2xx responses.
    pub async fn get_json<T: DeserializeOwned + Default>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.url(path);
        let mut req = self.client.get(&url);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        self.parse(res).await
    }

    /// Issue a `POST` with a JSON body. Returns the parsed JSON
    /// body, or `None` for empty 2xx responses.
    pub async fn post_json<B: Serialize, T: DeserializeOwned + Default>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let mut req = self.client.post(&url).json(body);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        self.parse(res).await
    }

    /// Issue a `PUT` with a JSON body.
    pub async fn put_json<B: Serialize, T: DeserializeOwned + Default>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let mut req = self.client.put(&url).json(body);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        self.parse(res).await
    }

    /// Issue a `PATCH` with a JSON body.
    pub async fn patch_json<B: Serialize, T: DeserializeOwned + Default>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let mut req = self.client.patch(&url).json(body);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        self.parse(res).await
    }

    /// Issue a `DELETE`. Returns the parsed JSON body (some handlers
    /// return `{ok: true}` on delete).
    pub async fn delete_json<T: DeserializeOwned + Default>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.url(path);
        let mut req = self.client.delete(&url);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        self.parse(res).await
    }

    /// Multipart upload. Used by `uploadAgentFiles` / `uploadSkill`.
    pub async fn post_multipart<T: DeserializeOwned + Default>(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let mut req = self.client.post(&url).multipart(form);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        self.parse(res).await
    }

    async fn parse<T: DeserializeOwned + Default>(
        &self,
        res: reqwest::Response,
    ) -> Result<T, ApiError> {
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            let message = if body.len() > 4096 {
                format!("{}...", &body[..4096])
            } else {
                body
            };
            return Err(ApiError::Http {
                status: status.as_u16(),
                message,
            });
        }
        // Some endpoints return 204 No Content (or empty body on
        // success); fall back to Default::default().
        let bytes = res.bytes().await?;
        if bytes.is_empty() {
            return Ok(T::default());
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

// =====================================================================
// URL helpers
// =====================================================================

/// `urlencode` — minimal URL-component encoder.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// `urlencode_decode` — inverse of `urlencode`. Accepts `%XX` and
/// `+` (the latter decoded to space). Invalid sequences are passed
/// through verbatim.
pub fn urlencode_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h1 = hex_nibble(bytes[i + 1]);
                let h2 = hex_nibble(bytes[i + 2]);
                match (h1, h2) {
                    (Some(a), Some(b)) => {
                        out.push((a << 4) | b);
                        i += 3;
                    }
                    _ => {
                        out.push(b);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Build a query string from a `HashMap<String, String>`. Skips
/// empty values.
pub fn build_query(q: &HashMap<String, String>) -> String {
    if q.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = q
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect();
    parts.sort();
    format!("?{}", parts.join("&"))
}

// =====================================================================
// Endpoints — auth
// =====================================================================

impl ApiClient {
    pub async fn register(&self, req: &RegisterRequest) -> Result<MeResponse, ApiError> {
        self.post_json("/register", req).await
    }

    pub async fn login(&self, login_field: &str, password: &str) -> Result<MeResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            login: &'a str,
            password: &'a str,
        }
        self.post_json("/login", &Body { login: login_field, password }).await
    }

    pub async fn logout(&self) -> Result<serde_json::Value, ApiError> {
        self.post_json::<(), _>("/logout", &()).await
    }

    pub async fn get_me(&self) -> Result<MeResponse, ApiError> {
        self.get_json("/me").await
    }

    pub async fn update_me(&self, req: &UpdateMeRequest) -> Result<serde_json::Value, ApiError> {
        self.put_json("/me", req).await
    }

    pub async fn change_my_password(&self, req: &ChangePasswordRequest) -> Result<serde_json::Value, ApiError> {
        self.post_json("/me/password", req).await
    }

    pub async fn onboard(&self, req: &OnboardRequest) -> Result<OnboardResponse, ApiError> {
        self.post_json("/onboard", req).await
    }
}

// =====================================================================
// Endpoints — admin: users + apikeys
// =====================================================================

impl ApiClient {
    pub async fn admin_list_users(&self) -> Result<serde_json::Value, ApiError> {
        self.get_json("/users").await
    }

    pub async fn admin_list_agents(&self) -> Result<serde_json::Value, ApiError> {
        self.get_json("/agents?all=true").await
    }

    pub async fn admin_create_user(&self, req: &AdminCreateUserRequest) -> Result<serde_json::Value, ApiError> {
        self.post_json("/users", req).await
    }

    pub async fn admin_update_user(
        &self,
        id: &str,
        req: &AdminUpdateUserRequest,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json(&format!("/users/{id}"), req).await
    }

    pub async fn admin_delete_user(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/users/{id}")).await
    }

    pub async fn admin_reset_password(
        &self,
        id: &str,
        password: &str,
    ) -> Result<serde_json::Value, ApiError> {
        self.post_json(&format!("/users/{id}/password"), &AdminResetPasswordRequest {
            password: password.to_string(),
        })
        .await
    }

    pub async fn list_apikeys(&self) -> Result<serde_json::Value, ApiError> {
        self.get_json("/apikeys").await
    }

    pub async fn create_apikey(
        &self,
        name: &str,
        kind: ApikeyType,
        agent_ids: Option<&[String]>,
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            #[serde(rename = "type")]
            kind: ApikeyType,
            #[serde(skip_serializing_if = "Option::is_none")]
            agent_ids: Option<&'a [String]>,
        }
        self.post_json(
            "/apikeys",
            &Body {
                name,
                kind,
                agent_ids,
            },
        )
        .await
    }

    pub async fn delete_apikey(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/apikeys/{id}")).await
    }

    pub async fn rotate_apikey(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.post_json::<(), _>(&format!("/apikeys/{id}/rotate"), &()).await
    }

    pub async fn set_apikey_agents(
        &self,
        id: &str,
        agent_ids: &[String],
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            agent_ids: &'a [String],
        }
        self.put_json(&format!("/apikeys/{id}/agents"), &Body { agent_ids }).await
    }

    /// `/v1/admin/apikeys` — v1 admin surface (the new namespace
    /// that lives alongside the resource-style `/api/apikeys`).
    pub async fn list_api_keys(&self) -> Result<Vec<APIKey>, ApiError> {
        self.get_json("/../v1/admin/apikeys").await
    }

    pub async fn create_api_key(
        &self,
        id: &str,
        name: &str,
    ) -> Result<APIKeyCreateResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            id: &'a str,
            name: &'a str,
        }
        self.post_json("/../v1/admin/apikeys", &Body { id, name }).await
    }

    pub async fn delete_api_key(&self, id: &str) -> Result<(), ApiError> {
        let _: serde_json::Value = self.delete_json(&format!("/../v1/admin/apikeys/{id}")).await?;
        Ok(())
    }

    pub async fn rotate_api_key(&self, id: &str) -> Result<String, ApiError> {
        let r: APIKeyRotateResponse = self
            .post_json::<(), _>(&format!("/../v1/admin/apikeys/{id}/rotate"), &())
            .await?;
        Ok(r.key)
    }

    pub async fn list_agent_bindings(&self) -> Result<AgentBindings, ApiError> {
        self.get_json("/agent-bindings").await
    }

    pub async fn bind_agent(
        &self,
        agent_id: &str,
        api_key_id: &str,
    ) -> Result<BindAgentResponse, ApiError> {
        self.post_json(
            &format!("/agents/{agent_id}/binding"),
            &BindAgentRequest { api_key_id: api_key_id.to_string() },
        )
        .await
    }
}

// =====================================================================
// Endpoints — providers + channels
// =====================================================================

impl ApiClient {
    pub async fn list_providers(
        &self,
        scope: Option<ScopeName>,
        scope_id: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let mut q = HashMap::new();
        if let Some(s) = scope {
            q.insert("scope".into(), s.as_str().into());
        }
        if let Some(id) = scope_id {
            q.insert("scopeId".into(), id.into());
        }
        self.get_json(&format!("/providers{}", build_query(&q))).await
    }

    pub async fn create_provider(&self, body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        self.post_json("/providers", body).await
    }

    pub async fn update_provider(
        &self,
        id: &str,
        body: &ProviderRow,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json(&format!("/providers/{id}"), body).await
    }

    pub async fn delete_provider(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/providers/{id}")).await
    }

    pub async fn test_stored_provider(
        &self,
        provider_id: &str,
        model: &str,
        overrides: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            model: &'a str,
            #[serde(flatten, skip_serializing_if = "Option::is_none")]
            overrides: Option<serde_json::Value>,
        }
        self.post_json(
            &format!("/providers/{provider_id}/test"),
            &Body { model, overrides },
        )
        .await
    }

    pub async fn list_scoped_channels(
        &self,
        scope: Option<ScopeName>,
        scope_id: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let mut q = HashMap::new();
        if let Some(s) = scope {
            q.insert("scope".into(), s.as_str().into());
        }
        if let Some(id) = scope_id {
            q.insert("scopeId".into(), id.into());
        }
        self.get_json(&format!("/scoped-channels{}", build_query(&q))).await
    }

    pub async fn create_scoped_channel(&self, body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        self.post_json("/scoped-channels", body).await
    }

    pub async fn update_scoped_channel(
        &self,
        id: &str,
        body: &ChannelRow,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json(&format!("/scoped-channels/{id}"), body).await
    }

    pub async fn delete_scoped_channel(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/scoped-channels/{id}")).await
    }
}

// =====================================================================
// Endpoints — status, config, test-provider
// =====================================================================

impl ApiClient {
    pub async fn get_status(&self) -> Result<StatusResponse, ApiError> {
        self.get_json("/status").await
    }

    pub async fn test_provider(
        &self,
        api_base: &str,
        api_key: &str,
        model: &str,
        api_type: Option<&str>,
        auth_type: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            api_base: &'a str,
            api_key: &'a str,
            model: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            api_type: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            auth_type: Option<&'a str>,
        }
        self.post_json(
            "/test-provider",
            &Body { api_base, api_key, model, api_type, auth_type },
        )
        .await
    }

    pub async fn get_config(&self) -> Result<ConfigResponse, ApiError> {
        self.get_json("/config").await
    }

    pub async fn save_config(
        &self,
        config: &serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        self.post_json("/config", config).await
    }

    pub async fn update_config(
        &self,
        config: &serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        // Same endpoint as save; alias for clarity.
        self.save_config(config).await
    }

    pub async fn update_skill_entries(
        &self,
        entries: &serde_json::Value,
        agent_id: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let body = if let Some(a) = agent_id {
            serde_json::json!({ "skills": { "agentEntries": { a: entries } } })
        } else {
            serde_json::json!({ "skills": { "entries": entries } })
        };
        self.post_json("/config", &body).await
    }
}

// =====================================================================
// Endpoints — workspace files (per agent)
// =====================================================================

impl ApiClient {
    pub async fn reveal_agent_workspace(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let mut q = HashMap::new();
        if let Some(s) = session_id {
            q.insert("sessionId".into(), s.into());
        }
        if let Some(p) = project_id {
            q.insert("projectId".into(), p.into());
        }
        let path = format!(
            "/agents/{}/workspace/reveal{}",
            urlencode(agent_id),
            build_query(&q)
        );
        self.post_json::<(), _>(&path, &()).await
    }

    pub async fn list_agent_files(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<WorkspaceFile>, ApiError> {
        let mut q = HashMap::new();
        if let Some(s) = session_id {
            q.insert("sessionId".into(), s.into());
        }
        if let Some(p) = project_id {
            q.insert("projectId".into(), p.into());
        }
        let path = format!("/agents/{}/files{}", urlencode(agent_id), build_query(&q));
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            files: Vec<WorkspaceFile>,
        }
        let w: Wrap = self.get_json(&path).await?;
        Ok(w.files)
    }

    /// Upload a file to an agent workspace. The form is built
    /// outside; this is a thin convenience over `post_multipart`.
    pub async fn upload_agent_files(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        form: reqwest::multipart::Form,
    ) -> Result<serde_json::Value, ApiError> {
        let qs = match session_id {
            Some(s) => format!("?sessionId={}", urlencode(s)),
            None => String::new(),
        };
        self.post_multipart(&format!("/agents/{}/files{}", urlencode(agent_id), qs), form)
            .await
    }
}

// =====================================================================
// Endpoints — chat
// =====================================================================

impl ApiClient {
    pub async fn get_chat_todo(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<TodoState, ApiError> {
        let path = format!(
            "/chat/todo?agentId={}&sessionId={}",
            urlencode(agent_id),
            urlencode(session_id)
        );
        self.get_json(&path).await
    }

    pub async fn get_chat_history(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Vec<ChatHistoryMessage>, ApiError> {
        let path = format!(
            "/chat/history?agentId={}&sessionId={}",
            urlencode(agent_id),
            urlencode(session_id)
        );
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            history: Vec<ChatHistoryMessage>,
        }
        let w: Wrap = self.get_json(&path).await?;
        Ok(w.history)
    }

    pub async fn get_chat_history_with_cursor(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<ChatHistoryResult, ApiError> {
        let path = format!(
            "/chat/history?agentId={}&sessionId={}",
            urlencode(agent_id),
            urlencode(session_id)
        );
        self.get_json(&path).await
    }

    pub async fn get_chat_sessions(
        &self,
        agent_id: &str,
    ) -> Result<Vec<ChatSessionEntry>, ApiError> {
        let path = format!("/chat/sessions?agentId={}", urlencode(agent_id));
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            sessions: Vec<ChatSessionEntry>,
        }
        let w: Wrap = self.get_json(&path).await?;
        Ok(w.sessions)
    }

    pub async fn admin_list_chats(&self) -> Result<Vec<AdminChatSessionEntry>, ApiError> {
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            sessions: Vec<AdminChatSessionEntry>,
        }
        let w: Wrap = self.get_json("/admin/chats").await?;
        Ok(w.sessions)
    }

    pub async fn rename_chat_session(
        &self,
        _agent_id: &str,
        session_id: &str,
        title: &str,
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            agent_id: &'a str,
            title: &'a str,
        }
        self.put_json(
            &format!("/chat/sessions/{}", urlencode(session_id)),
            &Body { agent_id: _agent_id, title },
        )
        .await
    }

    pub async fn delete_chat_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let path = format!(
            "/chat/sessions/{}?agentId={}",
            urlencode(session_id),
            urlencode(agent_id)
        );
        self.delete_json(&path).await
    }

    pub async fn move_chat_session_to_project(
        &self,
        agent_id: &str,
        session_id: &str,
        project_id: &str,
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            agent_id: &'a str,
            project_id: &'a str,
        }
        self.patch_json(
            &format!("/chat/sessions/{}/project", urlencode(session_id)),
            &Body { agent_id, project_id },
        )
        .await
    }

    pub async fn send_chat(
        &self,
        agent_id: &str,
        session_id: &str,
        message: &str,
    ) -> Result<SendChatResponse, ApiError> {
        self.post_json(
            "/chat",
            &SendChatRequest {
                agent_id: agent_id.into(),
                session_id: session_id.into(),
                message: message.into(),
            },
        )
        .await
    }

    pub async fn steer_chat(
        &self,
        agent_id: &str,
        session_id: &str,
        message: &str,
        project_id: Option<&str>,
    ) -> Result<bool, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            agent_id: &'a str,
            session_id: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            project_id: Option<&'a str>,
            message: &'a str,
        }
        let url = self.url("/chat/steer");
        let mut req = self.client.post(&url).json(&Body {
            agent_id,
            session_id,
            project_id,
            message,
        });
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        if res.status().as_u16() == 409 {
            return Ok(false);
        }
        if !res.status().is_success() {
            return Err(ApiError::Http {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        let v: serde_json::Value = res.json().await?;
        Ok(v.get("buffered").and_then(|x| x.as_bool()).unwrap_or(false))
    }
}

// =====================================================================
// Endpoints — projects
// =====================================================================

impl ApiClient {
    pub async fn list_projects(&self, agent_id: &str) -> Result<Vec<ProjectEntry>, ApiError> {
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            projects: Vec<ProjectEntry>,
        }
        let w: Wrap = self
            .get_json(&format!("/agents/{}/projects", urlencode(agent_id)))
            .await?;
        Ok(w.projects)
    }

    pub async fn create_project(
        &self,
        agent_id: &str,
        req: &CreateProjectRequest,
    ) -> Result<ProjectEntry, ApiError> {
        self.post_json(&format!("/agents/{}/projects", urlencode(agent_id)), req)
            .await
    }

    pub async fn update_project(
        &self,
        agent_id: &str,
        project_id: &str,
        req: &UpdateProjectRequest,
    ) -> Result<ProjectEntry, ApiError> {
        self.patch_json(
            &format!("/agents/{}/projects/{}", urlencode(agent_id), urlencode(project_id)),
            req,
        )
        .await
    }

    pub async fn delete_project(
        &self,
        agent_id: &str,
        project_id: &str,
    ) -> Result<DeleteProjectResponse, ApiError> {
        self.delete_json(&format!(
            "/agents/{}/projects/{}",
            urlencode(agent_id),
            urlencode(project_id)
        ))
        .await
    }
}

// =====================================================================
// Endpoints — agents (CRUD + tools + config)
// =====================================================================

impl ApiClient {
    pub async fn get_agents(&self) -> Result<Vec<AgentDetail>, ApiError> {
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            agents: Vec<AgentDetail>,
        }
        let w: Wrap = self.get_json("/agents").await?;
        Ok(w.agents)
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<AgentDetail>, ApiError> {
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            agent: Option<AgentDetail>,
        }
        let w: Wrap = self.get_json(&format!("/agents/{}", urlencode(id))).await?;
        Ok(w.agent)
    }

    pub async fn get_agent_status(
        &self,
        id: &str,
    ) -> Result<(u16, Option<AgentDetail>), ApiError> {
        let url = self.url(&format!("/agents/{}", urlencode(id)));
        let mut req = self.client.get(&url);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        if !res.status().is_success() {
            return Ok((status, None));
        }
        let v: serde_json::Value = res.json().await?;
        let agent = serde_json::from_value(v.get("agent").cloned().unwrap_or_default()).ok();
        Ok((status, agent))
    }

    pub async fn list_agent_registered_tools(
        &self,
        id: &str,
    ) -> Result<Option<Vec<AgentRegisteredTool>>, ApiError> {
        let url = self.url(&format!("/agents/{}/tools/registered", urlencode(id)));
        let mut req = self.client.get(&url);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        if !res.status().is_success() {
            return Ok(None);
        }
        let v: serde_json::Value = res.json().await?;
        let tools = serde_json::from_value(v.get("tools").cloned().unwrap_or_default()).ok();
        Ok(tools)
    }

    pub async fn create_agent(
        &self,
        agent: &serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        self.post_json("/agents", agent).await
    }

    pub async fn update_agent(
        &self,
        id: &str,
        agent: &AgentUpdatePayload,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json(&format!("/agents/{}", urlencode(id)), agent).await
    }

    pub async fn get_agent_config(&self, id: &str) -> Result<AgentFileConfig, ApiError> {
        self.get_json(&format!("/agents/{}/config", urlencode(id))).await
    }

    pub async fn delete_agent(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/agents/{}", urlencode(id))).await
    }

    pub async fn list_hook_plugins(&self) -> Result<Vec<HookPlugin>, ApiError> {
        let url = self.url("/plugins/hook");
        let mut req = self.client.get(&url);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await?;
        if !res.status().is_success() {
            return Ok(vec![]);
        }
        Ok(res.json().await.unwrap_or_default())
    }
}

// =====================================================================
// Endpoints — skills
// =====================================================================

impl ApiClient {
    pub async fn get_skills(&self) -> Result<Vec<SkillInfo>, ApiError> {
        self.get_json("/skills").await
    }

    pub async fn delete_skill(&self, name: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/skills/{}", urlencode(name))).await
    }

    pub async fn get_agent_skills(&self, agent_id: &str) -> Result<Vec<SkillInfo>, ApiError> {
        self.get_json(&format!("/agents/{}/skills", urlencode(agent_id)))
            .await
    }

    pub async fn delete_agent_skill(
        &self,
        agent_id: &str,
        name: &str,
    ) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!(
            "/agents/{}/skills/{}",
            urlencode(agent_id),
            urlencode(name)
        ))
        .await
    }

    pub async fn search_skills(&self, query: &str) -> Result<Vec<SkillSearchResult>, ApiError> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let path = format!("/skills/search?source=skillssh&q={}", urlencode(query));
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            results: Vec<SkillSearchResult>,
        }
        let w: Wrap = self.get_json(&path).await?;
        Ok(w.results)
    }

    pub async fn install_skill(
        &self,
        req: &InstallSkillRequest,
    ) -> Result<InstallSkillResponse, ApiError> {
        self.post_json("/skills/install", req).await
    }

    pub async fn upload_skill(
        &self,
        agent_id: Option<&str>,
        form: reqwest::multipart::Form,
    ) -> Result<InstallSkillResponse, ApiError> {
        let qs = match agent_id {
            Some(a) => format!("?agent={}", urlencode(a)),
            None => String::new(),
        };
        self.post_multipart(&format!("/skills/upload{qs}"), form).await
    }
}

// =====================================================================
// Endpoints — tools
// =====================================================================

impl ApiClient {
    pub async fn get_tools(&self) -> Result<ToolsConfig, ApiError> {
        self.get_json("/tools").await
    }

    pub async fn save_tools(
        &self,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json("/tools", payload).await
    }
}

// =====================================================================
// Endpoints — plugins
// =====================================================================

impl ApiClient {
    pub async fn get_plugins(&self) -> Result<Vec<PluginInfo>, ApiError> {
        self.get_json("/plugins").await
    }

    pub async fn update_plugin(
        &self,
        id: &str,
        data: &PluginInfo,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json(&format!("/plugins/{}", urlencode(id)), data).await
    }
}

// =====================================================================
// Endpoints — channels
// =====================================================================

impl ApiClient {
    pub async fn get_channels(&self) -> Result<Vec<ChannelInfo>, ApiError> {
        self.get_json("/channels").await
    }
}

// =====================================================================
// Endpoints — cron
// =====================================================================

impl ApiClient {
    pub async fn get_cron_jobs(&self) -> Result<Vec<CronJobInfo>, ApiError> {
        self.get_json("/cron").await
    }

    pub async fn create_cron_job(
        &self,
        job: &CronJobInfo,
    ) -> Result<serde_json::Value, ApiError> {
        self.post_json("/cron", job).await
    }

    pub async fn update_cron_job(
        &self,
        id: &str,
        job: &CronJobInfo,
    ) -> Result<serde_json::Value, ApiError> {
        self.put_json(&format!("/cron/{}", urlencode(id)), job).await
    }

    pub async fn delete_cron_job(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!("/cron/{}", urlencode(id))).await
    }

    pub async fn list_agent_cron_jobs(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentCronJob>, ApiError> {
        let path = format!("/agents/{}/cron", urlencode(agent_id));
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            jobs: Vec<AgentCronJob>,
        }
        let w: Wrap = self.get_json(&path).await?;
        Ok(w.jobs)
    }

    pub async fn delete_agent_cron_job(
        &self,
        agent_id: &str,
        job_id: &str,
    ) -> Result<serde_json::Value, ApiError> {
        self.delete_json(&format!(
            "/agents/{}/cron/{}",
            urlencode(agent_id),
            urlencode(job_id)
        ))
        .await
    }

    pub async fn toggle_agent_cron_job(
        &self,
        agent_id: &str,
        job_id: &str,
        enabled: bool,
    ) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body {
            enabled: bool,
        }
        self.put_json(
            &format!("/agents/{}/cron/{}", urlencode(agent_id), urlencode(job_id)),
            &Body { enabled },
        )
        .await
    }
}

// =====================================================================
// Endpoints — per-agent IM channels
// =====================================================================

impl ApiClient {
    pub async fn list_agent_channels(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentChannel>, ApiError> {
        let path = format!("/agents/{}/channels", urlencode(agent_id));
        #[derive(serde::Deserialize, Default)]
        struct Wrap {
            #[serde(default)]
            channels: Vec<AgentChannel>,
        }
        let w: Wrap = self.get_json(&path).await?;
        Ok(w.channels)
    }

    pub async fn connect_agent_telegram(
        &self,
        agent_id: &str,
        bot_token: &str,
    ) -> Result<ConnectTelegramResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            bot_token: &'a str,
        }
        self.post_json(
            &format!("/agents/{}/channels/telegram", urlencode(agent_id)),
            &Body { bot_token },
        )
        .await
    }

    pub async fn connect_agent_discord(
        &self,
        agent_id: &str,
        bot_token: &str,
    ) -> Result<ConnectDiscordResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            bot_token: &'a str,
        }
        self.post_json(
            &format!("/agents/{}/channels/discord", urlencode(agent_id)),
            &Body { bot_token },
        )
        .await
    }

    pub async fn connect_agent_slack(
        &self,
        agent_id: &str,
        bot_token: &str,
        app_token: &str,
    ) -> Result<ConnectSlackResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            bot_token: &'a str,
            app_token: &'a str,
        }
        self.post_json(
            &format!("/agents/{}/channels/slack", urlencode(agent_id)),
            &Body { bot_token, app_token },
        )
        .await
    }

    pub async fn start_agent_wechat_login(
        &self,
        agent_id: &str,
    ) -> Result<StartWeChatLoginResponse, ApiError> {
        self.post_json::<serde_json::Value, _>(
            &format!("/agents/{}/channels/wechat/login", urlencode(agent_id)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn poll_agent_wechat_login_status(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<PollWeChatLoginResponse, ApiError> {
        self.get_json(&format!(
            "/agents/{}/channels/wechat/login/status?session={}",
            urlencode(agent_id),
            urlencode(session_id)
        ))
        .await
    }

    pub async fn connect_agent_line(
        &self,
        agent_id: &str,
        channel_token: &str,
        channel_secret: &str,
    ) -> Result<ConnectLineResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            channel_token: &'a str,
            channel_secret: &'a str,
        }
        self.post_json(
            &format!("/agents/{}/channels/line", urlencode(agent_id)),
            &Body { channel_token, channel_secret },
        )
        .await
    }

    pub async fn connect_agent_feishu(
        &self,
        agent_id: &str,
        app_id: &str,
        app_secret: &str,
        verification_token: &str,
        encrypt_key: &str,
        use_long_conn: bool,
    ) -> Result<ConnectFeishuResponse, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            app_id: &'a str,
            app_secret: &'a str,
            verification_token: &'a str,
            encrypt_key: &'a str,
            use_long_conn: bool,
        }
        self.post_json(
            &format!("/agents/{}/channels/feishu", urlencode(agent_id)),
            &Body { app_id, app_secret, verification_token, encrypt_key, use_long_conn },
        )
        .await
    }

    pub async fn disconnect_agent_channel(
        &self,
        agent_id: &str,
        kind: &str,
        account_id: &str,
    ) -> Result<DisconnectChannelResponse, ApiError> {
        self.delete_json(&format!(
            "/agents/{}/channels/{}/{}",
            urlencode(agent_id),
            urlencode(kind),
            urlencode(account_id)
        ))
        .await
    }
}

// =====================================================================
// Endpoints — token usage
// =====================================================================

impl ApiClient {
    pub async fn admin_get_token_usage(
        &self,
        range: TokenUsageRange,
        limit: u32,
    ) -> Result<TokenUsageReport, ApiError> {
        let path = format!("/usage?range={}&limit={}", range.as_str(), limit);
        self.get_json(&path).await
    }

    pub async fn get_agent_token_usage(
        &self,
        agent_id: &str,
        range: TokenUsageRange,
        limit: u32,
    ) -> Result<AgentTokenUsage, ApiError> {
        let path = format!(
            "/agents/{}/usage?range={}&limit={}",
            urlencode(agent_id),
            range.as_str(),
            limit
        );
        self.get_json(&path).await
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_basic() {
        assert_eq!(urlencode("hello"), "hello");
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn build_query_skips_empty() {
        let mut q = HashMap::new();
        q.insert("a".into(), "1".into());
        q.insert("b".into(), "".into());
        let s = build_query(&q);
        assert!(s.contains("a=1"));
        assert!(!s.contains("b="));
    }

    #[test]
    fn build_query_sorts() {
        let mut q = HashMap::new();
        q.insert("b".into(), "2".into());
        q.insert("a".into(), "1".into());
        assert_eq!(build_query(&q), "?a=1&b=2");
    }

    #[test]
    fn url_joins_base_and_path() {
        let c = ApiClient::new("/api".into());
        assert_eq!(c.url("/me"), "/api/me");
        assert_eq!(c.url("/me?x=1"), "/api/me?x=1");
    }

    #[test]
    fn url_appends_act_as() {
        let c = ApiClient::new("/api".into()).with_act_as("user_42");
        assert!(c.url("/me").contains("actAs=user_42"));
        assert!(c.url("/me?actAs=user_99").contains("actAs=user_99"));
    }

    #[test]
    fn url_no_double_slash() {
        let c = ApiClient::new("/api/".into());
        assert_eq!(c.url("/me"), "/api/me");
    }

    #[test]
    fn scope_name_as_str() {
        assert_eq!(ScopeName::System.as_str(), "system");
        assert_eq!(ScopeName::User.as_str(), "user");
        assert_eq!(ScopeName::Agent.as_str(), "agent");
    }

    #[test]
    fn token_usage_range_default() {
        assert_eq!(TokenUsageRange::default().as_str(), "7d");
    }

    #[test]
    fn api_error_http_status() {
        let e = ApiError::Http {
            status: 404,
            message: "not found".into(),
        };
        assert_eq!(e.http_status(), Some(404));
        let e2 = ApiError::Decode(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "x",
        )));
        assert_eq!(e2.http_status(), None);
    }

    /// End-to-end test: spin up a tiny axum server that records the
    /// last request and replies with a canned body; verify the
    /// client sends the right verb + body + auth header, and parses
    /// the response.
    #[tokio::test]
    async fn client_sends_get_and_parses_json() {
        use axum::{routing::get, Json, Router};
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let recorded: Arc<Mutex<Option<(String, String, Option<String>)>>> =
            Arc::new(Mutex::new(None));
        let recorded_h = recorded.clone();

        let app = Router::new().route(
            "/api/status",
            get(move || {
                let recorded = recorded_h.clone();
                async move {
                    *recorded.lock().await = Some(("GET".into(), "/api/status".into(), None));
                    Json(serde_json::json!({
                        "configured": true,
                        "running": true,
                        "port": 8080,
                        "uptime": "1h",
                        "provider": {
                            "name": "openai",
                            "model": "gpt-4o",
                            "apiBase": "https://api.openai.com/v1",
                            "apiKey": "***"
                        },
                        "agents": [],
                        "channels": []
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // Client is rooted at "http://<addr>/api" so the leading "/" in
        // "/status" is replaced by "/api/status".
        let client = ApiClient::new(format!("http://{addr}/api"));
        let s: StatusResponse = client.get_status().await.unwrap();
        assert!(s.configured);
        assert_eq!(s.provider.model, "gpt-4o");
        let rec = recorded.lock().await.clone().unwrap();
        assert_eq!(rec.0, "GET");
        assert_eq!(rec.1, "/api/status");
        h.abort();
    }

    #[tokio::test]
    async fn client_sends_post_with_body() {
        use axum::{routing::post, Json, Router};
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let recorded: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let rec_h = recorded.clone();

        let app = Router::new().route(
            "/api/login",
            post(move |body: String| {
                let recorded = rec_h.clone();
                async move {
                    *recorded.lock().await = Some(body);
                    Json(serde_json::json!({
                        "ok": true,
                        "user": {
                            "id": "u1",
                            "username": "ada",
                            "email": "ada@example.com",
                            "role": "user",
                            "status": "active"
                        }
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = ApiClient::new(format!("http://{addr}/api"));
        let r: MeResponse = client.login("ada", "secret").await.unwrap();
        assert!(r.ok);
        let body = recorded.lock().await.clone().unwrap();
        assert!(body.contains("\"login\":\"ada\""));
        assert!(body.contains("\"password\":\"secret\""));
        h.abort();
    }

    #[tokio::test]
    async fn client_sends_bearer_header() {
        use axum::{routing::get, Router};
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let recorded: Arc<Mutex<Option<Option<String>>>> = Arc::new(Mutex::new(None));
        let rec_h = recorded.clone();
        let app = Router::new().route(
            "/api/me",
            get(move |headers: axum::http::HeaderMap| {
                let recorded = rec_h.clone();
                async move {
                    *recorded.lock().await =
                        Some(headers.get("authorization").and_then(|v| v.to_str().ok().map(String::from)));
                    axum::Json(serde_json::json!({ "ok": true }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = ApiClient::new(format!("http://{addr}/api")).with_bearer("test-token-xyz");
        let _: serde_json::Value = client.get_json("/me").await.unwrap();
        let auth = recorded.lock().await.clone();
        // The recorded slot is `Option<Option<String>>` — outer from
        // the lock's initial value, inner from the header lookup.
        assert_eq!(
            auth,
            Some(Some("Bearer test-token-xyz".to_string())),
            "bearer header sent"
        );
        h.abort();
    }

    #[tokio::test]
    async fn client_404_returns_api_error() {
        use axum::{routing::get, Router};
        // Only register /api/other; the test queries /api/missing which
        // should 404.
        let app = Router::new().route("/api/other", get(|| async { "nope" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = ApiClient::new(format!("http://{addr}/api"));
        let e: ApiError = client.get_json::<serde_json::Value>("/missing").await.unwrap_err();
        assert_eq!(e.http_status(), Some(404));
        h.abort();
    }
}
