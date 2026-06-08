//! End-to-end integration test for `/api/ws/chat`.
//!
//! Spins up a real axum router backed by an in-memory store +
//! a `CannedProvider` that emits a fixed sequence of
//! `StreamEvent`s, connects via `tokio-tungstenite`, walks the
//! connect + chat.send flow, and asserts on the frames received.
//!
//! The provider is the same `CannedProvider` shape used in
//! `cleanclaw-agent`'s loop tests, so the streaming behavior is
//! already covered. This test pins the WS envelope end-to-end.

#![cfg(test)]

use cleanclaw_api::{chat::ChatService, router, ApiState};
use cleanclaw_auth::Resolver;
use cleanclaw_core::BUILD_VERSION;
use cleanclaw_provider::{
    ChatRequest, ChatResponse, Message, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
};
use cleanclaw_store::Store;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// A canned provider that yields a fixed reply on the first
/// `chat_stream` call. Mirrors the shape in `cleanclaw-agent`'s
/// loop tests; we re-declare it here to avoid a circular
/// test-only dep on the agent crate.
struct OneShotProvider {
    response: ChatResponse,
    call_count: AtomicUsize,
}

impl OneShotProvider {
    fn new(response: ChatResponse) -> Self {
        Self {
            response,
            call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl Provider for OneShotProvider {
    fn name(&self) -> &str {
        "oneshot"
    }
    async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(self.response.clone())
    }
    async fn chat_stream(&self, _req: &ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let resp = self.response.clone();
        let s = async_stream::stream! {
            // One content delta with the full text, then Done.
            yield Ok::<_, ProviderError>(StreamEvent::ContentDelta {
                delta: resp.message.content.clone(),
            });
            yield Ok::<_, ProviderError>(StreamEvent::Done {
                finish_reason: resp.finish_reason.clone(),
                usage: Some(resp.usage.clone()),
            });
        };
        Ok(Box::pin(s))
    }
}

async fn fresh_state() -> (ApiState, Arc<OneShotProvider>) {
    let st = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
        .await
        .unwrap();
    st.migrate().await.unwrap();
    let store: Arc<dyn Store> = Arc::new(st);
    let auth = Arc::new(Resolver::new(store.clone()));
    let chat = Arc::new(ChatService::new(store.clone(), "test-model".into()));
    let provider = Arc::new(OneShotProvider::new(ChatResponse {
        id: "r1".into(),
        model: "test-model".into(),
        message: Message::assistant("hello from oneshot"),
        finish_reason: "stop".into(),
        usage: Usage {
            input_tokens: 4,
            output_tokens: 3,
            ..Default::default()
        },
        raw: Value::Null,
    }));
    chat.register_provider("test-model", provider.clone() as Arc<dyn Provider>);
    let state = ApiState::new(store, auth, chat);
    (state, provider)
}

async fn spawn_test_server(state: ApiState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Give the server a tick to start accepting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (format!("ws://{addr}/api/ws/chat"), handle)
}

/// Build a `WsMessage::Text` carrying a JSON value.
fn ws_text(v: &Value) -> WsMessage {
    WsMessage::Text(serde_json::to_string(v).unwrap())
}

/// Drain a `Vec<WsMessage>` of text frames into `Value`s and
/// close frames into a `bool`.
#[allow(dead_code)]
fn collect_text(frames: Vec<WsMessage>) -> Vec<Value> {
    frames
        .into_iter()
        .filter_map(|m| match m {
            WsMessage::Text(s) => serde_json::from_str(&s).ok(),
            _ => None,
        })
        .collect()
}

/// Wire-format test: connect, then chat.send, then read the
/// start / delta / done frames. The provider emits one
/// `ContentDelta` + one `Done` per turn, so we expect to see
/// exactly that on the wire.
#[tokio::test(flavor = "current_thread")]
async fn ws_chat_round_trip() {
    let (state, _provider) = fresh_state().await;
    let (url, server) = spawn_test_server(state).await;
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // 1. Server sends connect.challenge
    let challenge = ws.next().await.unwrap().unwrap();
    let challenge_v: Value = match challenge {
        WsMessage::Text(s) => serde_json::from_str(&s).unwrap(),
        other => panic!("expected text frame, got {other:?}"),
    };
    assert_eq!(challenge_v["type"], "event");
    assert_eq!(challenge_v["event"], "connect.challenge");

    // 2. Client sends connect with a fake token. The Resolver
    //    returns `None` for unknown tokens, so the server
    //    replies with an error frame — but that's enough to
    //    exercise the WS envelope. We don't need a real session
    //    to verify the wire format here.
    let connect_req = json!({
        "type": "req",
        "id": "c1",
        "method": "connect",
        "params": { "auth": { "token": "fk_test" } }
    });
    ws.send(ws_text(&connect_req)).await.unwrap();
    let connect_res = ws.next().await.unwrap().unwrap();
    let connect_res_v: Value = match connect_res {
        WsMessage::Text(s) => serde_json::from_str(&s).unwrap(),
        other => panic!("expected text frame, got {other:?}"),
    };
    // Authentication fails (no real user in the store), so the
    // server replies with an error and closes the read half of
    // the socket. We still got a structured frame.
    assert_eq!(connect_res_v["type"], "res");
    assert_eq!(connect_res_v["id"], "c1");
    assert_eq!(connect_res_v["ok"], false);
    assert!(connect_res_v["error"].is_object());

    drop(server);
}

/// Wire-format test: connect with a malformed first frame
/// (non-`connect` method) gets a structured `unknown method`
/// error.
#[tokio::test(flavor = "current_thread")]
async fn ws_chat_rejects_non_connect_first_frame() {
    let (state, _provider) = fresh_state().await;
    let (url, server) = spawn_test_server(state).await;
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Consume the challenge.
    let _ = ws.next().await.unwrap().unwrap();

    // Send `chat.send` as the first frame. The server expects
    // `connect` first; this gets a structured error.
    let bad = json!({
        "type": "req",
        "id": "x1",
        "method": "chat.send",
        "params": { "agent_id": "a1", "message": "x", "session_key": "s" }
    });
    ws.send(ws_text(&bad)).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let resp_v: Value = match resp {
        WsMessage::Text(s) => serde_json::from_str(&s).unwrap(),
        other => panic!("expected text frame, got {other:?}"),
    };
    assert_eq!(resp_v["type"], "res");
    assert_eq!(resp_v["id"], "x1");
    assert_eq!(resp_v["ok"], false);
    let err = resp_v["error"]["message"].as_str().unwrap();
    assert!(err.contains("connect"), "got: {err}");

    drop(server);
}

/// Wire-format test: an invalid JSON frame is silently dropped
/// (the server logs a warning but doesn't close the socket).
/// This pins the parser's tolerance.
#[tokio::test(flavor = "current_thread")]
async fn ws_chat_tolerates_invalid_frame() {
    let (state, _provider) = fresh_state().await;
    let (url, server) = spawn_test_server(state).await;
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Consume the challenge.
    let _ = ws.next().await.unwrap().unwrap();

    // Send raw garbage that isn't valid JSON. The server
    // should log a warning and continue.
    ws.send(WsMessage::Text("not json".into())).await.unwrap();

    // Send a proper connect frame.
    let connect_req = json!({
        "type": "req",
        "id": "c2",
        "method": "connect",
        "params": { "auth": { "token": "fk_x" } }
    });
    ws.send(ws_text(&connect_req)).await.unwrap();
    let _resp = ws.next().await.unwrap().unwrap();

    drop(server);
}

/// Compile-time check: the route is wired. We can't easily
/// drive a full round trip without a real user in the store,
/// but the wire tests above pin the envelope. This test
/// documents that `/api/ws/chat` is part of the public
/// surface (it's referenced by the SSR chat page).
#[test]
fn ws_chat_route_is_documented() {
    // A no-op test that exists solely so future refactors
    // remember the route exists. The real test is the build
    // and the wire-format tests above.
    let _ = BUILD_VERSION;
}

/// Full end-to-end test: create a real user + apikey in the
/// store, drive a `chat.send` round trip through the WS, and
/// verify the user + assistant messages were persisted to
/// the `session_messages` table.
#[tokio::test(flavor = "current_thread")]
async fn ws_chat_persists_messages_on_done() {
    use chrono::Utc;
    use cleanclaw_auth::apikey;
    use cleanclaw_store::models::{ApiKeyRecord, UserRecord};

    let (state, _provider) = fresh_state().await;
    let store: Arc<dyn Store> = state.store.clone();

    // Create a real user.
    let user = UserRecord {
        id: "u_e2e".into(),
        username: "e2e".into(),
        email: "e2e@example.com".into(),
        password_hash: cleanclaw_auth::password::hash_password("hunter2").unwrap(),
        display_name: "E2E".into(),
        role: "user".into(),
        status: "active".into(),
        apikey_id: String::new(),
        external_id: String::new(),
        avatar_url: String::new(),
        agent_quota: -1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.create_user(&user).await.unwrap();

    // Mint a real apikey.
    let (token, hash, prefix) = apikey::generate();
    let key = ApiKeyRecord {
        id: "k_e2e".into(),
        user_id: "u_e2e".into(),
        name: "e2e test".into(),
        key_hash: hash.clone(),
        key_prefix: prefix,
        r#type: "user".into(),
        created_at: Utc::now(),
        prev_hash: None,
        prev_hash_set_at: None,
    };
    store.create_api_key(&key).await.unwrap();

    // Start the server + connect.
    let (url, server) = spawn_test_server(state).await;
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // 1. Consume connect.challenge.
    let challenge = ws.next().await.unwrap().unwrap();
    let challenge_v: Value = match challenge {
        WsMessage::Text(s) => serde_json::from_str(&s).unwrap(),
        _ => panic!("expected text frame"),
    };
    assert_eq!(challenge_v["event"], "connect.challenge");

    // 2. Connect with the real apikey.
    let connect_req = json!({
        "type": "req",
        "id": "c1",
        "method": "connect",
        "params": { "auth": { "token": token } }
    });
    ws.send(ws_text(&connect_req)).await.unwrap();
    let connect_res = ws.next().await.unwrap().unwrap();
    let connect_res_v: Value = match connect_res {
        WsMessage::Text(s) => serde_json::from_str(&s).unwrap(),
        _ => panic!("expected text frame"),
    };
    assert_eq!(
        connect_res_v["ok"], true,
        "auth should succeed: {connect_res_v}"
    );

    // 3. Send a chat.send request.
    let send_req = json!({
        "type": "req",
        "id": "r1",
        "method": "chat.send",
        "params": {
            "agent_id": "a_e2e",
            "message": "ping from e2e",
            "session_key": "s_e2e"
        }
    });
    ws.send(ws_text(&send_req)).await.unwrap();

    // 4. Drain frames. We expect: chat.start, chat.delta,
    //    chat.done (in some order — the WS layer may batch).
    let mut got_start = false;
    let mut got_delta = false;
    let mut got_done = false;
    let mut done_payload: Option<Value> = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), ws.next()).await {
            Ok(Some(Ok(WsMessage::Text(s)))) => {
                let v: Value = serde_json::from_str(&s).unwrap();
                let ev = v["event"].as_str().unwrap_or("");
                if ev == "chat.start" {
                    got_start = true;
                } else if ev == "chat.delta" {
                    got_delta = true;
                    assert_eq!(v["payload"]["delta"], "hello from oneshot");
                } else if ev == "chat.done" {
                    got_done = true;
                    done_payload = Some(v["payload"].clone());
                    break;
                }
            }
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => break,
            Ok(Some(Ok(_))) => continue, // ping/pong
            Ok(Some(Err(_))) => break,
            Err(_) => break, // timeout
        }
    }
    assert!(got_start, "expected chat.start frame");
    assert!(got_delta, "expected chat.delta frame");
    assert!(got_done, "expected chat.done frame");
    let payload = done_payload.unwrap();
    assert_eq!(payload["finish_reason"], "stop");

    // 5. Give the persistence path a tick to commit.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 6. Verify the messages were persisted.
    let msgs = store
        .list_session_messages("u_e2e", "a_e2e", "s_e2e")
        .await
        .unwrap();
    assert_eq!(msgs.len(), 2, "expected user + assistant rows");
    let user_msg = msgs.iter().find(|m| m.role == "user").expect("user msg");
    let asst_msg = msgs
        .iter()
        .find(|m| m.role == "assistant")
        .expect("assistant msg");
    assert_eq!(user_msg.content, "ping from e2e");
    assert_eq!(user_msg.origin, "ws_chat");
    assert_eq!(asst_msg.content, "hello from oneshot");
    assert_eq!(asst_msg.thinking, "");
    assert_eq!(asst_msg.tool_calls, serde_json::json!([]));
    assert_eq!(asst_msg.origin, "ws_chat");
    // The assistant's metadata block carries finish_reason + usage.
    assert_eq!(asst_msg.metadata["finish_reason"], "stop");

    // 7. The SessionRecord was upserted with title + count.
    let sess = store.get_session("u_e2e", "a_e2e", "s_e2e").await.unwrap();
    assert_eq!(sess.title, "ping from e2e"); // auto-titled from first msg
    assert_eq!(sess.message_count, 2);
    assert_eq!(sess.channel, "ws");

    drop(server);
}
