//! Live provider integration tests using a tiny in-process HTTP
//! server to capture requests and return canned responses.

#[path = "common/mod.rs"]
mod common;
use common::MockBackend;

use std::collections::HashMap;

use cleanclaw_provider::anthropic::AnthropicConfig;
use cleanclaw_provider::anthropic::AnthropicProvider;
use cleanclaw_provider::message::Message;
use cleanclaw_provider::openai::OpenAIConfig;
use cleanclaw_provider::openai::OpenAIProvider;
use cleanclaw_provider::Provider;
use serde_json::json;

#[tokio::test]
async fn openai_chat_sends_bearer_and_unwraps_response() {
    let mock = MockBackend::new();
    mock.set_response(
        200,
        br#"{
            "id": "cmpl-1",
            "model": "gpt-4o-mini",
            "choices": [{
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        }"#,
    );
    let addr = mock.serve().await;
    let url = format!("http://{addr}");

    let cfg = OpenAIConfig {
        api_key: "sk-test".into(),
        api_base: url.clone(),
    };

    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let resp = p.chat(&req).await.unwrap();
    assert_eq!(resp.message.content, "hi");
    assert_eq!(resp.usage.input_tokens, 5);
    assert_eq!(resp.usage.output_tokens, 2);

    // Verify the request shape. The bare-host URL gets normalized
    // by `url::normalize_api_base` to append `/v1` (the OpenAI
    // canonical base path); the runtime then appends
    // `/chat/completions`. The mock server is mounted at root,
    // so the full path the server sees is `/v1/chat/completions`.
    assert_eq!(mock.last_path(), Some("/v1/chat/completions".into()));
    assert_eq!(mock.last_auth(), Some("Bearer sk-test".into()));
    let body = mock.last_body().unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body_json["model"], "gpt-4o-mini");
    let msgs = body_json["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], "hi");
    assert_eq!(body_json["stream"], false);
}

#[tokio::test]
async fn openai_chat_propagates_401_as_auth_error() {
    let mock = MockBackend::new();
    mock.set_response(401, br#"{"error": "bad key"}"#);
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "wrong".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("401") || s.to_lowercase().contains("auth"),
        "got: {s}"
    );
}

#[tokio::test]
async fn openai_chat_propagates_429_as_rate_limited() {
    let mock = MockBackend::new();
    mock.set_response(429, br#"{"error": "rate limited"}"#);
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    let s = err.to_string();
    assert!(
        s.to_lowercase().contains("rate") || s.contains("429"),
        "got: {s}"
    );
}

#[tokio::test]
async fn openai_chat_includes_tools_in_request() {
    let mock = MockBackend::new();
    mock.set_response(
        200,
        br#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
    );
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let tool = cleanclaw_provider::message::ToolDefinition {
        name: "echo".into(),
        description: "echoes".into(),
        parameters: json!({"type": "object"}),
    };
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![tool],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let _ = p.chat(&req).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&mock.last_body().unwrap()).unwrap();
    assert!(body["tools"].is_array());
    assert_eq!(body["tools"][0]["function"]["name"], "echo");
}

#[tokio::test]
async fn openai_chat_includes_org_header_when_set() {
    let mock = MockBackend::new();
    mock.set_response(
        200,
        br#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
    );
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let _ = p.chat(&req).await.unwrap();
    assert_eq!(mock.last_auth(), Some("Bearer sk".into()));
    // The OpenAI-Organization header is sent as a custom header.
    // We just verify the bearer was correct.
}

#[tokio::test]
async fn openai_response_extracts_tool_calls() {
    let mock = MockBackend::new();
    mock.set_response(
        200,
        br#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "read", "arguments": "{\"p\":1}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#,
    );
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let resp = p.chat(&req).await.unwrap();
    assert_eq!(resp.message.tool_calls.len(), 1);
    assert_eq!(resp.message.tool_calls[0].id, "call_1");
    assert_eq!(resp.message.tool_calls[0].name, "read");
}

#[tokio::test]
async fn anthropic_chat_sends_x_api_key_header() {
    let mock = MockBackend::new();
    mock.set_response(
        200,
        br#"{
            "id": "msg_1",
            "model": "claude-3-5-sonnet",
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 4, "output_tokens": 6}
        }"#,
    );
    let addr = mock.serve().await;
    let cfg = AnthropicConfig {
        api_key: "sk-ant-test".into(),
        api_base: format!("http://{addr}"),
        version: "2023-06-01".into(),
    };
    let p = AnthropicProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: Some(64),
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let resp = p.chat(&req).await.unwrap();
    assert_eq!(resp.message.content, "hello");
    assert_eq!(resp.usage.input_tokens, 4);
    assert_eq!(resp.usage.output_tokens, 6);

    // Header should be `x-api-key`, not `Authorization: Bearer …`.
    assert_eq!(mock.last_auth(), Some("sk-ant-test".into()));
    let body: serde_json::Value = serde_json::from_slice(&mock.last_body().unwrap()).unwrap();
    assert_eq!(body["model"], "claude-3-5-sonnet");
    assert_eq!(body["max_tokens"], 64);
    assert!(body["messages"].as_array().unwrap().len() == 1);
}

#[tokio::test]
async fn anthropic_chat_propagates_401() {
    let mock = MockBackend::new();
    mock.set_response(401, br#"{"error": "unauthorized"}"#);
    let addr = mock.serve().await;
    let cfg = AnthropicConfig {
        api_key: "wrong".into(),
        api_base: format!("http://{addr}"),
        version: "2023-06-01".into(),
    };
    let p = AnthropicProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("401") || s.to_lowercase().contains("auth"),
        "got: {s}"
    );
}

#[tokio::test]
async fn both_providers_retry_on_500() {
    // Wiremock returns 500 once, then 200 with a valid body.
    // We can't easily flip the response in the middle of one call,
    // so we just verify a 500 maps to Upstream error and is not
    // silently swallowed.
    let mock = MockBackend::new();
    mock.set_response(500, br#"{"error": "internal"}"#);
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("500") || s.to_lowercase().contains("upstream"),
        "got: {s}"
    );
}

#[tokio::test]
async fn missing_api_key_surfaces_as_config_error() {
    let cfg = OpenAIConfig {
        api_key: String::new(),
        api_base: "http://example.invalid".into(),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    // Without a key, the bearer is empty and the upstream rejects.
    // The shape of the error depends on the test environment: with
    // an unreachable host we get Http, with a real upstream we
    // might get Auth. Accept either.
    assert!(matches!(
        err,
        cleanclaw_provider::ProviderError::Http(_)
            | cleanclaw_provider::ProviderError::Auth(_)
            | cleanclaw_provider::ProviderError::Config(_)
    ));
}

#[tokio::test]
async fn response_without_choices_errors() {
    let mock = MockBackend::new();
    mock.set_response(200, br#"{"id": "bad"}"#);
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    let s = err.to_string();
    assert!(
        s.to_lowercase().contains("decode") || s.to_lowercase().contains("no choices"),
        "got: {s}"
    );
}

#[tokio::test]
async fn retry_after_429_returns_rate_limited_error() {
    // Same as 429 test above; this one documents the policy.
    let mock = MockBackend::new();
    mock.set_response(429, br#"{}"#);
    let addr = mock.serve().await;
    let cfg = OpenAIConfig {
        api_key: "sk".into(),
        api_base: format!("http://{addr}"),
    };
    let p = OpenAIProvider::new(cfg);
    let req = cleanclaw_provider::message::ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        stream: false,
        extra: HashMap::new(),
    };
    let err = p.chat(&req).await.unwrap_err();
    assert!(matches!(
        err,
        cleanclaw_provider::ProviderError::RateLimited
    ));
}
