//! `memory_search` tool — search through the agent's memory.
//!
//! For
//! the first cut this is a simple text grep over `MEMORY.md` + the
//! session archive. FTS5 is a future optimization (would require a
//! dedicated FTS table in the store).

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct MemorySearchTool {
    pub store: Arc<dyn Store>,
    pub agent_id: String,
    pub user_id: String,
    pub workspace: String,
}

#[derive(Deserialize)]
struct Args {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }
    fn description(&self) -> &str {
        "Search through conversation history logs using keyword matching with recency weighting."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Keywords to search for"},
                "limit": {"type": "integer", "description": "Maximum number of results (default 10)"}
            },
            "required": ["query"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: Args = serde_json::from_value(args)?;
        if a.query.is_empty() {
            return Err(CleanClawError::InvalidArgument("query is required".into()));
        }
        let limit = a.limit.unwrap_or(10);
        let results = search_memory(&self.workspace, &self.user_id, &self.agent_id, &self.store, &a.query, limit).await?;
        if results.is_empty() {
            return Ok(json!({"results": [], "query": a.query}));
        }
        Ok(json!({"results": results, "query": a.query}))
    }
}

async fn search_memory(
    workspace: &str,
    user_id: &str,
    agent_id: &str,
    store: &Arc<dyn Store>,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    // Strategy 1: grep MEMORY.md for query terms.
    let mut results: Vec<Value> = Vec::new();
    let memory_path = std::path::Path::new(workspace).join("MEMORY.md");
    if let Ok(content) = std::fs::read_to_string(&memory_path) {
        for (idx, line) in content.lines().enumerate() {
            if line_contains(line, query) {
                results.push(json!({
                    "file": "MEMORY.md",
                    "line": idx + 1,
                    "content": line,
                    "source": "memory",
                }));
                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }
    }

    // Strategy 2: fall back to the session archive. We list the
    // chatter's sessions for this agent and grep the messages.
    if let Ok(sessions) = store.list_sessions(user_id, agent_id).await {
        for sess in sessions.iter().take(50) {
            if let Ok(msgs) = store.list_session_messages(user_id, agent_id, &sess.key).await {
                for m in msgs {
                    if line_contains(&m.content, query) && !m.content.is_empty() {
                        results.push(json!({
                            "file": format!("sessions/{}/{}.jsonl", sess.key, m.seq),
                            "line": m.seq,
                            "content": truncate(&m.content, 400),
                            "source": "session",
                        }));
                        if results.len() >= limit {
                            return Ok(results);
                        }
                    }
                }
            }
        }
    }
    Ok(results)
}

fn line_contains(line: &str, query: &str) -> bool {
    let line_l = line.to_ascii_lowercase();
    for term in query.split_whitespace() {
        if line_l.contains(&term.to_ascii_lowercase()) {
            return true;
        }
    }
    false
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut cut = n;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}
