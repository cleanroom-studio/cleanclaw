//! Anthropic Messages API client.
//!
//! Maps provider-agnostic `ChatRequest` / `Message` types to the
//! Anthropic-specific shape (system as a top-level string, content
//! blocks, prompt cache via `cache_control: { type: "ephemeral" }`,
//! thinking via the `thinking` field).

use super::message::*;
use super::provider::*;
use async_trait::async_trait;
use futures_util::Stream;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub api_base: String, // "https://api.anthropic.com"
    pub version: String,  // "2023-06-01"
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            api_base: "https://api.anthropic.com".into(),
            version: "2023-06-01".into(),
        }
    }
}

pub struct AnthropicProvider {
    cfg: AnthropicConfig,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(cfg: AnthropicConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        Self { cfg, client }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let url = format!("{}/v1/messages", self.cfg.api_base.trim_end_matches('/'));
        let body = build_anthropic_body(req, false);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", &self.cfg.version)
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
        parse_anthropic_response(&raw)
    }

    async fn chat_stream(&self, req: &ChatRequest) -> Result<ProviderStream, ProviderError> {
        let url = format!("{}/v1/messages", self.cfg.api_base.trim_end_matches('/'));
        let body = build_anthropic_body(req, true);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", &self.cfg.version)
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
        let stream = anthropic_stream(byte_stream);
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

fn build_anthropic_body(req: &ChatRequest, stream: bool) -> Value {
    let mut system_blocks: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();

    for m in &req.messages {
        match m.role {
            Role::System => {
                system_blocks.push(json!({
                    "type": "text",
                    "text": m.content,
                    "cache_control": cache_control_value(m.cache_control),
                }));
            }
            Role::User => {
                let content =
                    if m.content_parts.is_empty() {
                        json!(m.content)
                    } else {
                        json!(m.content_parts.iter().map(|p| match p {
                        ContentPart::Text { text } => json!({"type": "text", "text": text}),
                        ContentPart::ImageUrl { url } => {
                            json!({"type": "image", "source": {"type": "url", "url": url}})
                        }
                        ContentPart::ImageBase64 { media_type, data } => json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": media_type, "data": data}
                        }),
                    }).collect::<Vec<_>>())
                    };
                messages.push(json!({"role": "user", "content": content}));
            }
            Role::Assistant => {
                let mut blocks: Vec<Value> = vec![];
                if !m.content.is_empty() {
                    blocks.push(json!({"type": "text", "text": m.content}));
                }
                for tc in &m.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": ""}));
                }
                messages.push(json!({"role": "assistant", "content": blocks}));
            }
            Role::Tool => {
                let id = m.tool_call_id.clone().unwrap_or_default();
                let content = if m.content.is_empty() {
                    json!("")
                } else {
                    json!(m.content)
                };
                messages.push(json!({
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": id, "content": content}]
                }));
            }
        }
    }

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect();

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens.unwrap_or(4096),
        "stream": stream,
    });
    if !system_blocks.is_empty() {
        body["system"] = json!(system_blocks);
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = req.top_p {
        body["top_p"] = json!(p);
    }
    if !req.stop.is_empty() {
        body["stop_sequences"] = json!(req.stop);
    }
    for (k, v) in &req.extra {
        body[k] = v.clone();
    }
    body
}

fn cache_control_value(cc: Option<CacheControl>) -> Value {
    match cc {
        Some(CacheControl::Ephemeral) => json!({"type": "ephemeral"}),
        None => Value::Null,
    }
}

fn parse_anthropic_response(raw: &Value) -> Result<ChatResponse, ProviderError> {
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

    let content = raw
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in &content {
        let bt = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match bt {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    text.push_str(t);
                }
            }
            "thinking" => {
                if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                    thinking.push_str(t);
                }
            }
            "tool_use" => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments: input,
                });
            }
            _ => {}
        }
    }

    let finish_reason = raw
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("end_turn")
        .to_string();

    let usage = raw.get("usage").cloned().unwrap_or(json!({}));
    let input_tokens = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let output_tokens = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cache_read_tokens = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cache_creation_tokens = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut message = Message::assistant(text);
    message.tool_calls = tool_calls;
    if !thinking.is_empty() {
        message.thinking = Some(thinking);
    }
    message.raw = Some(raw.clone());

    Ok(ChatResponse {
        id,
        model,
        message,
        finish_reason,
        usage: Usage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        },
        raw: raw.clone(),
    })
}

fn anthropic_stream(
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
                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                    let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match event_type {
                        "content_block_delta" => {
                            if let Some(delta) = v.get("delta") {
                                let dt = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                match dt {
                                    "text_delta" => {
                                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                            yield StreamEvent::ContentDelta { delta: text.to_string() };
                                        }
                                    }
                                    "thinking_delta" => {
                                        if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                                            yield StreamEvent::ThinkingDelta { delta: text.to_string() };
                                        }
                                    }
                                    "input_json_delta" => {
                                        let partial = delta.get("partial_json").and_then(|t| t.as_str()).map(|s| s.to_string());
                                        if let Some(p) = partial {
                                            yield StreamEvent::ToolCallDelta {
                                                index: 0,
                                                id: None,
                                                name: None,
                                                arguments_delta: Some(p),
                                            };
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "content_block_start" => {
                            if let Some(block) = v.get("content_block") {
                                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                    let id = block.get("id").and_then(|s| s.as_str()).map(|s| s.to_string());
                                    let name = block.get("name").and_then(|s| s.as_str()).map(|s| s.to_string());
                                    yield StreamEvent::ToolCallDelta {
                                        index: 0,
                                        id,
                                        name,
                                        arguments_delta: None,
                                    };
                                }
                            }
                        }
                        "message_delta" => {
                            if let Some(stop) = v.get("delta").and_then(|d| d.get("stop_reason")).and_then(|s| s.as_str()) {
                                let usage = v.get("usage").and_then(|u| {
                                    serde_json::from_value::<Value>(u.clone()).ok()
                                }).map(|u| Usage {
                                    input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                    output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                    cache_read_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                    cache_creation_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                });
                                yield StreamEvent::Done {
                                    finish_reason: stop.to_string(),
                                    usage,
                                };
                                return;
                            }
                        }
                        "message_stop" => {
                            yield StreamEvent::Done {
                                finish_reason: "end_turn".into(),
                                usage: None,
                            };
                            return;
                        }
                        "error" => {
                            let msg = v.get("error").and_then(|e| e.get("message")).and_then(|s| s.as_str()).unwrap_or("error").to_string();
                            yield StreamEvent::Error { message: msg };
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
