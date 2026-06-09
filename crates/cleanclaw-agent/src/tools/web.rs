//! `web_fetch` and `web_search` tools.
//!
//! `web_fetch` is a direct reqwest call (the model needs the
//! ability to grab any URL on demand, not just configured
//! providers). `web_search` dispatches into the toolprov chain
//! — operators pick the upstream (DuckDuckGo, Brave, Bing,
//! Google, Baidu) via the Tools page; the chain runs primary
//! first then fallbacks on credential / network errors.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_toolprov::{websearch, ProviderConfig, Registry as ToolprovRegistry};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

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

/// `web_search` — dispatches to the toolprov chain. The
/// `Arc<Registry>` is the per-process toolprov registry built
/// at gateway boot (see `ChatService::new_with_toolprov`).
///
/// Per-call credentials come through `ToolContext.extra` under
/// the key `"web_search_configs"`, populated by the chat
/// service before each turn. The shape matches `HashMap<String,
/// ProviderConfig>` keyed by provider name.
pub struct WebSearchTool {
    pub registry: Arc<ToolprovRegistry>,
}

impl WebSearchTool {
    pub fn new(registry: Arc<ToolprovRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Deserialize)]
struct SearchArgs {
    #[allow(dead_code)]
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

const CONFIGS_KEY: &str = "web_search_configs";

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }
    fn description(&self) -> &'static str {
        "Search the web. Returns an ordered list of {title, url, snippet}. Backed by the operator-configured search chain (DuckDuckGo / Brave / Bing / Google / Baidu). Works without an API key via the default DuckDuckGo primary."
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
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let mut args = args;
        let a: SearchArgs = serde_json::from_value(args.clone())?;
        // Mirror the LLM's `limit` into the provider chain's
        // `n` argument so providers can clamp their own result
        // window.
        if let Some(n) = a.limit {
            args.as_object_mut()
                .map(|o| o.insert("n".to_string(), json!(n)));
        }
        let chain = cleanclaw_toolprov::Chain::from_registry(&self.registry, websearch::CATEGORY);
        if chain.is_empty() {
            return Err(CleanClawError::NotImplemented(
                "web_search: no provider registered in the toolprov chain".into(),
            ));
        }
        // Pull the per-provider credentials out of ToolContext.extra.
        // The chat service stashes them there before each turn
        // (after reading the system tools config row).
        let configs: std::collections::HashMap<String, ProviderConfig> = ctx
            .extra
            .get(CONFIGS_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let make_req = |p: &dyn cleanclaw_toolprov::Provider| {
            let cfg = configs.get(p.name()).cloned().unwrap_or_default();
            cleanclaw_toolprov::Request {
                args: args.clone(),
                config: cfg,
            }
        };
        match chain.run(make_req).await {
            Ok(r) => Ok(json!({ "results": r.text })),
            Err(e) => Err(CleanClawError::Upstream(format!("web_search: {e}"))),
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_toolprov::{Provider, ProviderError, Request, Response};

    /// A mock provider that returns hard-coded search results
    /// without any HTTP call.
    struct MockSearch;

    #[async_trait]
    impl Provider for MockSearch {
        fn category(&self) -> &'static str {
            websearch::CATEGORY
        }
        fn name(&self) -> &'static str {
            "mock"
        }
        fn credential_free(&self) -> bool {
            true
        }
        async fn execute(&self, req: Request) -> std::result::Result<Response, ProviderError> {
            let query = req.args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            if query.is_empty() {
                return Err(ProviderError::InvalidArgs("query required".into()));
            }
            Ok(Response::from_text(format!(
                "Search results for: {query}\n\n1. Mock Result\n   https://example.com\n   A fake result for testing.\n"
            )))
        }
    }

    #[tokio::test]
    async fn web_search_tool_empty_registry_errors() {
        let registry = Arc::new(ToolprovRegistry::new());
        let tool = WebSearchTool::new(registry);
        let ctx = ToolContext::default();
        let r = tool.call(&ctx, json!({"query": "test"})).await;
        assert!(r.is_err());
        let err = r.unwrap_err().to_string();
        assert!(err.contains("no provider registered"));
    }

    #[tokio::test]
    async fn web_search_tool_with_mock_provider_succeeds() {
        let registry = Arc::new(ToolprovRegistry::new());
        registry.register(Arc::new(MockSearch));
        let tool = WebSearchTool::new(registry);
        let ctx = ToolContext::default();
        let r = tool
            .call(&ctx, json!({"query": "rust programming"}))
            .await
            .unwrap();
        let results = r.get("results").and_then(|v| v.as_str()).unwrap();
        assert!(results.contains("Mock Result"));
        assert!(results.contains("rust programming"));
    }

    #[tokio::test]
    async fn web_search_tool_missing_query_errors() {
        let registry = Arc::new(ToolprovRegistry::new());
        registry.register(Arc::new(MockSearch));
        let tool = WebSearchTool::new(registry);
        let ctx = ToolContext::default();
        let r = tool.call(&ctx, json!({})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn web_search_tool_passes_configs_from_context() {
        use std::collections::HashMap;
        let registry = Arc::new(ToolprovRegistry::new());
        registry.register(Arc::new(MockSearch));
        let tool = WebSearchTool::new(registry);
        let mut extra = HashMap::new();
        extra.insert(
            CONFIGS_KEY.to_string(),
            json!({"mock": {"api_key": "test-key", "endpoint": ""}}),
        );
        let ctx = ToolContext {
            extra: Arc::new(extra),
            ..Default::default()
        };
        let r = tool.call(&ctx, json!({"query": "test"})).await.unwrap();
        let results = r.get("results").and_then(|v| v.as_str()).unwrap();
        assert!(results.contains("test"));
    }

    #[tokio::test]
    async fn web_search_tool_name_and_params_match() {
        let registry = Arc::new(ToolprovRegistry::new());
        let tool = WebSearchTool::new(registry);
        assert_eq!(tool.name(), "web_search");
        let params = tool.parameters();
        let props = params.get("properties").unwrap();
        assert!(props.get("query").is_some());
        assert!(props.get("limit").is_some());
        let req = params.get("required").unwrap().as_array().unwrap();
        assert!(req.contains(&json!("query")));
    }

    #[test]
    fn web_search_tool_description_not_empty() {
        let registry = Arc::new(ToolprovRegistry::new());
        let tool = WebSearchTool::new(registry);
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("DuckDuckGo"));
    }
}
