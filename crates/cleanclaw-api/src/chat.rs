//! Chat endpoints: `POST /api/chat/stream` and (eventually) the SSE
//! subscription endpoint.
//! `HandleChatStream` + the agent loop wiring.

use super::ApiState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use cleanclaw_agent::{AgentBuilder, IdentityFileStore, SharedEventHub, TurnInput};
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_provider::Provider;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;

pub use cleanclaw_agent::tools::builtins;

/// `ChatService` is the runtime chat subsystem. It owns the event hub
/// and a per-user agent cache. Construction is wired by the gateway.
pub struct ChatService {
    pub providers: parking_lot::RwLock<std::collections::HashMap<String, Arc<dyn Provider>>>,
    pub default_model: String,
    pub store: Arc<dyn Store>,
    pub event_hub: SharedEventHub,
    pub agents: parking_lot::RwLock<std::collections::HashMap<String, Arc<cleanclaw_agent::Agent>>>,
    /// Per-process toolprov registry. Built once at gateway boot
    /// (see `Gateway::boot`) and shared with every agent so the
    /// `web_search` tool can dispatch to the operator-configured
    /// chain.
    pub toolprov: Arc<cleanclaw_toolprov::Registry>,
}

impl ChatService {
    pub fn new(store: Arc<dyn Store>, default_model: String) -> Self {
        Self::new_with_toolprov(
            store,
            default_model,
            Arc::new(cleanclaw_toolprov::Registry::new()),
        )
    }

    /// Construct with a pre-built toolprov registry. The gateway
    /// uses this to share the registry across the whole process;
    /// tests can call `ChatService::new` to get a fresh empty
    /// registry and then `register_builtin` on it.
    pub fn new_with_toolprov(
        store: Arc<dyn Store>,
        default_model: String,
        toolprov: Arc<cleanclaw_toolprov::Registry>,
    ) -> Self {
        Self {
            providers: Default::default(),
            default_model,
            store,
            event_hub: cleanclaw_agent::event_hub::new_shared(1024),
            agents: Default::default(),
            toolprov,
        }
    }

    pub fn register_provider(&self, name: &str, provider: Arc<dyn Provider>) {
        self.providers.write().insert(name.to_string(), provider);
    }

    pub fn event_hub(&self) -> SharedEventHub {
        self.event_hub.clone()
    }

    /// Look up the provider for the given `model` string. A model
    /// reference uses `<provider>/<model>`; a bare `model` falls through
    /// to the default provider (the first registered one).
    pub fn provider_for(&self, model: &str) -> Option<Arc<dyn Provider>> {
        if let Some((name, _)) = model.split_once('/') {
            if let Some(p) = self.providers.read().get(name).cloned() {
                return Some(p);
            }
        }
        // Default: first provider.
        self.providers.read().values().next().cloned()
    }

    /// Run a one-shot turn using the agent matching `model`. The
    /// `Agent` is constructed on demand from store config and cached.
    pub async fn run(
        &self,
        user_id: &str,
        agent_id: &str,
        model: &str,
        message: &str,
        session_key: &str,
    ) -> Result<cleanclaw_agent::AgentOutput> {
        let agent = self.get_or_build_agent(agent_id, model, user_id).await?;
        let input = TurnInput {
            user_text: message.to_string(),
            channel: "web".into(),
            chat_id: user_id.to_string(),
            session_key: session_key.to_string(),
            user_id: user_id.to_string(),
            owner_user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
            is_admin: true,
            history: Vec::new(),
            attachments: Vec::new(),
        };
        agent.run_turn(input).await
    }

    async fn get_or_build_agent(
        &self,
        agent_id: &str,
        model: &str,
        user_id: &str,
    ) -> Result<Arc<cleanclaw_agent::Agent>> {
        if let Some(a) = self.agents.read().get(agent_id).cloned() {
            return Ok(a);
        }
        let provider = self
            .provider_for(model)
            .ok_or_else(|| CleanClawError::NotImplemented("no provider registered".into()))?;

        let model_name = model
            .split_once('/')
            .map(|(_, m)| m.to_string())
            .unwrap_or_else(|| model.to_string());

        // Load identity files for this agent.
        struct StoreAdapter(Arc<dyn Store>, String);
        #[async_trait::async_trait]
        impl cleanclaw_agent::IdentityFileStore for StoreAdapter {
            async fn read(
                &self,
                agent_id: &str,
                _owner_user_id: &str,
                _chatter_user_id: &str,
                filename: &str,
            ) -> Result<Option<String>> {
                let row = self
                    .0
                    .get_workspace_file(agent_id, &self.1, filename)
                    .await
                    .ok();
                Ok(row.map(|(_, bytes)| String::from_utf8_lossy(&bytes).to_string()))
            }
        }
        let adapter = StoreAdapter(self.store.clone(), user_id.to_string());
        let files =
            cleanclaw_agent::IdentityFiles::load(&adapter, agent_id, user_id, user_id).await?;

        // Build the tool registry with builtins.
        let mut tools = cleanclaw_agent::ToolRegistry::new();
        builtins::register_builtins(&mut tools, &self.toolprov);
        // Wire the cron tools.
        tools.register(Arc::new(cleanclaw_agent::tools::cron_tool::CronTool::new(
            self.store.clone(),
            user_id.to_string(),
            agent_id.to_string(),
        )));
        tools.register(Arc::new(
            cleanclaw_agent::tools::cron_tool::ListCronTool::new(
                self.store.clone(),
                agent_id.to_string(),
            ),
        ));
        tools.register(Arc::new(
            cleanclaw_agent::tools::cron_tool::DeleteCronTool::new(
                self.store.clone(),
                user_id.to_string(),
            ),
        ));

        let agent = AgentBuilder::new(agent_id, user_id, model_name, provider, self.store.clone())
            .display_name(agent_id)
            .max_iterations(8)
            .max_tokens(2048)
            .temperature(0.7)
            .tools(tools)
            .event_hub(self.event_hub.clone())
            .identity(files)
            .build();
        let arc = Arc::new(agent);

        // Read the system tools config (Brave key / Bing key /
        // Google cx / etc.) and stash it on the agent's
        // `tool_extras` so `web_search` can pick it up on every
        // turn. The agent cache rebuild fires the next time the
        // agent is asked to run, so an admin can rotate keys
        // without restarting the gateway.
        if let Ok(rows) = self.store.list_configs("tools", "", "").await {
            if let Some(row) = rows.into_iter().next() {
                if let Some(map) = row.data.get("web_search").and_then(|v| v.as_object()) {
                    let provider = map
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .unwrap_or("duckduckgo")
                        .to_string();
                    let api_key = map
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let endpoint = map
                        .get("endpoint")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cfg = cleanclaw_toolprov::ProviderConfig {
                        api_key,
                        endpoint,
                        model: String::new(),
                        options: Default::default(),
                    };
                    let mut configs = std::collections::HashMap::new();
                    configs.insert(provider, cfg);
                    if let Ok(v) = serde_json::to_value(configs) {
                        arc.tool_extras
                            .write()
                            .insert("web_search_configs".to_string(), v);
                    }
                }
            }
        }

        self.agents
            .write()
            .insert(agent_id.to_string(), arc.clone());
        Ok(arc)
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub agent_id: String,
    pub message: String,
    pub session_key: String,
    pub model: Option<String>,
}

pub async fn stream(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let model = req
        .model
        .unwrap_or_else(|| state.chat.default_model.clone());

    // Drive the turn and forward every event from the broadcast
    // event hub as a server-sent-event. Mirrors the CleanClaw
    // `/api/chat/stream` wire format:
    //   event: content_delta\ndata: {"delta": "..."}\n\n
    //   event: content\ndata: {"content": "..."}\n\n
    //   event: tool_call\ndata: {"id":..., "name":..., "arguments":...}\n\n
    //   event: tool_result\ndata: {"id":..., "result":...}\n\n
    //   event: done\ndata: {"finish_reason":..., "usage":...}\n\n
    //   event: error\ndata: {"message": "..."}\n\n
    //
    // Implementation note: we follow the same pattern as
    // `chat_ws::run_turn_and_stream` — subscribe to the hub
    // BEFORE spawning the turn, so no deltas are lost between
    // the spawn and the first hub event.
    let chat = state.chat.clone();
    let user_id = ident.user_id.clone();
    let agent_id = req.agent_id.clone();
    let session_key = req.session_key.clone();
    let message = req.message.clone();

    // Open the hub subscription up-front; filter to our
    // (agent_id, session_key) so concurrent turns on the same
    // chat (e.g. an IM bot replying while the dashboard is
    // streaming) don't interleave.
    let mut hub_rx = chat.event_hub().subscribe();

    // Kick off the turn in the background.
    let turn_chat = chat.clone();
    let turn_user = user_id.clone();
    let turn_agent = agent_id.clone();
    let turn_model = model.clone();
    let turn_message = message.clone();
    let turn_session = session_key.clone();
    let turn_task = tokio::spawn(async move {
        turn_chat
            .run(
                &turn_user,
                &turn_agent,
                &turn_model,
                &turn_message,
                &turn_session,
            )
            .await
    });

    // Forward hub events as SSE frames until we see the terminal
    // `Done` for this (agent_id, session_key) tuple.
    use futures_util::stream::StreamExt;
    let stream = async_stream::stream! {
        use cleanclaw_agent::event_hub::AgentEvent;
        let mut saw_done = false;
        loop {
            let env = match hub_rx.recv().await {
                Ok(e) => e,
                Err(_) => break,
            };
            if env.agent_id != agent_id || env.session_key != session_key {
                continue;
            }
            match env.event {
                AgentEvent::Content { delta } => {
                    yield sse_event("content_delta", json!({ "delta": delta }));
                }
                AgentEvent::Thinking { delta } => {
                    yield sse_event("thinking_delta", json!({ "delta": delta }));
                }
                AgentEvent::ToolCall { name, id, arguments } => {
                    yield sse_event(
                        "tool_call",
                        json!({ "id": id, "name": name, "arguments": arguments }),
                    );
                }
                AgentEvent::ToolResult { id, content, is_error } => {
                    yield sse_event(
                        "tool_result",
                        json!({ "id": id, "result": content, "is_error": is_error }),
                    );
                }
                AgentEvent::Done { finish_reason, usage } => {
                    yield sse_event(
                        "done",
                        json!({ "finish_reason": finish_reason, "usage": usage }),
                    );
                    saw_done = true;
                    break;
                }
                AgentEvent::Error { message } => {
                    yield sse_event("error", json!({ "message": message }));
                    saw_done = true;
                    break;
                }
            }
        }
        let _ = saw_done;
    };
    // SSE wants a `Stream<Item = Result<Event, Infallible>>` — we
    // already produce that, so wrap with `StreamExt::map` to the
    // exact bound axum expects.
    let stream = stream.map(Ok::<_, Infallible>);

    let sse =
        Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    sse.into_response()
}

/// Wrap a JSON payload in an SSE `event: <name>` frame.
fn sse_event<T: serde::Serialize>(name: &str, data: T) -> Event {
    let payload = serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    Event::default().event(name).data(payload)
}

// Expose the require_auth helper for the parent module.
pub(super) use super::require_auth;

// =====================================================================
// /api/chat/history + /api/chat/sessions — dashboard surface
// =====================================================================

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub agent_id: String,
    pub session_key: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub thinking: String,
    pub tool_calls: Value,
    pub tool_call_id: String,
    pub name: String,
    pub created_at: String,
}

pub async fn get_history(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<HistoryQuery>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    // The agent persists the per-session transcript to the
    // `sessions.messages` JSONB column (see `compact::save_compacted`),
    // NOT to `session_messages` (which is reserved for the
    // append-only archive). Read from `get_session` and decode
    // the JSON array. If absent, fall through to the archive
    // (preserves messages from older installs that wrote to
    // `session_messages` directly).
    let mut out: Vec<HistoryMessage> = Vec::new();
    if let Ok(session) = state
        .store
        .get_session(&ident.user_id, &q.agent_id, &q.session_key)
        .await
    {
        if let Some(arr) = session.messages.as_array() {
            for m in arr {
                let role = m
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = m
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let thinking = m
                    .get("thinking")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_calls = m.get("tool_calls").cloned().unwrap_or(Value::Null);
                let tool_call_id = m
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(HistoryMessage {
                    role,
                    content,
                    thinking,
                    tool_calls,
                    tool_call_id,
                    name,
                    created_at: session.updated_at.to_rfc3339(),
                });
            }
        }
    }
    if out.is_empty() {
        // Fallback: read from the session_messages archive (older
        // installs wrote directly there before `save_compacted`
        // took over).
        let limit = q.limit.unwrap_or(200);
        match state
            .store
            .list_session_messages(&ident.user_id, &q.agent_id, &q.session_key)
            .await
        {
            Ok(msgs) => {
                out = msgs
                    .into_iter()
                    .map(|m| HistoryMessage {
                        role: m.role,
                        content: m.content,
                        thinking: m.thinking,
                        tool_calls: m.tool_calls,
                        tool_call_id: m.tool_call_id,
                        name: m.name,
                        created_at: m.created_at.to_rfc3339(),
                    })
                    .collect();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    }
    (StatusCode::OK, Json(json!({ "messages": out }))).into_response()
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    pub agent_id: String,
}

pub async fn list_sessions(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<SessionsQuery>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let rows = match state.store.list_sessions(&ident.user_id, &q.agent_id).await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let out: Vec<Value> = rows
        .into_iter()
        .map(|s| {
            json!({
                "key": s.key,
                "channel": s.channel,
                "account_id": s.account_id,
                "chat_id": s.chat_id,
                "project_id": s.project_id,
                "title": s.title,
                "message_count": s.message_count,
                "updated_at": s.updated_at.to_rfc3339(),
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({ "sessions": out }))).into_response()
}

#[derive(Deserialize)]
pub struct RenameSessionRequest {
    pub title: String,
}

pub async fn rename_session(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(key): Path<String>,
    Query(q): Query<AgentIdOnly>,
    Json(req): Json<RenameSessionRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    // The store's `rename_session` is keyed on the user/agent/session
    // triple, but the path only carries the session key. The
    // dashboard sends `?agent_id=<id>` alongside.
    let agent_id = if q.agent_id.is_empty() {
        "default".to_string()
    } else {
        q.agent_id
    };
    if let Err(e) = state
        .store
        .rename_session(&ident.user_id, &agent_id, &key, &req.title)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

#[derive(Deserialize, Default)]
pub struct AgentIdOnly {
    #[serde(default)]
    pub agent_id: String,
}

pub async fn delete_session(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(key): Path<String>,
    Query(q): Query<AgentIdOnly>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let agent_id = if q.agent_id.is_empty() {
        "default".to_string()
    } else {
        q.agent_id
    };
    if let Err(e) = state
        .store
        .delete_session(&ident.user_id, &agent_id, &key)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

#[derive(Deserialize)]
pub struct MoveSessionProjectRequest {
    pub project_id: String,
}

pub async fn move_session_project(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(key): Path<String>,
    Query(q): Query<AgentIdOnly>,
    Json(req): Json<MoveSessionProjectRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let agent_id = if q.agent_id.is_empty() {
        "default".to_string()
    } else {
        q.agent_id
    };
    // Read the existing session, update its project_id, then save
    // the new record.
    let existing = match state
        .store
        .get_session(&ident.user_id, &agent_id, &key)
        .await
    {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "session not found" })),
            )
                .into_response();
        }
    };
    let mut s = existing;
    s.project_id = req.project_id.clone();
    if let Err(e) = state
        .store
        .save_session(&ident.user_id, &agent_id, &key, &s)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

// =====================================================================
// /api/agents/:id/projects — per-agent project CRUD
// =====================================================================

pub async fn list_projects(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let rows = state.store.list_projects(&ident.user_id, &agent_id).await;
    match rows {
        Ok(v) => {
            let out: Vec<Value> = v
                .into_iter()
                .map(|p| {
                    json!({
                        "id": p.project_id,
                        "name": p.name,
                        "agent_id": p.agent_id,
                        "description": p.description,
                        "created_at": p.created_at.to_rfc3339(),
                        "updated_at": p.updated_at.to_rfc3339(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(json!({ "projects": out }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

pub async fn create_project(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let project_id = format!("prj_{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now();
    let rec = cleanclaw_store::models::ProjectRecord {
        user_id: ident.user_id.clone(),
        agent_id: agent_id.clone(),
        project_id: project_id.clone(),
        name: req.name.clone(),
        description: req.description.clone(),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = state.store.save_project(&rec).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        Json(json!({ "id": project_id, "ok": true })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct UpdateProjectRequest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

pub async fn update_project(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path((agent_id, pid)): Path<(String, String)>,
    Json(req): Json<UpdateProjectRequest>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let existing = match state
        .store
        .get_project(&ident.user_id, &agent_id, &pid)
        .await
    {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "project not found" })),
            )
                .into_response();
        }
    };
    let mut p = existing;
    if !req.name.is_empty() {
        p.name = req.name;
    }
    if !req.description.is_empty() {
        p.description = req.description;
    }
    p.updated_at = chrono::Utc::now();
    if let Err(e) = state.store.save_project(&p).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

pub async fn delete_project(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Path((agent_id, pid)): Path<(String, String)>,
) -> impl IntoResponse {
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(e) = state
        .store
        .delete_project(&ident.user_id, &agent_id, &pid)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}
