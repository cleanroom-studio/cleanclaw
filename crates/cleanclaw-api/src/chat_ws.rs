//! WebSocket chat streaming endpoint.
//!
//! `GET /api/ws/chat` upgrades an HTTP request to a WebSocket and
//! drives agent turns, streaming every `AgentEvent` to the client
//! as a JSON frame. Mirrors the React UI's `sendChatStream()` call
//! shape so the browser can swap its `fetch()`-based streaming
//! for a true bidirectional channel.
//!
//! ## Wire protocol
//!
//! The endpoint uses the same `WsFrame` envelope as `/api/ws`
//! (see `websocket.rs`). The flow is:
//!
//! 1. Server sends `event: connect.challenge` (no `id`, no
//!    `method`) — the client must answer with a `req: connect`
//!    frame carrying the bearer token.
//! 2. Server replies `res: connect` with `ok: true`.
//! 3. Client sends `req: chat.send` with params
//!    `{ agent_id, message, session_key, model? }`.
//! 4. Server starts the agent turn on a background task. While
//!    the turn runs, it forwards every `AgentEvent` from the
//!    shared event hub (filtered by `session_key`) as an
//!    `event: chat.delta | chat.thinking | chat.tool_call |
//!    chat.tool_result | chat.error` frame. The `id` in each
//!    frame matches the inbound `req.id` so multiple turns can
//!    be in flight over the same socket without interleaving.
//! 5. The agent's terminal `Done` event closes the stream.
//! 6. Server awaits the background task and surfaces any
//!    error as a final `chat.error` frame.
//!
//! Unknown methods get a structured `res` with `ok: false`.

use super::websocket::{respond_error, respond_ok, send_frame, WsError, WsFrame};
use super::ApiState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use cleanclaw_agent::AgentEvent;
use cleanclaw_store::models::SessionMessageRecord;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};

/// Inbound `chat.send` params.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatSendParams {
    pub agent_id: String,
    pub message: String,
    pub session_key: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Optional override for the owner / chatter user id. When
    /// `None`, the authenticated identity's user id is used.
    #[serde(default)]
    pub user_id: Option<String>,
}

/// Outbound event frame. Serializes as a `WsFrame` envelope
/// with `type: "event"`, `event: "chat.<variant>"`, and the
/// payload under the canonical `payload` field.
#[derive(Debug, Clone, Serialize)]
struct ChatEventFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    event: &'static str,
    id: &'a str,
    payload: Value,
}

/// `GET /api/ws/chat` — accept a WebSocket upgrade.
pub async fn chat_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_chat_socket(socket, state))
}

async fn handle_chat_socket(socket: WebSocket, state: ApiState) {
    let (mut tx, mut rx) = socket.split();
    info!("websocket chat client connected");

    // Send the connect challenge (same shape as /api/ws).
    let challenge = WsFrame {
        kind: "event".into(),
        id: None,
        event: Some("connect.challenge".into()),
        method: None,
        params: None,
        ok: None,
        payload: None,
        error: None,
    };
    if let Err(e) = send_frame(&mut tx, &challenge).await {
        error!(?e, "websocket chat: write challenge failed");
        return;
    }

    // First frame must be `connect`. The token is the bearer.
    let first = match rx.next().await {
        Some(Ok(Message::Text(t))) => t,
        Some(Ok(Message::Close(_))) | None => return,
        Some(Ok(other)) => {
            warn!("websocket chat: unexpected first frame: {other:?}");
            return;
        }
        Some(Err(e)) => {
            warn!(?e, "websocket chat: first-frame read error");
            return;
        }
    };
    let first_frame: WsFrame = match serde_json::from_str(&first) {
        Ok(f) => f,
        Err(e) => {
            let _ = respond_error_tx(&mut tx, "", &format!("invalid frame: {e}")).await;
            return;
        }
    };
    if first_frame.kind != "req" || first_frame.method.as_deref() != Some("connect") {
        let _ = respond_error_tx(
            &mut tx,
            first_frame.id.as_deref().unwrap_or(""),
            "expected connect frame first",
        )
        .await;
        return;
    }
    let token = first_frame
        .params
        .as_ref()
        .and_then(|v| v.get("auth"))
        .and_then(|v| v.get("token"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let Some(token) = token else {
        let _ = respond_error_tx(
            &mut tx,
            first_frame.id.as_deref().unwrap_or(""),
            "missing auth.token",
        )
        .await;
        return;
    };
    let ident = match state.auth.resolve(Some(&token), None).await {
        Ok(Some(i)) => i,
        _ => {
            let _ = respond_error_tx(
                &mut tx,
                first_frame.id.as_deref().unwrap_or(""),
                "authentication failed",
            )
            .await;
            return;
        }
    };
    let _ = respond_ok(
        &mut tx,
        first_frame.id.as_deref().unwrap_or(""),
        json!({}),
    )
    .await;

    // Main loop: read `chat.send` frames, drive a turn each.
    while let Some(msg) = rx.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                debug!(?e, "websocket chat: read error");
                return;
            }
        };
        let raw = match msg {
            Message::Text(t) => t,
            Message::Binary(b) => match std::str::from_utf8(&b) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    warn!("websocket chat: binary frame is not utf-8");
                    continue;
                }
            },
            Message::Close(_) => {
                info!("websocket chat: client closed");
                return;
            }
            Message::Ping(_) | Message::Pong(_) => continue,
        };
        let frame: WsFrame = match serde_json::from_str(&raw) {
            Ok(f) => f,
            Err(e) => {
                warn!(?e, "websocket chat: invalid frame");
                continue;
            }
        };
        if frame.kind != "req" {
            continue;
        }
        let method = frame.method.clone().unwrap_or_default();
        let id = frame.id.clone().unwrap_or_default();
        match method.as_str() {
            "chat.send" => {
                let params: ChatSendParams = match frame
                    .params
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                {
                    Some(p) => p,
                    None => {
                        respond_error(&mut tx, &id, "invalid chat.send params").await;
                        continue;
                    }
                };
                if params.agent_id.is_empty() || params.message.is_empty() {
                    respond_error(&mut tx, &id, "agent_id and message are required").await;
                    continue;
                }
                let model = params
                    .model
                    .clone()
                    .unwrap_or_else(|| state.chat.default_model.clone());
                let owner_user_id = params
                    .user_id
                    .clone()
                    .unwrap_or_else(|| ident.user_id.clone());
                run_turn_and_stream(
                    &state,
                    &mut tx,
                    &id,
                    &owner_user_id,
                    &params.agent_id,
                    &model,
                    &params.message,
                    &params.session_key,
                )
                .await;
            }
            other => {
                respond_error(&mut tx, &id, &format!("unknown method: {other}")).await;
            }
        }
    }
}

/// Drive one turn and stream every `AgentEvent` to the client.
/// The connection stays open after the turn — multiple turns can
/// be driven over the same socket.
async fn run_turn_and_stream(
    state: &ApiState,
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    request_id: &str,
    user_id: &str,
    agent_id: &str,
    model: &str,
    message: &str,
    session_key: &str,
) {
    // Send a start frame so the client can render the spinner
    // before the first delta lands.
    let _ = send_event(
        tx,
        request_id,
        "chat.start",
        json!({
            "agent_id": agent_id,
            "session_key": session_key,
            "model": model,
        }),
    )
    .await;

    // Subscribe to the event hub before kicking off the turn.
    // The hub is broadcast — multiple subscribers coexist; we
    // filter on session_key so simultaneous turns on other
    // sockets don't interleave into our stream.
    let mut hub_rx = state.chat.event_hub().subscribe();

    // Drive the turn on a background task. The result is only
    // used to detect end-of-turn; per-event updates come from
    // the hub.
    let chat = state.chat.clone();
    let agent_id_owned = agent_id.to_string();
    let model_owned = model.to_string();
    let user_id_owned = user_id.to_string();
    let session_key_owned = session_key.to_string();
    let message_owned = message.to_string();
    let turn_task = tokio::spawn(async move {
        chat.run(
            &user_id_owned,
            &agent_id_owned,
            &model_owned,
            &message_owned,
            &session_key_owned,
        )
        .await
    });

    // While the turn is running, forward every hub event whose
    // session_key matches the one we kicked off. Stop when we
    // see the terminal `Done` event. Accumulate the content +
    // tool calls + thinking so we can persist them at the end.
    let mut turn_error: Option<String> = None;
    let mut saw_done = false;
    let mut acc_content = String::new();
    let mut acc_thinking = String::new();
    let mut acc_tool_calls: Vec<Value> = Vec::new();
    let mut acc_tool_results: Vec<Value> = Vec::new();
    let mut final_usage: Option<cleanclaw_agent::Usage> = None;
    let mut final_finish = String::new();
    loop {
        match hub_rx.recv().await {
            Ok(env) => {
                if env.session_key != session_key || env.agent_id != agent_id {
                    continue;
                }
                // Disassemble the event up-front so the borrow
                // checker doesn't complain about the partial
                // move on the `matches!` check below.
                let ev_name;
                let payload;
                let is_done;
                match env.event {
                    AgentEvent::Content { delta } => {
                        acc_content.push_str(&delta);
                        ev_name = "chat.delta";
                        payload = json!({"delta": delta});
                        is_done = false;
                    }
                    AgentEvent::Thinking { delta } => {
                        acc_thinking.push_str(&delta);
                        ev_name = "chat.thinking";
                        payload = json!({"delta": delta});
                        is_done = false;
                    }
                    AgentEvent::ToolCall { name, id, arguments } => {
                        let tc = json!({"name": name, "id": id, "arguments": arguments});
                        acc_tool_calls.push(tc.clone());
                        ev_name = "chat.tool_call";
                        payload = json!({"name": name, "id": id, "arguments": arguments});
                        is_done = false;
                    }
                    AgentEvent::ToolResult { id, content, is_error } => {
                        acc_tool_results.push(json!({
                            "id": id,
                            "content": content,
                            "is_error": is_error,
                        }));
                        ev_name = "chat.tool_result";
                        payload = json!({"id": id, "content": content, "is_error": is_error});
                        is_done = false;
                    }
                    AgentEvent::Done { finish_reason, usage } => {
                        final_finish = finish_reason.clone();
                        final_usage = usage.clone();
                        ev_name = "chat.done";
                        payload = json!({"finish_reason": finish_reason, "usage": usage});
                        is_done = true;
                    }
                    AgentEvent::Error { message } => {
                        // The hub error event isn't terminal — the
                        // loop will fall through to blocking and
                        // still publish a Done. Don't break here.
                        ev_name = "chat.error";
                        payload = json!({"message": message});
                        is_done = false;
                    }
                }
                if let Err(e) = send_event(tx, request_id, ev_name, payload).await {
                    warn!(?e, "websocket chat: event write failed");
                    break;
                }
                if is_done {
                    saw_done = true;
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                // The hub buffer dropped events. Send a warning
                // frame so the client knows.
                warn!("websocket chat: hub lagged {n} events");
                let _ = send_event(
                    tx,
                    request_id,
                    "chat.error",
                    json!({"message": format!("hub lagged {n} events")}),
                )
                .await;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                // Hub closed — turn completed without a Done
                // event. Break and let the join handle the
                // outcome.
                break;
            }
        }
    }

    // Wait for the turn task to complete. If it errored, the
    // hub may not have published a Done event with the error
    // message; surface it as a `chat.error` frame.
    let mut final_reply_for_persist = acc_content.clone();
    match turn_task.await {
        Ok(Ok(out)) => {
            if !saw_done {
                // The run path didn't publish a Done event (e.g.
                // it was blocking and short-circuited). Send a
                // synthetic one so the client can flush.
                let _ = send_event(
                    tx,
                    request_id,
                    "chat.done",
                    json!({
                        "finish_reason": out.finish_reason,
                        "usage": out.usage,
                        "synthetic": true,
                    }),
                )
                .await;
                if final_reply_for_persist.is_empty() {
                    final_reply_for_persist = out.reply.clone();
                }
            }
        }
        Ok(Err(e)) => {
            turn_error = Some(e.to_string());
        }
        Err(e) => {
            turn_error = Some(format!("turn task join: {e}"));
        }
    }
    if let Some(msg) = turn_error {
        let _ = send_event(tx, request_id, "chat.error", json!({"message": msg.clone()})).await;
        // Persist the error so the next page load shows it.
        if final_reply_for_persist.is_empty() {
            final_reply_for_persist = format!("[error] {msg}");
        }
    }

    // Persist the user + assistant messages + update the
    // SessionRecord (title preview, message count, updated_at).
    // This is the single source of truth — the SSR /agents/{id}
    // /sessions/{sid} route reads from the same store.
    persist_turn(
        state,
        user_id,
        agent_id,
        session_key,
        message,
        &final_reply_for_persist,
        &acc_thinking,
        &acc_tool_calls,
        &acc_tool_results,
        final_finish.as_str(),
        final_usage.as_ref(),
    )
    .await;
}

/// Persist the just-finished turn. Writes:
/// - the user message (append_session_message, role=user)
/// - the assistant message (append_session_message,
///   role=assistant, with the accumulated content + thinking +
///   tool calls + tool results)
/// - the SessionRecord (upsert) with title preview +
///   message_count += 2 + updated_at.
///
/// Errors are logged + dropped — the user already has the
/// streamed reply on screen; a failed persistence just means
/// the next page load won't see this turn.
async fn persist_turn(
    state: &ApiState,
    user_id: &str,
    agent_id: &str,
    session_key: &str,
    user_text: &str,
    assistant_text: &str,
    thinking: &str,
    tool_calls: &[Value],
    tool_results: &[Value],
    finish_reason: &str,
    usage: Option<&cleanclaw_agent::Usage>,
) {
    if user_text.is_empty() && assistant_text.is_empty() {
        return;
    }
    // Append the user message.
    let user_msg = SessionMessageRecord {
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        session_key: session_key.to_string(),
        seq: 0, // overwritten by the store
        role: "user".to_string(),
        content: user_text.to_string(),
        content_parts: json!([]),
        tool_calls: json!([]),
        tool_call_id: String::new(),
        name: String::new(),
        metadata: json!({}),
        thinking: String::new(),
        raw_assistant: Value::Null,
        origin: "ws_chat".to_string(),
        created_at: Utc::now(),
        chatter_user_id: user_id.to_string(),
    };
    if let Err(e) = state.store.append_session_message(&user_msg).await {
        warn!(?e, user_id, agent_id, session_key, "persist user msg failed");
    }

    // Append the assistant message.
    let assistant_msg = SessionMessageRecord {
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        session_key: session_key.to_string(),
        seq: 0,
        role: "assistant".to_string(),
        content: assistant_text.to_string(),
        content_parts: json!([]),
        tool_calls: json!(tool_calls),
        tool_call_id: String::new(),
        name: String::new(),
        metadata: json!({
            "thinking": thinking,
            "tool_results": tool_results,
            "finish_reason": finish_reason,
            "usage": usage,
        }),
        thinking: thinking.to_string(),
        raw_assistant: Value::Null,
        origin: "ws_chat".to_string(),
        created_at: Utc::now(),
        chatter_user_id: user_id.to_string(),
    };
    if let Err(e) = state.store.append_session_message(&assistant_msg).await {
        warn!(?e, user_id, agent_id, session_key, "persist assistant msg failed");
    }

    // Upsert the SessionRecord. The `message_count` is the
    // authoritative count from the `session_messages` archive
    // (we just appended 2 rows to it). The agent's
    // `save_compacted` path also writes a `message_count` (the
    // in-loop working-set size), so we explicitly OVERWRITE
    // that field with the archive count to keep the two in
    // sync. Without this, the count would drift by +2 every
    // turn because `save_compacted` runs *before* our
    // `persist_turn` and our naive +2 increment would compound.
    let existing = state
        .store
        .get_session(user_id, agent_id, session_key)
        .await
        .ok();
    let archive_count = state
        .store
        .list_session_messages(user_id, agent_id, session_key)
        .await
        .map(|v| v.len() as i32)
        .unwrap_or(0);
    let now = Utc::now();
    let title = existing
        .as_ref()
        .map(|r| r.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            // Use the first ~50 chars of the user message as
            // the auto-title.
            let trimmed: String = user_text.chars().take(50).collect();
            if trimmed.len() < user_text.len() {
                format!("{trimmed}…")
            } else {
                trimmed
            }
        });
    let preview: String = assistant_text.chars().take(120).collect();
    let rec = cleanclaw_store::models::SessionRecord {
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        session_key: session_key.to_string(),
        channel: "ws".to_string(),
        account_id: String::new(),
        chat_id: session_key.to_string(),
        project_id: existing
            .as_ref()
            .map(|r| r.project_id.clone())
            .unwrap_or_default(),
        title,
        messages: json!([user_text, assistant_text]),
        message_count: archive_count,
        updated_at: now,
        chatter_user_id: user_id.to_string(),
    };
    // The store saves by the (user_id, agent_id, session_key)
    // triple and reuses the existing row. We pass the new
    // record; the chat_id / preview are also surfaced via the
    // SessionMeta when the SSR list page loads.
    let _ = preview;
    if let Err(e) = state
        .store
        .save_session(user_id, agent_id, session_key, &rec)
        .await
    {
        warn!(?e, user_id, agent_id, session_key, "persist session record failed");
    }
}

async fn send_event(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    id: &str,
    event: &'static str,
    payload: Value,
) -> Result<(), axum::Error> {
    let frame = WsFrame {
        kind: "event".into(),
        id: Some(id.to_string()),
        event: Some(event.to_string()),
        method: None,
        params: None,
        ok: None,
        payload: Some(payload),
        error: None,
    };
    super::websocket::send_frame(tx, &frame).await
}

async fn respond_error_tx(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    id: &str,
    msg: &str,
) -> Result<(), axum::Error> {
    let frame = WsFrame {
        kind: "res".into(),
        id: Some(id.to_string()),
        event: None,
        method: None,
        params: None,
        ok: Some(false),
        payload: None,
        error: Some(WsError {
            message: msg.to_string(),
        }),
    };
    super::websocket::send_frame(tx, &frame).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_send_params_deserialize_minimal() {
        let raw = json!({
            "agent_id": "a1",
            "message": "hello",
            "session_key": "s1"
        });
        let p: ChatSendParams = serde_json::from_value(raw).unwrap();
        assert_eq!(p.agent_id, "a1");
        assert_eq!(p.message, "hello");
        assert_eq!(p.session_key, "s1");
        assert!(p.model.is_none());
        assert!(p.user_id.is_none());
    }

    #[test]
    fn chat_send_params_deserialize_full() {
        let raw = json!({
            "agent_id": "a1",
            "message": "hello",
            "session_key": "s1",
            "model": "openai/gpt-4o",
            "user_id": "u_override"
        });
        let p: ChatSendParams = serde_json::from_value(raw).unwrap();
        assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(p.user_id.as_deref(), Some("u_override"));
    }

    #[test]
    fn chat_event_serializes_with_type_event() {
        let env = ChatEventFrame {
            kind: "event",
            event: "chat.delta",
            id: "r1",
            payload: json!({"delta": "hi"}),
        };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"type\":\"event\""));
        assert!(s.contains("\"event\":\"chat.delta\""));
        assert!(s.contains("\"id\":\"r1\""));
        assert!(s.contains("\"delta\":\"hi\""));
    }

    #[test]
    fn chat_event_serializes_tool_call_payload() {
        let env = ChatEventFrame {
            kind: "event",
            event: "chat.tool_call",
            id: "r2",
            payload: json!({
                "name": "echo",
                "id": "tc1",
                "arguments": {"text": "hi"}
            }),
        };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"event\":\"chat.tool_call\""));
        assert!(s.contains("\"name\":\"echo\""));
        assert!(s.contains("\"id\":\"tc1\""));
    }

    #[test]
    fn chat_event_serializes_done_with_usage() {
        let env = ChatEventFrame {
            kind: "event",
            event: "chat.done",
            id: "r3",
            payload: json!({
                "finish_reason": "stop",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "cache_read_tokens": 0,
                    "cache_creation_tokens": 0
                }
            }),
        };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"event\":\"chat.done\""));
        assert!(s.contains("\"finish_reason\":\"stop\""));
        assert!(s.contains("\"input_tokens\":10"));
    }

    #[test]
    fn chat_event_serializes_error() {
        let env = ChatEventFrame {
            kind: "event",
            event: "chat.error",
            id: "r4",
            payload: json!({"message": "boom"}),
        };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"event\":\"chat.error\""));
        assert!(s.contains("\"message\":\"boom\""));
    }
}
