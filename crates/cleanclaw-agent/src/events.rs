//! Event types surfaced by the agent runtime.
//!
//! (the non-streaming
//! part — streaming deltas are in `event_hub.rs` / `cleanclaw_provider`).

use serde::{Deserialize, Serialize};

/// `AgentEventType` is the persisted / event-bus shape. The streaming
/// `StreamEvent` is in `cleanclaw_provider::StreamEvent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEventType {
    /// A new turn started.
    TurnStart {
        agent_id: String,
        user_id: String,
        session_key: String,
    },
    /// A complete turn finished.
    TurnEnd {
        agent_id: String,
        user_id: String,
        session_key: String,
        finish_reason: String,
        iterations: u32,
        usage: super::event_hub::Usage,
    },
    /// A tool call dispatched.
    ToolCall {
        agent_id: String,
        session_key: String,
        tool: String,
        arguments: serde_json::Value,
    },
    /// A tool call returned.
    ToolResult {
        agent_id: String,
        session_key: String,
        tool: String,
        is_error: bool,
        duration_ms: u64,
    },
    /// An error escaped the turn (the gateway forwards to the
    /// channel / chat surface).
    Error {
        agent_id: String,
        user_id: String,
        message: String,
    },
}

impl AgentEventType {
    pub fn agent_id(&self) -> &str {
        match self {
            AgentEventType::TurnStart { agent_id, .. }
            | AgentEventType::TurnEnd { agent_id, .. }
            | AgentEventType::ToolCall { agent_id, .. }
            | AgentEventType::ToolResult { agent_id, .. }
            | AgentEventType::Error { agent_id, .. } => agent_id,
        }
    }
}
