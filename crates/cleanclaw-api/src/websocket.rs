//! WebSocket RPC bridge for the OpenClaw protocol.
//!
//! The wire protocol
//! is a small JSON envelope:
//!
//! ```json
//! { "type": "req"|"res"|"event", "id": "…", "method": "…", "params": …,
//!   "ok": true|false, "payload": …, "error": { "message": "…" },
//!   "event": "connect.challenge" }
//! ```
//!
//! The server upgrades an HTTP request to a WebSocket, sends a
//! `connect.challenge` event, then loops on inbound `req` frames. The
//! only verb the offline bridge implements today is `connect` (Bearer
//! auth) and `agents.list`; unknown methods get a structured
//! `"unknown method"` error so callers can fall back gracefully.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use cleanclaw_auth::{Identity, Resolver};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Bridge state. Currently only the auth resolver is wired; the
/// per-user agent listing is computed off the same store.
#[derive(Clone)]
pub struct WsState {
    pub auth: Arc<Resolver>,
    pub store: Arc<dyn cleanclaw_store::Store>,
}

impl WsState {
    pub fn new(auth: Arc<Resolver>, store: Arc<dyn cleanclaw_store::Store>) -> Self {
        Self { auth, store }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsFrame {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WsError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsError {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectParams {
    pub auth: ConnectAuth,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectAuth {
    pub token: String,
}

/// `GET /api/ws` — accept a WebSocket upgrade.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WsState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WsState) {
    let (mut tx, mut rx) = socket.split();
    info!("websocket client connected");

    // Send connect challenge
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
        error!(?e, "websocket write challenge failed");
        return;
    }

    let mut authenticated = false;
    let mut ws_ident: Option<Identity> = None;

    while let Some(msg) = rx.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                debug!(?e, "websocket read error");
                return;
            }
        };
        let raw = match msg {
            Message::Text(t) => t,
            Message::Binary(b) => match std::str::from_utf8(&b) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    warn!("websocket binary frame is not utf-8");
                    continue;
                }
            },
            Message::Close(_) => {
                info!("websocket client closed");
                return;
            }
            Message::Ping(_) | Message::Pong(_) => continue,
        };
        let frame: WsFrame = match serde_json::from_str(&raw) {
            Ok(f) => f,
            Err(e) => {
                warn!(?e, "websocket invalid frame");
                continue;
            }
        };
        if frame.kind != "req" {
            continue;
        }
        let method = frame.method.clone().unwrap_or_default();
        let id = frame.id.clone().unwrap_or_default();
        match method.as_str() {
            "connect" => {
                let params: ConnectParams = match frame
                    .params
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                {
                    Some(p) => p,
                    None => {
                        respond_error(&mut tx, &id, "invalid connect params").await;
                        continue;
                    }
                };
                match state.auth.resolve(Some(&params.auth.token), None).await {
                    Ok(Some(ident)) => {
                        ws_ident = Some(ident);
                        authenticated = true;
                        respond_ok(&mut tx, &id, json!({})).await;
                    }
                    _ => {
                        respond_error(&mut tx, &id, "authentication failed").await;
                    }
                }
            }
            "agents.list" => {
                if !authenticated {
                    respond_error(&mut tx, &id, "not authenticated").await;
                    continue;
                }
                let ident = ws_ident.as_ref().expect("authenticated");
                let payload = match build_agent_list(&state, ident).await {
                    Ok(v) => v,
                    Err(e) => {
                        respond_error(&mut tx, &id, &format!("agent list failed: {e}")).await;
                        continue;
                    }
                };
                respond_ok(&mut tx, &id, payload).await;
            }
            other => {
                respond_error(&mut tx, &id, &format!("unknown method: {other}")).await;
            }
        }
    }
}

async fn build_agent_list(
    state: &WsState,
    ident: &Identity,
) -> Result<Value, cleanclaw_core::CleanClawError> {
    let rows = if ident.can_admin_platform() {
        state.store.list_all_agents().await?
    } else {
        state.store.list_agents(&ident.user_id).await?
    };
    let list: Vec<Value> = rows
        .into_iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "user_id": a.user_id,
                "is_public": a.is_public,
            })
        })
        .collect();
    Ok(json!({ "agents": list }))
}

pub(crate) async fn send_frame(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: &WsFrame,
) -> Result<(), axum::Error> {
    let s = serde_json::to_string(frame).map_err(axum::Error::new)?;
    tx.send(Message::Text(s)).await
}

pub(crate) async fn respond_ok(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    id: &str,
    payload: Value,
) {
    let frame = WsFrame {
        kind: "res".into(),
        id: Some(id.to_string()),
        event: None,
        method: None,
        params: None,
        ok: Some(true),
        payload: Some(payload),
        error: None,
    };
    if let Err(e) = send_frame(tx, &frame).await {
        error!(?e, "websocket write ok failed");
    }
}

pub(crate) async fn respond_error(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    id: &str,
    msg: &str,
) {
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
    if let Err(e) = send_frame(tx, &frame).await {
        error!(?e, "websocket write error failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a frame the same way the handler does, for round-trip
    /// assertions in tests.
    fn encode(frame: &WsFrame) -> String {
        serde_json::to_string(frame).unwrap()
    }

    #[test]
    fn encode_decode_round_trip_req() {
        let f = WsFrame {
            kind: "req".into(),
            id: Some("42".into()),
            event: None,
            method: Some("connect".into()),
            params: Some(json!({ "auth": { "token": "fk_x" } })),
            ok: None,
            payload: None,
            error: None,
        };
        let s = encode(&f);
        let back: WsFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, "req");
        assert_eq!(back.id.as_deref(), Some("42"));
        assert_eq!(back.method.as_deref(), Some("connect"));
    }

    #[test]
    fn encode_decode_round_trip_res() {
        let f = WsFrame {
            kind: "res".into(),
            id: Some("1".into()),
            event: None,
            method: None,
            params: None,
            ok: Some(true),
            payload: Some(json!({ "agents": [] })),
            error: None,
        };
        let s = encode(&f);
        let back: WsFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(back.ok, Some(true));
        assert_eq!(back.payload.unwrap()["agents"], json!([]));
    }

    #[test]
    fn encode_decode_round_trip_error() {
        let f = WsFrame {
            kind: "res".into(),
            id: Some("9".into()),
            event: None,
            method: None,
            params: None,
            ok: Some(false),
            payload: None,
            error: Some(WsError {
                message: "nope".into(),
            }),
        };
        let s = encode(&f);
        let back: WsFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(back.ok, Some(false));
        assert_eq!(back.error.unwrap().message, "nope");
    }

    #[test]
    fn event_frame_has_no_id_or_method() {
        let f = WsFrame {
            kind: "event".into(),
            id: None,
            event: Some("connect.challenge".into()),
            method: None,
            params: None,
            ok: None,
            payload: None,
            error: None,
        };
        let s = encode(&f);
        // Must not include id/method/ok in the wire form
        assert!(!s.contains("\"id\""));
        assert!(!s.contains("\"method\""));
        assert!(!s.contains("\"ok\""));
        assert!(s.contains("\"event\":\"connect.challenge\""));
    }

    #[test]
    fn connect_params_deserialize() {
        let raw = json!({ "auth": { "token": "fk_test" } });
        let p: ConnectParams = serde_json::from_value(raw).unwrap();
        assert_eq!(p.auth.token, "fk_test");
    }

    #[test]
    fn empty_frame_round_trip() {
        // An empty frame (no fields) deserializes to "event"-with-no-fields
        // variant because of how serde handles missing optionals.
        let s = r#"{"type":"event","event":"x"}"#;
        let f: WsFrame = serde_json::from_str(s).unwrap();
        assert_eq!(f.event.as_deref(), Some("x"));
    }
}
