//! In-process event hub for streaming agent turns to the web SSE
//! pipeline.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Content { delta: String },
    Thinking { delta: String },
    ToolCall { name: String, id: String, arguments: serde_json::Value },
    ToolResult { id: String, content: String, is_error: bool },
    Done { finish_reason: String, usage: Option<Usage> },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

pub struct EventHub {
    tx: broadcast::Sender<EventEnvelope>,
}

#[derive(Debug, Clone)]
pub struct EventEnvelope {
    pub agent_id: String,
    pub user_id: String,
    pub session_key: String,
    pub event: AgentEvent,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, env: EventEnvelope) {
        let _ = self.tx.send(env);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.tx.subscribe()
    }
}

pub type SharedEventHub = Arc<EventHub>;

pub fn new_shared(capacity: usize) -> SharedEventHub {
    Arc::new(EventHub::new(capacity))
}

// Suppress unused import for HashMap if not used.
#[allow(dead_code)]
fn _unused_hashmap_marker() -> HashMap<(), ()> {
    HashMap::new()
}
