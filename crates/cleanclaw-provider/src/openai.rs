//! OpenAI / OpenAI-compatible chat completions.
//!
//! Implements the `Provider` trait against the `/v1/chat/completions`
//! endpoint. Any OpenAI-compatible base URL (OpenRouter, Together,
//! vLLM, …) is supported by setting `api_base`.

use super::message::*;
use super::provider::*;
use async_trait::async_trait;
use futures_util::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub api_base: String, // e.g. "https://api.openai.com/v1"
}

pub struct OpenAIProvider {
    cfg: OpenAIConfig,
    client: reqwest::Client,
}

impl OpenAIProvider {
    /// Build a provider, normalizing the API base per
    /// `url::normalize_api_base` so user-typed values like
    /// `https://api.openai.com` (no `/v1`) and `https://api.openai.com/v1/`
    /// (trailing slash) both resolve to the same final URL.
    pub fn new(mut cfg: OpenAIConfig) -> Self {
        cfg.api_base = crate::url::normalize_api_base(&cfg.api_base, "openai-chat");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        Self { cfg, client }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let url = format!(
            "{}/chat/completions",
            self.cfg.api_base.trim_end_matches('/')
        );
        let body = build_openai_body(req, false);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_http(status.as_u16(), &text));
        }
        let raw: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        parse_openai_response(&raw)
    }

    async fn chat_stream(&self, req: &ChatRequest) -> Result<ProviderStream, ProviderError> {
        let url = format!(
            "{}/chat/completions",
            self.cfg.api_base.trim_end_matches('/')
        );
        let body = build_openai_body(req, true);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_http(status.as_u16(), &text));
        }
        let byte_stream = resp.bytes_stream();
        let stream = openai_stream(byte_stream);
        Ok(Box::pin(stream))
    }
}

fn classify_http(code: u16, text: &str) -> ProviderError {
    match code {
        401 | 403 => ProviderError::Auth(text.to_string()),
        429 => ProviderError::RateLimited,
        400..=499 => ProviderError::Upstream(text.to_string()),
        _ => ProviderError::Upstream(text.to_string()),
    }
}

fn build_openai_body(req: &ChatRequest, stream: bool) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            let mut o = json!({"role": role_str(&m.role), "content": text_or_parts(m)});
            if !m.tool_calls.is_empty() {
                o["tool_calls"] = json!(
                    m.tool_calls
                        .iter()
                        .map(|tc| json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string()),
                            }
                        }))
                        .collect::<Vec<_>>()
                );
            }
            if let Some(tcid) = &m.tool_call_id {
                o["tool_call_id"] = json!(tcid);
            }
            if let Some(name) = &m.name {
                o["name"] = json!(name);
            }
            // DeepSeek's "thinking mode" requires the assistant
            // message to echo the previous turn's `reasoning_content`
            // back to the API. Pure OpenAI ignores unknown fields,
            // so `omitempty` keeps non-DeepSeek providers unaffected.
            if let Some(reasoning) = &m.thinking {
                o["reasoning_content"] = json!(reasoning);
            }
            o
        })
        .collect();

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect();

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": stream,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(m) = req.max_tokens {
        body["max_tokens"] = json!(m);
    }
    if let Some(p) = req.top_p {
        body["top_p"] = json!(p);
    }
    if !req.stop.is_empty() {
        body["stop"] = json!(req.stop);
    }
    // `stream_options.include_usage = true` tells OpenAI-compat APIs
    // (OpenAI, DeepSeek, OpenRouter, Together, vLLM, …) to emit one
    // final SSE chunk with the total token counts before [DONE].
    // Without this flag the streaming path returns no usage, which
    // breaks per-turn goal-token accounting and admin metering.
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    for (k, v) in &req.extra {
        body[k] = v.clone();
    }
    body
}

fn role_str(r: &Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn text_or_parts(m: &Message) -> Value {
    if m.content_parts.is_empty() {
        return json!(m.content);
    }
    let parts: Vec<Value> = m
        .content_parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => json!({"type": "text", "text": text}),
            ContentPart::ImageUrl { url } => {
                json!({"type": "image_url", "image_url": {"url": url}})
            }
            ContentPart::ImageBase64 { media_type, data } => json!({
                "type": "image_url",
                "image_url": {"url": format!("data:{};base64,{}", media_type, data)}
            }),
        })
        .collect();
    json!(parts)
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: RespMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RespMessage {
    role: Option<String>,
    content: Option<String>,
    /// DeepSeek's `reasoning_content` — separate from `content`,
    /// surfaces the chain-of-thought for the turn.
    #[serde(default)]
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<RespToolCall>>,
}

#[derive(Debug, Deserialize)]
struct RespToolCall {
    id: Option<String>,
    function: RespFunction,
}

#[derive(Debug, Deserialize)]
struct RespFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RespUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    #[serde(default)]
    prompt_tokens_details: Option<RespPromptDetails>,
}

#[derive(Debug, Deserialize, Default)]
struct RespPromptDetails {
    cached_tokens: Option<u32>,
}

fn parse_openai_response(raw: &Value) -> Result<ChatResponse, ProviderError> {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model = raw
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let choice = raw
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| ProviderError::Decode("no choices".into()))?;
    let choice: Choice =
        serde_json::from_value(choice.clone()).map_err(|e| ProviderError::Decode(e.to_string()))?;

    let tool_calls: Vec<ToolCall> = choice
        .message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id.unwrap_or_default(),
            name: tc.function.name.unwrap_or_default(),
            arguments: tc
                .function
                .arguments
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null),
        })
        .collect();

    let role = match choice.message.role.as_deref() {
        Some("assistant") => Role::Assistant,
        Some("system") => Role::System,
        Some("tool") => Role::Tool,
        _ => Role::Assistant,
    };
    let content = choice.message.content.clone().unwrap_or_default();
    let message = Message {
        role,
        content,
        name: None,
        tool_call_id: None,
        tool_calls,
        content_parts: vec![],
        cache_control: None,
        raw: None,
        // Surface DeepSeek's reasoning in the same slot the
        // Anthropic thinking-mode uses — the runtime treats them
        // uniformly downstream.
        thinking: choice
            .message
            .reasoning_content
            .clone()
            .filter(|s| !s.is_empty()),
        timestamp: None,
    };

    let usage = raw
        .get("usage")
        .and_then(|u| serde_json::from_value::<RespUsage>(u.clone()).ok())
        .map(|u| Usage {
            input_tokens: u.prompt_tokens.unwrap_or(0),
            output_tokens: u.completion_tokens.unwrap_or(0),
            cache_read_tokens: u
                .prompt_tokens_details
                .and_then(|d| d.cached_tokens)
                .unwrap_or(0),
            cache_creation_tokens: 0,
        })
        .unwrap_or(Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        });

    Ok(ChatResponse {
        id,
        model,
        message,
        finish_reason: choice.finish_reason.unwrap_or_else(|| "stop".into()),
        usage,
        raw: raw.clone(),
    })
}

// --- streaming --------------------------------------------------------------

fn openai_stream(
    bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent, ProviderError>> + Send {
    use futures_util::stream::StreamExt;
    async_stream::try_stream! {
        let mut buf = String::new();
        let mut stream = std::pin::pin!(bytes);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ProviderError::Http(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(idx) = buf.find('\n') {
                let line: String = buf.drain(..=idx).collect();
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let payload = line.strip_prefix("data:").unwrap_or(line).trim();
                if payload == "[DONE]" {
                    yield StreamEvent::Done {
                        finish_reason: "stop".into(),
                        usage: None,
                    };
                    return;
                }
                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                    if let Some(choice) = v.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first()) {
                        if let Some(delta) = choice.get("delta") {
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                if !content.is_empty() {
                                    yield StreamEvent::ContentDelta { delta: content.to_string() };
                                }
                            }
                            // DeepSeek's `reasoning_content` field
                            // surfaces the chain-of-thought separately
                            // from the final answer. Emit it as a
                            // ThinkingDelta so the runtime can choose
                            // to render it, store it, or strip it
                            // before display.
                            if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                                if !reasoning.is_empty() {
                                    yield StreamEvent::ThinkingDelta { delta: reasoning.to_string() };
                                }
                            }
                            if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                for (i, tc) in tcs.iter().enumerate() {
                                    let id = tc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                                    let name = tc
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let args_delta = tc
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    yield StreamEvent::ToolCallDelta {
                                        index: i,
                                        id,
                                        name,
                                        arguments_delta: args_delta,
                                    };
                                }
                            }
                        }
                        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                            if !reason.is_empty() {
                                let usage = v.get("usage").and_then(|u| {
                                    serde_json::from_value::<RespUsage>(u.clone()).ok()
                                }).map(|u| Usage {
                                    input_tokens: u.prompt_tokens.unwrap_or(0),
                                    output_tokens: u.completion_tokens.unwrap_or(0),
                                    cache_read_tokens: u
                                        .prompt_tokens_details
                                        .and_then(|d| d.cached_tokens)
                                        .unwrap_or(0),
                                    cache_creation_tokens: 0,
                                });
                                yield StreamEvent::Done {
                                    finish_reason: reason.to_string(),
                                    usage,
                                };
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, Role};

    fn sample_req() -> ChatRequest {
        ChatRequest {
            model: "deepseek-chat".into(),
            messages: vec![Message::user("hello")],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            stop: vec![],
            stream: false,
            extra: Default::default(),
        }
    }

    #[test]
    fn build_body_echoes_reasoning_content() {
        // DeepSeek-style: an assistant turn carries the previous
        // turn's reasoning in `thinking`; the body must echo it as
        // `reasoning_content` so the next turn doesn't break.
        let mut req = sample_req();
        req.messages.push(Message {
            role: Role::Assistant,
            content: "answer".into(),
            thinking: Some("because".into()),
            ..Message::assistant("answer")
        });
        let body = build_openai_body(&req, false);
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["reasoning_content"], "because");
    }

    #[test]
    fn build_body_omits_reasoning_when_absent() {
        let req = sample_req();
        let body = build_openai_body(&req, false);
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert!(last.get("reasoning_content").is_none());
    }

    #[test]
    fn build_body_stream_adds_include_usage() {
        let req = sample_req();
        let body_stream = build_openai_body(&req, true);
        assert_eq!(body_stream["stream_options"]["include_usage"], true);
        let body_nostream = build_openai_body(&req, false);
        assert!(body_nostream.get("stream_options").is_none());
    }

    #[test]
    fn parse_response_surfaces_reasoning_content() {
        let raw = serde_json::json!({
            "id": "x",
            "model": "deepseek",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "42",
                    "reasoning_content": "I think therefore I am."
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        let resp = parse_openai_response(&raw).unwrap();
        assert_eq!(resp.message.content, "42");
        assert_eq!(
            resp.message.thinking.as_deref(),
            Some("I think therefore I am.")
        );
    }

    #[test]
    fn parse_response_no_reasoning_leaves_thinking_empty() {
        let raw = serde_json::json!({
            "id": "x",
            "model": "gpt",
            "choices": [{
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        let resp = parse_openai_response(&raw).unwrap();
        assert!(resp.message.thinking.is_none());
    }
}
