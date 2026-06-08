//! Shared message types — provider-agnostic shapes that all adapters map
//! to/from. Mirrors the union of
//! and `anthropic.go`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema object
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { url: String },
    ImageBase64 { media_type: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_parts: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
    /// Raw provider-specific payload (e.g. Anthropic "raw_assistant"
    /// blocks for prompt cache replay). Preserved verbatim for
    /// cache-hit optimization on subsequent turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
    /// Thinking / reasoning content. Preserved for memory extraction
    /// and for tools that surface it to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Optional unix-ms timestamp. Set by the session manager on
    /// append; consumed by the dashboard's per-turn timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: s.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: s.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: s.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        }
    }

    /// Construct an assistant message carrying both visible content
    /// and a separate `thinking` channel (used by DeepSeek
    /// `reasoning_content` and Anthropic `thinking` blocks). The
    /// runtime assembles both into the system prompt snippet.
    pub fn assistant_with_thinking(
        content: impl Into<String>,
        thinking: impl Into<String>,
    ) -> Self {
        let thinking: String = thinking.into();
        Self {
            role: Role::Assistant,
            content: content.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            },
            timestamp: None,
        }
    }
    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: Some(id.into()),
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheControl {
    Ephemeral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    /// Whether the provider should stream deltas back.
    #[serde(default)]
    pub stream: bool,
    /// Provider-specific extras (e.g. Anthropic `thinking`).
    #[serde(default)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub model: String,
    pub message: Message,
    pub finish_reason: String,
    pub usage: Usage,
    /// Provider-specific blob the runtime can save onto
    /// session_messages.raw_assistant for cache-hit replay.
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// A chunk of assistant text arrived.
    ContentDelta { delta: String },
    /// A tool call started or got argument deltas.
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    },
    /// Thinking / reasoning delta.
    ThinkingDelta { delta: String },
    /// The provider reports the turn is done. Optional `usage` may
    /// accompany the final event.
    Done {
        finish_reason: String,
        usage: Option<Usage>,
    },
    /// Provider surfaced an error mid-stream.
    Error { message: String },
}
