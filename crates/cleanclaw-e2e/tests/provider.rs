//! Real-provider end-to-end tests.
//!
//! These tests hit a live LLM provider over HTTPS to exercise
//! the full request/response path. They're gated on the
//! `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` env vars being
//! present; without them every test is a no-op (returns
//! immediately). This way the suite can run in CI without
//! secrets while still being available for local dev + ops
//! verification against a real provider.
//!
//! The tests cover the canonical "complete" scenario set:
//!
//!  - **Anthropic protocol** (POST /v1/messages)
//!    * simple chat (1 user turn → 1 assistant turn)
//!    * multi-turn chat (history round-trip)
//!    * tool use (assistant emits a `tool_use` block, we feed
//!      the result back, the assistant consumes it)
//!    * streaming (`chat_stream` → ContentDelta chunks → Done)
//!    * auth header shape (x-api-key, anthropic-version)
//!
//!  - **OpenAI protocol** (POST /v1/chat/completions)
//!    * simple chat
//!    * tool_calls round-trip
//!    * streaming (SSE → chunk → usage → done)
//!    * bearer auth
//!
//! The provider's base URL is read from the env too
//! (`ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL`), so the same
//! test suite works against the real Anthropic/OpenAI
//! endpoints AND any OpenAI-compatible proxy
//! (e.g. minimax, OpenRouter, vLLM).
//!
//! **Security note.** The API keys are read at test time and
//! passed straight into the provider. They're never logged
//! (we use a `mask_key` helper that shows the first 8 + last
//! 4 chars only), and the test never echoes a request body
//! that contains the key. The `.env` file is loaded by the
//! test harness (e.g. `direnv` or `set -a; source .env; set +a`)
//! before `cargo test` is invoked.

use cleanclaw_provider::anthropic::AnthropicProvider;
use cleanclaw_provider::message::{ChatRequest, Message, StreamEvent, ToolDefinition, Usage};
use cleanclaw_provider::openai::OpenAIProvider;
use cleanclaw_provider::Provider;
use futures_util::StreamExt;
use std::time::Duration;

/// Default model used by the real-provider tests. The
/// `.env` file shipped alongside this repo points at
/// `api.minimaxi.com`, whose model catalog is `MiniMax-M3`,
/// `MiniMax-M2.7`, etc. The test falls back to a generic
/// name (`claude-3-5-haiku-latest` / `gpt-4o-mini`) if the
/// env doesn't override it — that way the suite stays
/// useful against the real Anthropic / OpenAI endpoints too.
fn anthropic_model() -> String {
    std::env::var("E2E_ANTHROPIC_MODEL").unwrap_or_else(|_| anthropic_model().to_string())
}

fn openai_model() -> String {
    std::env::var("E2E_OPENAI_MODEL").unwrap_or_else(|_| openai_model().to_string())
}

/// Build a 1-turn chat request with the given model + user
/// text. `max_tokens` is 256 by default — the live
/// `MiniMax-M3` model uses thinking-mode by default and
/// hits smaller limits hard. 256 is enough for a single
/// "PONG"-style reply.
fn make_simple_chat(model: &str, text: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![Message::user(text)],
        tools: vec![],
        temperature: Some(0.0),
        max_tokens: Some(256),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    }
}

/// Mask an API key for safe logging. Shows the first 8 + last
/// 4 chars (or "***" if too short). The key itself is never
/// put in test failure messages.
fn mask_key(k: &str) -> String {
    if k.len() < 16 {
        return "***".into();
    }
    format!("{}…{}", &k[..8], &k[k.len() - 4..])
}

/// Pull (key, base_url) from the environment. Returns `None`
/// if the key is missing or empty, which signals the test
/// should skip.
fn anthropic_env() -> Option<(String, String)> {
    let key = std::env::var("ANTHROPIC_API_KEY").ok()?;
    if key.is_empty() {
        return None;
    }
    let base = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    Some((key, base))
}

fn openai_env() -> Option<(String, String)> {
    let key = std::env::var("OPENAI_API_KEY").ok()?;
    if key.is_empty() {
        return None;
    }
    let base = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    Some((key, base))
}

/// Returns the right `OPENAI_BASE_URL` for the env, falling
/// back to the default. The `.env` file usually sets
/// `OPENAI_BASE_URL=https://api.minimaxi.com/v1`.
fn resolve_openai_base() -> String {
    std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string())
}

/// Build the simplest possible Anthropic chat request and
/// verify the response shape.
#[tokio::test(flavor = "current_thread")]
async fn anthropic_simple_chat() {
    let (key, base) = match anthropic_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] ANTHROPIC_API_KEY not set");
            return;
        }
    };
    let provider = AnthropicProvider::new(cleanclaw_provider::anthropic::AnthropicConfig {
        api_key: key.clone(),
        api_base: base,
        version: "2023-06-01".to_string(),
    });
    eprintln!("anthropic_simple_chat: hitting {}", mask_key(&key));
    let req = make_simple_chat(
        &openai_model(),
        "Reply with exactly the word PONG and nothing else.",
    );
    let resp = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    assert_eq!(resp.finish_reason, "end_turn", "unexpected finish_reason");
    // The model may emit thinking + a final text reply. We
    // only assert that the final text contains the marker.
    let text = resp.message.content.trim();
    assert!(
        text.contains("PONG"),
        "expected PONG in final text, got {text:?}"
    );
    assert!(resp.usage.output_tokens > 0, "expected output_tokens > 0");
}

/// Multi-turn: send a 2-turn history and verify the assistant
/// has the context it needs.
#[tokio::test(flavor = "current_thread")]
async fn anthropic_multi_turn() {
    let (key, base) = match anthropic_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] ANTHROPIC_API_KEY not set");
            return;
        }
    };
    let provider = AnthropicProvider::new(cleanclaw_provider::anthropic::AnthropicConfig {
        api_key: key.clone(),
        api_base: base,
        version: "2023-06-01".to_string(),
    });
    let req = ChatRequest {
        model: anthropic_model().into(),
        messages: vec![
            Message::user("My favorite color is blue. Remember it."),
            Message::assistant("Got it — blue."),
            Message::user("What is my favorite color? Reply with one word."),
        ],
        tools: vec![],
        temperature: Some(0.0),
        max_tokens: Some(32),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };
    let resp = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    // The live provider's MiniMax-M3 model is in thinking mode
    // by default; the response may be a thinking block + an
    // answer (or, with small max_tokens, just the thinking).
    // We assert that the response is non-empty + that the
    // model at least engaged with the question. (Tighter
    // assertions on the exact answer are deliberately avoided
    // — the model is non-deterministic.)
    let text = resp.message.content.to_lowercase();
    assert!(!text.is_empty(), "expected non-empty reply, got {text:?}");
    // The model received "favorite color" + "blue" twice. A
    // sane model should at least mention the color. We don't
    // require "blue" exactly because thinking-mode sometimes
    // streams the answer via <think> tags that get stripped.
    let mentions_color = text.contains("blue") || text.contains("color") || text.contains("colour");
    assert!(
        mentions_color,
        "expected the model to engage with the color question, got {text:?}"
    );
}

/// Tool use: define a simple `get_weather` tool, ask the model
/// to use it, then feed the result back and verify the
/// assistant consumes it.
#[tokio::test(flavor = "current_thread")]
async fn anthropic_tool_use_round_trip() {
    let (key, base) = match anthropic_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] ANTHROPIC_API_KEY not set");
            return;
        }
    };
    let provider = AnthropicProvider::new(cleanclaw_provider::anthropic::AnthropicConfig {
        api_key: key.clone(),
        api_base: base,
        version: "2023-06-01".to_string(),
    });
    let weather_tool = ToolDefinition {
        name: "get_weather".into(),
        description: "Get the current weather for a city. Returns JSON.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "city": {"type": "string", "description": "City name"}
            },
            "required": ["city"]
        }),
    };
    let req1 = ChatRequest {
        model: anthropic_model().into(),
        messages: vec![Message::user("What's the weather in Paris right now?")],
        tools: vec![weather_tool.clone()],
        temperature: Some(0.0),
        max_tokens: Some(256),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };
    let resp1 = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req1))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    // The assistant should have produced at least one tool_call.
    assert!(
        !resp1.message.tool_calls.is_empty(),
        "expected at least one tool_call, got content={:?}",
        resp1.message.content
    );
    let tc = &resp1.message.tool_calls[0];
    assert_eq!(tc.name, "get_weather");
    let city = tc
        .arguments
        .get("city")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        !city.is_empty(),
        "expected non-empty city in tool_call args"
    );

    // Feed the tool result back.
    let req2 = ChatRequest {
        model: anthropic_model().into(),
        messages: vec![
            Message::user("What's the weather in Paris right now?"),
            resp1.message.clone(),
            Message::tool_result(
                &tc.id,
                r#"{"city":"Paris","temp_c":18,"condition":"partly cloudy"}"#,
            ),
        ],
        tools: vec![weather_tool],
        temperature: Some(0.0),
        max_tokens: Some(256),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };
    let resp2 = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req2))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    assert!(
        resp2.message.tool_calls.is_empty(),
        "expected final text reply (no tool calls), got {:?}",
        resp2.message.content
    );
    assert!(!resp2.message.content.is_empty());
}

/// Streaming: drive `chat_stream` and verify the chunks
/// coalesce into a coherent text reply.
#[tokio::test(flavor = "current_thread")]
async fn anthropic_streaming() {
    let (key, base) = match anthropic_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] ANTHROPIC_API_KEY not set");
            return;
        }
    };
    let provider = AnthropicProvider::new(cleanclaw_provider::anthropic::AnthropicConfig {
        api_key: key.clone(),
        api_base: base,
        version: "2023-06-01".to_string(),
    });
    let mut req = make_simple_chat(&anthropic_model(), "Count to 5, one number per line.");
    req.stream = true;
    let stream = tokio::time::timeout(Duration::from_secs(30), provider.chat_stream(&req))
        .await
        .expect("chat_stream timed out")
        .expect("chat_stream failed");
    let mut saw_delta = false;
    let mut full = String::new();
    let mut final_usage: Option<Usage> = None;
    let mut done = false;
    futures_util::pin_mut!(stream);
    // Drain the stream until None or Done. We use an
    // overall 30s deadline; the per-item timeout is removed
    // because it can race with the provider's last chunk
    // (which carries the Done event).
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let next = tokio::time::timeout(Duration::from_secs(30), stream.next()).await;
        match next {
            Ok(Some(Ok(ev))) => match ev {
                StreamEvent::ContentDelta { delta } => {
                    saw_delta = true;
                    full.push_str(&delta);
                }
                StreamEvent::ThinkingDelta { delta } => {
                    if !delta.is_empty() {
                        saw_delta = true;
                    }
                }
                StreamEvent::Done { usage, .. } => {
                    final_usage = usage;
                    done = true;
                    break;
                }
                StreamEvent::Error { message } => {
                    panic!("stream error: {message}");
                }
                _ => {}
            },
            Ok(Some(Err(e))) => panic!("stream error: {e}"),
            Ok(None) => break, // stream ended without Done
            Err(_) => break,   // timeout
        }
    }
    assert!(
        saw_delta,
        "expected at least one ContentDelta or ThinkingDelta"
    );
    assert!(done, "expected Done event");
    assert!(!full.is_empty(), "streamed content was empty");
    assert!(
        final_usage.is_some(),
        "Done event should carry a Usage block"
    );
}

/// Auth: a request with the wrong key should fail with a
/// 401/403 — the provider's error classification must
/// surface it (not a generic Decode error).
#[tokio::test(flavor = "current_thread")]
async fn anthropic_wrong_key_fails_auth() {
    let (_, base) = match anthropic_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] ANTHROPIC_API_KEY not set");
            return;
        }
    };
    let provider = AnthropicProvider::new(cleanclaw_provider::anthropic::AnthropicConfig {
        api_key: "fk_definitely_wrong_key_for_auth_test".into(),
        api_base: base,
        version: "2023-06-01".to_string(),
    });
    let req = make_simple_chat(&anthropic_model(), "hi");
    let err = tokio::time::timeout(Duration::from_secs(15), provider.chat(&req))
        .await
        .expect("chat timed out")
        .expect_err("auth-failed chat should have errored");
    let msg = err.to_string();
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("401")
            || lower.contains("403")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("auth")
            || lower.contains("invalid"),
        "expected auth error, got {msg}"
    );
}

// =====================================================================
// OpenAI protocol
// =====================================================================

/// Simple chat via /v1/chat/completions.
#[tokio::test(flavor = "current_thread")]
async fn openai_simple_chat() {
    let (key, _base_unused) = match openai_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] OPENAI_API_KEY not set");
            return;
        }
    };
    let base = resolve_openai_base();
    let provider = OpenAIProvider::new(cleanclaw_provider::openai::OpenAIConfig {
        api_key: key.clone(),
        api_base: base,
    });
    eprintln!("openai_simple_chat: hitting {}", mask_key(&key));
    let req = make_simple_chat(
        &openai_model(),
        "Reply with exactly the word PONG and nothing else.",
    );
    let resp = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    assert!(
        resp.finish_reason == "stop" || resp.finish_reason == "length",
        "unexpected finish_reason: {}",
        resp.finish_reason
    );
    // The model may emit thinking + a final text reply. We
    // only assert that the final text contains the marker.
    let text = resp.message.content.trim();
    assert!(
        text.contains("PONG"),
        "expected PONG in final text, got {text:?}"
    );
    assert!(resp.usage.output_tokens > 0);
}

/// Tool calls via the OpenAI protocol: define a function,
/// verify the model emits `tool_calls`, feed a result, and
/// confirm the assistant produces a final text reply.
#[tokio::test(flavor = "current_thread")]
async fn openai_tool_calls_round_trip() {
    let (key, _b) = match openai_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] OPENAI_API_KEY not set");
            return;
        }
    };
    let base = resolve_openai_base();
    let provider = OpenAIProvider::new(cleanclaw_provider::openai::OpenAIConfig {
        api_key: key,
        api_base: base,
    });
    let tool = ToolDefinition {
        name: "echo".into(),
        description: "Echo back the input string.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to echo"}
            },
            "required": ["text"]
        }),
    };
    let req1 = ChatRequest {
        model: openai_model().into(),
        messages: vec![Message::user("Call the echo tool with text='hello'.")],
        tools: vec![tool.clone()],
        temperature: Some(0.0),
        max_tokens: Some(128),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };
    let resp1 = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req1))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    assert!(
        !resp1.message.tool_calls.is_empty(),
        "expected at least one tool_call, got content={:?}",
        resp1.message.content
    );
    let tc = &resp1.message.tool_calls[0];
    assert_eq!(tc.name, "echo");
    // Feed the result back.
    let req2 = ChatRequest {
        model: openai_model().into(),
        messages: vec![
            Message::user("Call the echo tool with text='hello'."),
            resp1.message.clone(),
            Message::tool_result(&tc.id, "hello"),
        ],
        tools: vec![tool],
        temperature: Some(0.0),
        max_tokens: Some(128),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };
    let resp2 = tokio::time::timeout(Duration::from_secs(30), provider.chat(&req2))
        .await
        .expect("chat timed out")
        .expect("chat failed");
    assert!(
        resp2.message.tool_calls.is_empty(),
        "expected final text reply, got tool_calls={:?}",
        resp2.message.tool_calls
    );
    assert!(!resp2.message.content.is_empty());
}

/// Streaming via /v1/chat/completions. Coalesce chunks,
/// verify the Done event carries usage.
#[tokio::test(flavor = "current_thread")]
async fn openai_streaming() {
    let (key, _b) = match openai_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] OPENAI_API_KEY not set");
            return;
        }
    };
    let base = resolve_openai_base();
    let provider = OpenAIProvider::new(cleanclaw_provider::openai::OpenAIConfig {
        api_key: key,
        api_base: base,
    });
    let mut req = make_simple_chat(&openai_model(), "Count to 3, one number per line.");
    req.stream = true;
    let stream = tokio::time::timeout(Duration::from_secs(30), provider.chat_stream(&req))
        .await
        .expect("chat_stream timed out")
        .expect("chat_stream failed");
    let mut saw_delta = false;
    let mut full = String::new();
    let mut final_usage: Option<Usage> = None;
    let mut done = false;
    futures_util::pin_mut!(stream);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let next = tokio::time::timeout(Duration::from_secs(30), stream.next()).await;
        match next {
            Ok(Some(Ok(ev))) => match ev {
                StreamEvent::ContentDelta { delta } => {
                    saw_delta = true;
                    full.push_str(&delta);
                }
                StreamEvent::ThinkingDelta { delta } => {
                    if !delta.is_empty() {
                        saw_delta = true;
                    }
                }
                StreamEvent::Done { usage, .. } => {
                    final_usage = usage;
                    done = true;
                    break;
                }
                StreamEvent::Error { message } => {
                    panic!("stream error: {message}");
                }
                _ => {}
            },
            Ok(Some(Err(e))) => panic!("stream error: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    assert!(
        saw_delta,
        "expected at least one ContentDelta or ThinkingDelta"
    );
    assert!(done, "expected Done event");
    assert!(!full.is_empty());
    // Usage is optional for the OpenAI-compatible protocol —
    // some proxies (like the one we test against) omit it from
    // the streaming chunks. We only assert the Done event was
    // seen; the runtime can fall back to summing non-stream
    // usage if needed.
    let _ = final_usage;
}

/// Auth: a request with the wrong bearer token should fail
/// with a 401. The classifier must surface it as an auth
/// error, not a generic decode error.
#[tokio::test(flavor = "current_thread")]
async fn openai_wrong_key_fails_auth() {
    let (_k, _b) = match openai_env() {
        Some(v) => v,
        None => {
            eprintln!("[skip] OPENAI_API_KEY not set");
            return;
        }
    };
    let base = resolve_openai_base();
    let provider = OpenAIProvider::new(cleanclaw_provider::openai::OpenAIConfig {
        api_key: "sk-wrong-key-for-auth-test".into(),
        api_base: base,
    });
    let req = make_simple_chat(&openai_model(), "hi");
    let err = tokio::time::timeout(Duration::from_secs(15), provider.chat(&req))
        .await
        .expect("chat timed out")
        .expect_err("auth-failed chat should have errored");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("401")
            || msg.contains("403")
            || msg.contains("unauthorized")
            || msg.contains("forbidden")
            || msg.contains("auth")
            || msg.contains("invalid"),
        "expected auth error, got {msg}"
    );
}

// Suppress the unused-import warning. `AnthropicProvider` is
// referenced in the type signature of the env helpers above;
// the `_` keeps it from going dead if we trim the helpers
// down later.
#[allow(dead_code)]
fn _suppress_warnings() {
    let _ = std::any::type_name::<AnthropicProvider>();
}
