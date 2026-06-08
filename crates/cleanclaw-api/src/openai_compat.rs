//! OpenAI-compatible `/v1/chat/completions` endpoint. Supports both
//! streaming (SSE) and non-streaming modes.

use super::ApiState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use cleanclaw_agent::TurnInput;
use cleanclaw_provider::{ChatRequest as ProviderChatRequest, Message, Role, StreamEvent};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::time::Duration;

#[derive(Deserialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub tools: Vec<OpenAITool>,
}

#[derive(Deserialize)]
pub struct OpenAIMessage {
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAIToolCall>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct OpenAIToolCall {
    id: String,
    #[serde(default)]
    r#type: Option<String>,
    function: OpenAIFunction,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct OpenAIFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct OpenAITool {
    r#type: String,
    function: OpenAIToolDef,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct OpenAIToolDef {
    name: String,
    description: String,
    parameters: Value,
}

pub async fn chat_completions(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<OpenAIRequest>,
) -> impl IntoResponse {
    // Auth
    let ident = match super::require_auth(&state, &headers).await {
        Ok(i) => i,
        Err(r) => return r,
    };

    // Convert OpenAI messages to provider messages.
    let messages: Vec<Message> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let mut msg = Message {
                role,
                content: m.content.clone(),
                content_parts: vec![],
                tool_calls: vec![],
                tool_call_id: m.tool_call_id.clone(),
                name: m.name.clone(),
                cache_control: None,
                raw: None,
                thinking: None,
                timestamp: None,
            };
            // Tool calls from OpenAI request.
            if !m.tool_calls.is_empty() {
                msg.tool_calls = m
                    .tool_calls
                    .iter()
                    .map(|tc| cleanclaw_provider::message::ToolCall {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(Value::Null),
                    })
                    .collect();
            }
            msg
        })
        .collect();

    // Pick a provider from the model string ("openai/gpt-4o-mini" → name "openai").
    let provider = match state.chat.provider_for(&req.model) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"message": "unknown model provider"}})),
            )
                .into_response();
        }
    };

    let tools: Vec<cleanclaw_provider::ToolDefinition> = req
        .tools
        .into_iter()
        .map(|t| cleanclaw_provider::ToolDefinition {
            name: t.function.name,
            description: t.function.description,
            parameters: t.function.parameters,
        })
        .collect();

    let chat_req = ProviderChatRequest {
        model: req
            .model
            .split_once('/')
            .map(|(_, m)| m.to_string())
            .unwrap_or_else(|| req.model.clone()),
        messages,
        tools,
        temperature: req.temperature,
        max_tokens: req.max_tokens.or(Some(1024)),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };

    if req.stream {
        // Simple SSE: stream content deltas then a final [DONE] marker.
        match provider.chat_stream(&chat_req).await {
            Ok(stream) => {
                let s = async_stream::stream! {
                    let mut s = std::pin::pin!(stream);
                    while let Some(ev) = s.next().await {
                        let ev = match ev {
                            Ok(e) => e,
                            Err(e) => {
                                yield Ok::<_, Infallible>(Event::default()
                                    .data(format!("{{\"error\": \"{}\"}}", e)));
                                break;
                            }
                        };
                        match ev {
                            StreamEvent::ContentDelta { delta } => {
                                let chunk = json!({
                                    "id": "chatcmpl-stream",
                                    "object": "chat.completion.chunk",
                                    "choices": [{
                                        "delta": {"content": delta},
                                        "index": 0,
                                    }]
                                });
                                yield Ok(Event::default().data(chunk.to_string()));
                            }
                            StreamEvent::Done { .. } => {
                                yield Ok(Event::default().data("[DONE]"));
                                break;
                            }
                            _ => {}
                        }
                    }
                };
                return Sse::new(s)
                    .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        }
    }

    // Non-streaming: just call chat() and shape the response like OpenAI.
    match provider.chat(&chat_req).await {
        Ok(resp) => {
            let body = json!({
                "id": resp.id,
                "object": "chat.completion",
                "model": resp.model,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": resp.message.content,
                    },
                    "finish_reason": resp.finish_reason,
                }],
                "usage": {
                    "prompt_tokens": resp.usage.input_tokens,
                    "completion_tokens": resp.usage.output_tokens,
                    "total_tokens": resp.usage.input_tokens + resp.usage.output_tokens,
                }
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"message": e.to_string()}})),
        )
            .into_response(),
    }
}
