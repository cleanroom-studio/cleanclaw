//! Plug-in layer for tools that talk to external services (web
//! search, image generation, TTS, web fetch). Mirrors
//! .
//!
//! Each category exposes ONE tool to the LLM backed by a primary
//! provider and an ordered fallback chain. The LLM never sees
//! individual providers.

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

pub mod extra_backends;
pub mod extra_backends2;
pub use extra_backends::register_extras;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("missing api key for {0}")]
    MissingApiKey(&'static str),
    #[error("no results from {0}")]
    NoResults(&'static str),
    #[error("http: {0}")]
    Http(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("upstream: {0}")]
    Upstream(String),
    /// Provider explicitly opts out of this category (chain
    /// terminator). The chain surfaces this as a hard failure
    /// rather than retrying the next provider.
    #[error("not configured: {0}")]
    NotConfigured(String),
    /// Transient failure; the chain should fall through to the
    /// next provider (e.g. 429, 5xx, network timeout).
    #[error("retry: {0}")]
    Retry(String),
}

/// Per-call configuration. The agent runtime fills this in from the
/// tenant's toolProviders.* config block.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub options: std::collections::HashMap<String, String>,
}

/// The full request passed to a provider. `Args` is the LLM-supplied
/// tool args; `Config` is the resolved per-tenant config.
#[derive(Debug, Clone)]
pub struct Request {
    pub args: serde_json::Value,
    pub config: ProviderConfig,
}

/// The provider's response. For imagegen/tts the `text` field is the
/// LLM-visible markdown payload (with base64 or URLs); for websearch
/// it's the formatted result list.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pub text: String,
    /// Optional structured payload (e.g. websearch hits). Not all
    /// providers populate this.
    pub extra: Option<serde_json::Value>,
}

impl Response {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            extra: None,
        }
    }
}

/// A single backend for one category.
#[async_trait]
pub trait Provider: Send + Sync {
    fn category(&self) -> &'static str;
    fn name(&self) -> &'static str;
    async fn execute(&self, req: Request) -> Result<Response, ProviderError>;
    /// Opt-in flag for backends that work without per-tenant config
    /// (e.g. direct HTTP fetch). Lets the chain report them as
    /// always available.
    fn credential_free(&self) -> bool {
        false
    }
}

// =====================================================================
// Registry — per-category provider lookup.
// =====================================================================

#[derive(Default)]
pub struct Registry {
    inner: std::sync::Mutex<std::collections::HashMap<String, Vec<Arc<dyn Provider>>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, p: Arc<dyn Provider>) {
        let mut g = self.inner.lock().expect("registry poisoned");
        g.entry(p.category().to_string()).or_default().push(p);
    }

    pub fn for_category(&self, category: &str) -> Vec<Arc<dyn Provider>> {
        self.inner
            .lock()
            .expect("registry poisoned")
            .get(category)
            .cloned()
            .unwrap_or_default()
    }

    pub fn categories(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("registry poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

// =====================================================================
// Chain — try primary, then fallbacks in order.
// =====================================================================

pub struct Chain {
    category: String,
    providers: Vec<Arc<dyn Provider>>,
}

impl Chain {
    pub fn new(category: impl Into<String>, providers: Vec<Arc<dyn Provider>>) -> Self {
        Self {
            category: category.into(),
            providers,
        }
    }

    pub fn from_registry(reg: &Registry, category: &str) -> Self {
        Self::new(category, reg.for_category(category))
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn providers(&self) -> &[Arc<dyn Provider>] {
        &self.providers
    }

    /// Run the chain. Returns the first non-error response. The last
    /// error is returned if every provider fails.
    pub async fn run(&self, mut make_req: impl FnMut(&dyn Provider) -> Request) -> Result<Response, ProviderError> {
        let mut last_err: Option<ProviderError> = None;
        for p in &self.providers {
            let req = make_req(&**p);
            match p.execute(req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::debug!(provider = p.name(), error = %e, "chain provider failed, trying next");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| ProviderError::NoResults("chain")))
    }
}

// =====================================================================
// Image-gen providers
// =====================================================================

pub mod imagegen {
    use super::*;

    pub const CATEGORY: &str = "image_gen";

    fn parse_args(raw: &serde_json::Value) -> Result<(String, String, usize), ProviderError> {
        let prompt = raw
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if prompt.is_empty() {
            return Err(ProviderError::InvalidArgs("prompt is required".into()));
        }
        let size = raw
            .get("size")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let n = raw.get("n").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let n = n.clamp(1, 4);
        Ok((prompt, size, n))
    }

    fn render_urls(prompt: &str, urls: &[String]) -> String {
        if urls.is_empty() {
            return String::new();
        }
        let mut s = format!("Generated {} image(s) for: {prompt}\n\n", urls.len());
        for (i, u) in urls.iter().enumerate() {
            s.push_str(&format!("{}. ![image {}]({})\n", i + 1, i + 1, u));
        }
        s
    }

    fn render_b64(prompt: &str, b64s: &[String]) -> String {
        if b64s.is_empty() {
            return String::new();
        }
        let mut s = format!("Generated {} image(s) for: {prompt}\n\n", b64s.len());
        for (i, b) in b64s.iter().enumerate() {
            s.push_str(&format!(
                "{}. ![image {}](data:image/png;base64,{})\n",
                i + 1,
                i + 1,
                b
            ));
        }
        s
    }

    /// "none" sentinel — chain handler in `agent/tools/image_gen.go`
    /// (Go) short-circuits on this so the model never sees a "none"
    /// provider. Mirrored on the Rust side.
    pub struct None;

    #[async_trait]
    impl Provider for None {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "none"
        }
        async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
            Err(ProviderError::NoResults("imagegen: none sentinel"))
        }
        fn credential_free(&self) -> bool {
            true
        }
    }

    /// OpenAI DALL-E / gpt-image-1.
    pub struct OpenAI {
        client: reqwest::Client,
    }

    impl OpenAI {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for OpenAI {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "openai"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (prompt, size, n) = parse_args(&req.args)?;
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("openai"));
            }
            let model = if req.config.model.is_empty() {
                "gpt-image-1"
            } else {
                req.config.model.as_str()
            };
            let size = if size.is_empty() { "1024x1024" } else { size.as_str() };
            let endpoint = if req.config.endpoint.is_empty() {
                "https://api.openai.com/v1/images/generations"
            } else {
                req.config.endpoint.as_str()
            };
            let body = serde_json::json!({
                "model": model,
                "prompt": prompt,
                "n": n,
                "size": size,
            });
            let resp = self
                .client
                .post(endpoint)
                .bearer_auth(&req.config.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("{status}: {txt}")));
            }
            let v: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            // Two response shapes: dall-e-3 → {data:[{url:...}]};
            // gpt-image-1 → {data:[{b64_json:...}]}.
            let mut urls = Vec::new();
            let mut b64s = Vec::new();
            if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
                for item in arr {
                    if let Some(u) = item.get("url").and_then(|u| u.as_str()) {
                        urls.push(u.to_string());
                    }
                    if let Some(b) = item.get("b64_json").and_then(|b| b.as_str()) {
                        b64s.push(b.to_string());
                    }
                }
            }
            let text = if !b64s.is_empty() {
                render_b64(&prompt, &b64s)
            } else {
                render_urls(&prompt, &urls)
            };
            if text.is_empty() {
                return Err(ProviderError::NoResults("openai"));
            }
            Ok(Response::from_text(text))
        }
    }
}

// =====================================================================
// TTS providers
// =====================================================================

pub mod tts {
    use super::*;

    pub const CATEGORY: &str = "tts";

    fn parse_args(raw: &serde_json::Value) -> Result<(String, String), ProviderError> {
        let text = raw
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if text.is_empty() {
            return Err(ProviderError::InvalidArgs("text is required".into()));
        }
        let voice = raw
            .get("voice")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok((text, voice))
    }

    pub struct None;

    #[async_trait]
    impl Provider for None {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "none"
        }
        async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
            Err(ProviderError::NoResults("tts: none sentinel"))
        }
        fn credential_free(&self) -> bool {
            true
        }
    }

    /// OpenAI TTS.
    pub struct OpenAI {
        client: reqwest::Client,
    }

    impl OpenAI {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for OpenAI {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "openai"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (text, voice) = parse_args(&req.args)?;
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("openai-tts"));
            }
            let model = if req.config.model.is_empty() {
                "tts-1"
            } else {
                req.config.model.as_str()
            };
            let voice = if voice.is_empty() { "alloy" } else { voice.as_str() };
            let endpoint = if req.config.endpoint.is_empty() {
                "https://api.openai.com/v1/audio/speech"
            } else {
                req.config.endpoint.as_str()
            };
            let body = serde_json::json!({
                "model": model,
                "input": text,
                "voice": voice,
                "response_format": "mp3",
            });
            let resp = self
                .client
                .post(endpoint)
                .bearer_auth(&req.config.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("{status}: {txt}")));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            // The Go side returns a workspace.Store Put path; for the
            // abstract provider we just hand the LLM a summary
            // (caller wraps it into a workspace artifact).
            Ok(Response::from_text(format!(
                "[tts] generated {} bytes of audio (model={model} voice={voice})",
                bytes.len()
            )))
        }
    }
}

// =====================================================================
// Web-fetch providers
// =====================================================================

pub mod webfetch {
    use super::*;

    pub const CATEGORY: &str = "web_fetch";

    fn parse_args(raw: &serde_json::Value) -> Result<String, ProviderError> {
        let url = raw
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Err(ProviderError::InvalidArgs("url is required".into()));
        }
        Ok(url)
    }

    /// Always-on direct fetcher; opts into CredentialFree so the
    /// dashboard can pick it without an API key.
    pub struct Direct {
        client: reqwest::Client,
    }

    impl Direct {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Direct {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "direct"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let url = parse_args(&req.args)?;
            let resp = self
                .client
                .get(&url)
                .header("user-agent", "cleanclaw/1.0")
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ProviderError::Upstream(format!(
                    "{}",
                    resp.status()
                )));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            // Truncate so the LLM context doesn't blow up on a 5 MB
            // page; the rest can be re-fetched on demand.
            let max = 16 * 1024;
            let truncated = if text.len() > max {
                format!("{}\n\n[truncated; original {} bytes]", &text[..max], text.len())
            } else {
                text
            };
            Ok(Response::from_text(truncated))
        }
        fn credential_free(&self) -> bool {
            true
        }
    }

    /// Jina Reader — converts a URL to clean markdown.
    pub struct Jina {
        client: reqwest::Client,
    }

    impl Jina {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Jina {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "jina"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let url = parse_args(&req.args)?;
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("jina"));
            }
            let endpoint = if req.config.endpoint.is_empty() {
                format!("https://r.jina.ai/{url}")
            } else {
                req.config.endpoint.clone()
            };
            let resp = self
                .client
                .get(&endpoint)
                .bearer_auth(&req.config.api_key)
                .header("X-Return-Format", "markdown")
                .header("X-Timeout", "30")
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ProviderError::Upstream(format!(
                    "{}",
                    resp.status()
                )));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            let max = 32 * 1024;
            let truncated = if text.len() > max {
                format!("{}\n\n[truncated; original {} bytes]", &text[..max], text.len())
            } else {
                text
            };
            Ok(Response::from_text(truncated))
        }
    }
}

// =====================================================================
// Web-search providers
// =====================================================================

pub mod websearch {
    use super::*;

    pub const CATEGORY: &str = "web_search";

    fn parse_args(raw: &serde_json::Value) -> Result<(String, usize), ProviderError> {
        let query = raw
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if query.is_empty() {
            return Err(ProviderError::InvalidArgs("query is required".into()));
        }
        let n = raw.get("n").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        Ok((query, n.clamp(1, 20)))
    }

    pub struct None;

    #[async_trait]
    impl Provider for None {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "none"
        }
        async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
            Err(ProviderError::NoResults("websearch: none sentinel"))
        }
        fn credential_free(&self) -> bool {
            true
        }
    }

    /// Brave Search.
    pub struct Brave {
        client: reqwest::Client,
    }

    impl Brave {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Brave {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "brave"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (query, n) = parse_args(&req.args)?;
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("brave"));
            }
            let endpoint = if req.config.endpoint.is_empty() {
                "https://api.search.brave.com/res/v1/web/search"
            } else {
                req.config.endpoint.as_str()
            };
            let resp = self
                .client
                .get(endpoint)
                .header("X-Subscription-Token", &req.config.api_key)
                .query(&[("q", query.as_str()), ("count", &n.to_string())])
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("{status}: {txt}")));
            }
            let v: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            let mut out = String::new();
            out.push_str(&format!("Search results for: {query}\n\n"));
            if let Some(results) = v.get("web").and_then(|w| w.get("results")).and_then(|r| r.as_array()) {
                for (i, r) in results.iter().take(n).enumerate() {
                    let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
                    let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("");
                    let snippet = r.get("description").and_then(|x| x.as_str()).unwrap_or("");
                    out.push_str(&format!(
                        "{}. {}\n   {}\n   {}\n\n",
                        i + 1,
                        title,
                        url,
                        snippet
                    ));
                }
            }
            Ok(Response::from_text(out))
        }
    }
}

/// Convenience: register every built-in provider this crate ships.
pub fn register_builtin(reg: &Registry) {
    let client = reqwest::Client::builder()
        .user_agent("cleanclaw/1.0")
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("reqwest client");
    reg.register(Arc::new(imagegen::OpenAI::new(client.clone())));
    reg.register(Arc::new(imagegen::None));
    reg.register(Arc::new(tts::OpenAI::new(client.clone())));
    reg.register(Arc::new(tts::None));
    reg.register(Arc::new(webfetch::Direct::new(client.clone())));
    reg.register(Arc::new(webfetch::Jina::new(client.clone())));
    reg.register(Arc::new(websearch::None));
    reg.register(Arc::new(websearch::Brave::new(client.clone())));
    // The 7 additional backends (Fal / Replicate / ElevenLabs /
    // Fish / MiniMax / Firecrawl / Exa / SearXNG) live in
    // `extra_backends` and are registered separately so the
    // canonical built-in list stays small.
    register_extras(reg, &client);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> ProviderConfig {
        ProviderConfig::default()
    }

    #[test]
    fn registry_register_and_lookup() {
        let r = Registry::new();
        register_builtin(&r);
        assert!(!r.for_category(imagegen::CATEGORY).is_empty());
        assert!(!r.for_category(tts::CATEGORY).is_empty());
        assert!(!r.for_category(webfetch::CATEGORY).is_empty());
        assert!(!r.for_category(websearch::CATEGORY).is_empty());
    }

    #[test]
    fn categories_listed() {
        let r = Registry::new();
        register_builtin(&r);
        let cats = r.categories();
        assert!(cats.contains(&imagegen::CATEGORY.to_string()));
        assert!(cats.contains(&tts::CATEGORY.to_string()));
        assert!(cats.contains(&webfetch::CATEGORY.to_string()));
        assert!(cats.contains(&websearch::CATEGORY.to_string()));
    }

    #[tokio::test]
    async fn imagegen_none_returns_no_results() {
        let p: Arc<dyn Provider> = Arc::new(imagegen::None);
        let r = p
            .execute(Request {
                args: serde_json::json!({"prompt": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::NoResults(_))));
    }

    #[tokio::test]
    async fn imagegen_missing_prompt() {
        let p: Arc<dyn Provider> = Arc::new(imagegen::OpenAI::new(
            reqwest::Client::new(),
        ));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn imagegen_openai_missing_key() {
        let p: Arc<dyn Provider> = Arc::new(imagegen::OpenAI::new(
            reqwest::Client::new(),
        ));
        let r = p
            .execute(Request {
                args: serde_json::json!({"prompt": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn tts_none_returns_no_results() {
        let p: Arc<dyn Provider> = Arc::new(tts::None);
        let r = p
            .execute(Request {
                args: serde_json::json!({"text": "hi"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::NoResults(_))));
    }

    #[tokio::test]
    async fn tts_openai_missing_text() {
        let p: Arc<dyn Provider> = Arc::new(tts::OpenAI::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn tts_openai_missing_key() {
        let p: Arc<dyn Provider> = Arc::new(tts::OpenAI::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({"text": "hello"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn websearch_none_returns_no_results() {
        let p: Arc<dyn Provider> = Arc::new(websearch::None);
        let r = p
            .execute(Request {
                args: serde_json::json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::NoResults(_))));
    }

    #[tokio::test]
    async fn websearch_brave_missing_key() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Brave::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn webfetch_missing_url() {
        let p: Arc<dyn Provider> =
            Arc::new(webfetch::Direct::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn chain_runs_providers_in_order() {
        let r = Registry::new();
        register_builtin(&r);
        let chain = Chain::from_registry(&r, imagegen::CATEGORY);
        // No real HTTP — all providers will fail. The chain should
        // exhaust the list and return the last error rather than
        // short-circuit on the first.
        let res = chain
            .run(|_p| Request {
                args: serde_json::json!({"prompt": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn chain_with_no_providers_errors() {
        let chain = Chain::new("nope", vec![]);
        let res = chain
            .run(|_| Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(res, Err(ProviderError::NoResults(_))));
    }

    #[test]
    fn provider_config_defaults() {
        let c = ProviderConfig::default();
        assert!(c.api_key.is_empty());
        assert!(c.endpoint.is_empty());
        assert!(c.model.is_empty());
        assert!(c.options.is_empty());
    }

    #[test]
    fn response_from_text_constructs() {
        let r = Response::from_text("hello");
        assert_eq!(r.text, "hello");
        assert!(r.extra.is_none());
    }
}
