//! `web_fetch` and `web_search` tools.
//!
//! For the first cut these are minimal placeholders that surface the URL
//! content (web_fetch) or return a stub response (web_search). The full
//! tool provider / chain implementation lives in `cleanclaw-toolprov`
//! (future phase) — when the providers are registered, these tools will
//! dispatch to them.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct WebFetchTool;

#[derive(Deserialize)]
struct FetchArgs {
    url: String,
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "Fetch a URL and return the first ~16 KiB of text. Use for reading documentation, blog posts, etc."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
                "max_bytes": {"type": "integer", "default": 16384}
            },
            "required": ["url"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: FetchArgs = serde_json::from_value(args)?;
        let max = a.max_bytes.unwrap_or(16 * 1024);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("CleanClaw/0.1")
            .build()
            .map_err(|e| CleanClawError::Internal(format!("web_fetch: {e}")))?;
        let resp = client
            .get(&a.url)
            .send()
            .await
            .map_err(|e| CleanClawError::Upstream(format!("web_fetch {}: {e}", a.url)))?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp
            .text()
            .await
            .map_err(|e| CleanClawError::Upstream(format!("web_fetch body: {e}")))?;
        let truncated = body.chars().take(max).collect::<String>();
        Ok(json!({
            "status": status,
            "content_type": content_type,
            "body": truncated,
            "truncated": body.len() > max,
        }))
    }
}

pub struct WebSearchTool;

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web. Returns an ordered list of {title, url, snippet}. Backed by the agent's configured search provider chain."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "default": 10}
            },
            "required": ["query"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: SearchArgs = serde_json::from_value(args)?;
        // Web search is plumbed through the tool-provider chain; for
        // the first cut we surface a clear "not configured" message
        // instead of a silent failure so the model knows what to do.
        Err(CleanClawError::NotImplemented(
            "web_search: no provider configured. Add one via the dashboard's Tools page or `cleanclaw provider add`.".into()
        ))
    }
}
