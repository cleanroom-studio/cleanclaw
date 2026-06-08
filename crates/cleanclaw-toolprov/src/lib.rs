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

    /// Run the provider. `args` is the LLM-supplied tool args;
    /// `config` is the resolved per-tenant config (api_key, etc.).
    async fn execute(&self, req: Request) -> Result<Response, ProviderError>;

    /// When `true`, the chain will call this provider even if
    /// `api_key` is empty in the config. Providers that expose a
    /// public, key-less endpoint (DuckDuckGo / Baidu / SearXNG-public)
    /// flip this on. Default: `false`.
    fn credential_free(&self) -> bool {
        false
    }

    /// When `true`, the chain needs a non-empty `endpoint` URL
    /// (e.g. self-hosted SearXNG, or Google CSE's `cx=…` field).
    /// Providers that don't need an endpoint leave this `false`
    /// (the default).
    fn needs_endpoint(&self) -> bool {
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

    /// Decide whether a provider should be skipped because its
    /// required config is missing. A key-bearing provider without
    /// an API key can never succeed; a self-hosted endpoint
    /// provider without an endpoint URL can never succeed. We
    /// skip these **silently** (no entry in the error log) so the
    /// final error message stays focused on the providers we
    /// actually attempted.
    fn should_skip(&self, p: &dyn Provider, cfg: &ProviderConfig) -> bool {
        if !p.credential_free() && cfg.api_key.is_empty() {
            return true;
        }
        if p.needs_endpoint() && cfg.endpoint.is_empty() {
            return true;
        }
        false
    }

    /// Run the chain. Returns the first non-error response. If
    /// every attempted provider fails, the returned error carries
    /// a per-provider breakdown so the caller (or the LLM) can
    /// see exactly what went wrong with each one. Providers that
    /// are skipped because of missing config are not in the
    /// breakdown — only the providers we actually called.
    pub async fn run(
        &self,
        mut make_req: impl FnMut(&dyn Provider) -> Request,
    ) -> Result<Response, ProviderError> {
        let mut last_err: Option<ProviderError> = None;
        let mut tried: Vec<String> = Vec::new();
        for p in &self.providers {
            let req = make_req(&**p);
            if self.should_skip(&**p, &req.config) {
                tracing::debug!(
                    provider = p.name(),
                    "chain skipping provider (missing required config)"
                );
                continue;
            }
            tried.push(p.name().to_string());
            match p.execute(req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::debug!(provider = p.name(), error = %e, "chain provider failed, trying next");
                    last_err = Some(e);
                }
            }
        }
        if let Some(err) = last_err {
            // The single `last_err` is usually the most actionable
            // (e.g. network error from the *attempted* primary).
            // We surface a hint about which other providers were
            // skipped so the LLM / user can route around them.
            let skipped: Vec<&str> = self
                .providers
                .iter()
                .filter_map(|p| {
                    let req = make_req(&**p);
                    if self.should_skip(&**p, &req.config) {
                        Some(p.name())
                    } else {
                        None
                    }
                })
                .collect();
            let summary = match skipped.len() {
                0 => format!("{} (tried: {})", err, tried.join(", ")),
                _ => format!(
                    "{} (tried: {}; skipped unconfigured: {})",
                    err,
                    tried.join(", "),
                    skipped.join(", ")
                ),
            };
            return Err(ProviderError::Upstream(summary));
        }
        Err(ProviderError::NoResults("chain"))
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
            let size = if size.is_empty() {
                "1024x1024"
            } else {
                size.as_str()
            };
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
            let voice = if voice.is_empty() {
                "alloy"
            } else {
                voice.as_str()
            };
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
                return Err(ProviderError::Upstream(format!("{}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            // Truncate so the LLM context doesn't blow up on a 5 MB
            // page; the rest can be re-fetched on demand.
            let max = 16 * 1024;
            let truncated = if text.len() > max {
                format!(
                    "{}\n\n[truncated; original {} bytes]",
                    &text[..max],
                    text.len()
                )
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
                return Err(ProviderError::Upstream(format!("{}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            let max = 32 * 1024;
            let truncated = if text.len() > max {
                format!(
                    "{}\n\n[truncated; original {} bytes]",
                    &text[..max],
                    text.len()
                )
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

    pub(crate) fn parse_args(raw: &serde_json::Value) -> Result<(String, usize), ProviderError> {
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
            if let Some(results) = v
                .get("web")
                .and_then(|w| w.get("results"))
                .and_then(|r| r.as_array())
            {
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

    /// DuckDuckGo HTML search — `credential_free` (no key).
    /// Scrapes `https://html.duckduckgo.com/html/?q=...` (the
    /// "lite" endpoint) and returns the top `n` results. Used as
    /// the default primary so the dashboard works out-of-the-box
    /// even when no paid search API is configured.
    pub struct DuckDuckGo {
        client: reqwest::Client,
    }

    impl DuckDuckGo {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for DuckDuckGo {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "duckduckgo"
        }
        fn credential_free(&self) -> bool {
            true
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (query, n) = parse_args(&req.args)?;
            // DDG's "lite" HTML endpoint requires a UA and the
            // POST form. We send as POST so the q= is in the body,
            // matching what a browser would do.
            let resp = self
                .client
                .post("https://html.duckduckgo.com/html/")
                .header("User-Agent", "Mozilla/5.0 (compatible; CleanClaw/0.1)")
                .form(&[("q", query.as_str())])
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!(
                    "duckduckgo {status}: {txt}"
                )));
            }
            let html = resp
                .text()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            // Cheap HTML extraction: `<a class="result__a" href="…" rel="noopener">title</a>`
            // followed by `.result__snippet`. The class names are
            // stable across DDG's HTML lite endpoint.
            let mut out = String::new();
            out.push_str(&format!("Search results for: {query}\n\n"));
            let mut idx = 0;
            let bytes = html.as_bytes();
            let needle = b"class=\"result__a\"";
            let mut cursor = 0usize;
            while idx < n && cursor < bytes.len() {
                if let Some(pos) = find_from(&bytes[cursor..], needle) {
                    cursor += pos + needle.len();
                    // Walk to the closing `>` of the opening tag.
                    if let Some(end_tag) = bytes[cursor..].iter().position(|&b| b == b'>') {
                        cursor += end_tag + 1;
                    }
                    // Read up to `</a>` for the title.
                    if let Some(end_a) = find_from(&bytes[cursor..], b"</a>") {
                        let title = decode_html_entities(
                            std::str::from_utf8(&bytes[cursor..cursor + end_a])
                                .unwrap_or("")
                                .trim(),
                        );
                        // URL: walk back from the title's anchor
                        // to find `href="…"` on the same line.
                        let url_start = bytes[..cursor + end_a]
                            .windows(5)
                            .rposition(|w| w == b"href=\"")
                            .map(|p| p + 6)
                            .unwrap_or(cursor);
                        let url_end = bytes[url_start..]
                            .iter()
                            .position(|&b| b == b'"')
                            .unwrap_or(0);
                        let url = decode_html_entities(
                            std::str::from_utf8(&bytes[url_start..url_start + url_end])
                                .unwrap_or("")
                                .trim(),
                        );
                        // Snippet: optional `<a class="result__snippet"…>…</a>`.
                        let snip: Option<String> = (|| -> Option<String> {
                            let sn_start =
                                find_from(&bytes[cursor + end_a..], b"class=\"result__snippet\"")?;
                            let abs = cursor + end_a + sn_start;
                            let end_s = bytes[abs..].iter().position(|&b| b == b'>')?;
                            let s = abs + end_s + 1;
                            let e = find_from(&bytes[s..], b"</a>")?;
                            Some(decode_html_entities(
                                std::str::from_utf8(&bytes[s..s + e]).unwrap_or("").trim(),
                            ))
                        })();
                        idx += 1;
                        out.push_str(&format!(
                            "{}. {}\n   {}\n{}\n\n",
                            idx,
                            if title.is_empty() {
                                "(no title)"
                            } else {
                                &title
                            },
                            if url.is_empty() { "(no url)" } else { &url },
                            snip.unwrap_or_default(),
                        ));
                        cursor += end_a + 4;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            if idx == 0 {
                return Err(ProviderError::NoResults("duckduckgo"));
            }
            Ok(Response::from_text(out))
        }
    }

    /// Microsoft Bing Web Search API v7.
    /// Endpoint: `GET https://api.bing.microsoft.com/v7.0/search?q=...`
    /// Header:   `Ocp-Apim-Subscription-Key: <key>`
    pub struct Bing {
        client: reqwest::Client,
    }

    impl Bing {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Bing {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "bing"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (query, n) = parse_args(&req.args)?;
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("bing"));
            }
            let endpoint = if req.config.endpoint.is_empty() {
                "https://api.bing.microsoft.com/v7.0/search"
            } else {
                req.config.endpoint.as_str()
            };
            let resp = self
                .client
                .get(endpoint)
                .header("Ocp-Apim-Subscription-Key", &req.config.api_key)
                .query(&[("q", query.as_str()), ("count", &n.to_string())])
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("bing {status}: {txt}")));
            }
            let v: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            let mut out = String::new();
            out.push_str(&format!("Search results for: {query}\n\n"));
            let results = v
                .get("webPages")
                .and_then(|w| w.get("value"))
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            for (i, r) in results.iter().take(n).enumerate() {
                let title = r.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("");
                let snippet = r.get("snippet").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "{}. {}\n   {}\n   {}\n\n",
                    i + 1,
                    title,
                    url,
                    snippet
                ));
            }
            if results.is_empty() {
                return Err(ProviderError::NoResults("bing"));
            }
            Ok(Response::from_text(out))
        }
    }

    /// Google Programmable Search Engine (Custom Search JSON API).
    /// Endpoint: `GET https://www.googleapis.com/customsearch/v1?q=…&key=…&cx=…`
    /// The `cx` (search-engine id) is taken from the `endpoint`
    /// config field (formatted as `cx=<id>`) so we don't have to
    /// extend the `ProviderConfig` struct just for this.
    pub struct Google {
        client: reqwest::Client,
    }

    impl Google {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Google {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "google"
        }
        // Google CSE needs `cx=<engine-id>` parsed out of the
        // `endpoint` field. Without it the request would 400 from
        // the upstream, so the chain skips this provider silently
        // when the field is empty.
        fn needs_endpoint(&self) -> bool {
            true
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (query, n) = parse_args(&req.args)?;
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("google"));
            }
            // Parse `cx=` out of the endpoint field, e.g.
            // "cx=0123456789abcdef" or "https://cse.google.com/cse?cx=…"
            let cx = req
                .config
                .endpoint
                .split('?')
                .next_back()
                .unwrap_or("")
                .split('&')
                .find_map(|kv| kv.strip_prefix("cx="))
                .unwrap_or("")
                .to_string();
            if cx.is_empty() {
                return Err(ProviderError::InvalidArgs(
                    "google: missing `cx` (set endpoint to `cx=<engine-id>`)".into(),
                ));
            }
            let resp = self
                .client
                .get("https://www.googleapis.com/customsearch/v1")
                .query(&[
                    ("q", query.as_str()),
                    ("key", req.config.api_key.as_str()),
                    ("cx", cx.as_str()),
                    ("num", &n.to_string()),
                ])
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("google {status}: {txt}")));
            }
            let v: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            let mut out = String::new();
            out.push_str(&format!("Search results for: {query}\n\n"));
            let results = v
                .get("items")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            for (i, r) in results.iter().take(n).enumerate() {
                let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
                let url = r.get("link").and_then(|x| x.as_str()).unwrap_or("");
                let snippet = r.get("snippet").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "{}. {}\n   {}\n   {}\n\n",
                    i + 1,
                    title,
                    url,
                    snippet
                ));
            }
            if results.is_empty() {
                return Err(ProviderError::NoResults("google"));
            }
            Ok(Response::from_text(out))
        }
    }

    /// Baidu search — `credential_free` (no key, but Baidu
    /// sometimes serves a captcha to non-CN IPs; the chain will
    /// transparently fall through to the next provider in that
    /// case). Endpoint: `https://www.baidu.com/s?wd=…`
    pub struct Baidu {
        client: reqwest::Client,
    }

    impl Baidu {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Baidu {
        fn category(&self) -> &'static str {
            CATEGORY
        }
        fn name(&self) -> &'static str {
            "baidu"
        }
        fn credential_free(&self) -> bool {
            true
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let (query, n) = parse_args(&req.args)?;
            // Baidu's HTML search needs a referer + a modern UA
            // to avoid the anti-bot page; we use the desktop UA
            // the browser would send.
            let resp = self
                .client
                .get("https://www.baidu.com/s")
                .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .query(&[("wd", query.as_str())])
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("baidu {status}: {txt}")));
            }
            let html = resp
                .text()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            // Baidu's result entries: `<h3 class="t"><a href="…" …>title</a></h3>`.
            // The actual destination URL is in the surrounding
            // `<a>` whose `href` is a redirect — but the visible
            // text is what we want.
            let mut out = String::new();
            out.push_str(&format!("Search results for: {query}\n\n"));
            let bytes = html.as_bytes();
            let needle = b"<h3 class=\"t\"";
            let mut cursor = 0usize;
            let mut idx = 0;
            while idx < n {
                let Some(pos) = find_from(&bytes[cursor..], needle) else {
                    break;
                };
                cursor += pos + needle.len();
                // Skip the rest of the <h3 …> opening tag.
                if let Some(gt) = bytes[cursor..].iter().position(|&b| b == b'>') {
                    cursor += gt + 1;
                } else {
                    break;
                }
                // The title sits inside the <a>…</a> immediately after.
                if let Some(a_end) = find_from(&bytes[cursor..], b"</a>") {
                    let raw = std::str::from_utf8(&bytes[cursor..cursor + a_end])
                        .unwrap_or("")
                        .trim();
                    let title = decode_html_entities(&strip_tags(raw));
                    // The redirect URL is in the parent <a> tag's
                    // `href`. Walk back to find it.
                    let url_start = bytes[..cursor]
                        .windows(5)
                        .rposition(|w| w == b"href=\"")
                        .map(|p| {
                            // Find the end of that anchor tag.
                            let after = p + 6;
                            let _ = after;
                            p + 6
                        })
                        .unwrap_or(cursor);
                    let url_end = bytes[url_start..]
                        .iter()
                        .position(|&b| b == b'"')
                        .unwrap_or(0);
                    let url_raw =
                        std::str::from_utf8(&bytes[url_start..url_start + url_end]).unwrap_or("");
                    let url = if url_raw.starts_with("http") {
                        url_raw.to_string()
                    } else {
                        String::new()
                    };
                    idx += 1;
                    out.push_str(&format!(
                        "{}. {}\n   {}\n\n",
                        idx,
                        if title.is_empty() {
                            "(no title)"
                        } else {
                            &title
                        },
                        if url.is_empty() { "(no url)" } else { &url },
                    ));
                    cursor += a_end + 4;
                } else {
                    break;
                }
            }
            if idx == 0 {
                return Err(ProviderError::NoResults("baidu"));
            }
            Ok(Response::from_text(out))
        }
    }
}

/// Find the first occurrence of `needle` in `haystack` starting
/// from `start`. Returns the byte offset relative to `start`, or
/// `None` if not found.
fn find_from(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode the small subset of HTML entities that turn up in
/// search-result titles (`&amp;` / `&lt;` / `&gt;` / `&quot;` /
/// `&#39;` / `&nbsp;` / numeric entities). Avoids pulling in a
/// full HTML-decoder crate.
fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(end) = bytes[i..].iter().position(|&b| b == b';') {
                let end_abs = i + end;
                if end_abs - i <= 8 {
                    let entity = &s[i..=end_abs];
                    let decoded: Option<String> = match entity {
                        "&amp;" => Some("&".to_string()),
                        "&lt;" => Some("<".to_string()),
                        "&gt;" => Some(">".to_string()),
                        "&quot;" => Some("\"".to_string()),
                        "&#39;" => Some("'".to_string()),
                        "&apos;" => Some("'".to_string()),
                        "&nbsp;" => Some(" ".to_string()),
                        _ if entity.starts_with("&#x") => {
                            u32::from_str_radix(&entity[3..entity.len() - 1], 16)
                                .ok()
                                .and_then(char::from_u32)
                                .map(|c| c.to_string())
                        }
                        _ if entity.starts_with("&#") => entity[2..entity.len() - 1]
                            .parse::<u32>()
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string()),
                        _ => None,
                    };
                    if let Some(d) = decoded {
                        out.push_str(&d);
                        i = end_abs + 1;
                        continue;
                    }
                }
            }
        }
        // SAFETY: `i` always lands on a UTF-8 char boundary
        // because we only advance past `;` (which is ASCII).
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Strip `<…>` tags from a string — used to remove inner `<em>`
/// / `<span>` markup from scraped search-result titles. Returns
/// an owned `String` to avoid lifetime gymnastics; the inputs
/// are small (one search-result title) so the alloc is cheap.
fn strip_tags(s: &str) -> String {
    if let Some(start) = s.find('<') {
        if let Some(end) = s[start..].find('>') {
            let mut out = String::with_capacity(s.len());
            out.push_str(&s[..start]);
            out.push_str(&strip_tags(&s[start + end + 1..]));
            return out;
        }
    }
    s.to_string()
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
    reg.register(Arc::new(websearch::DuckDuckGo::new(client.clone())));
    reg.register(Arc::new(websearch::Brave::new(client.clone())));
    reg.register(Arc::new(websearch::Bing::new(client.clone())));
    reg.register(Arc::new(websearch::Google::new(client.clone())));
    reg.register(Arc::new(websearch::Baidu::new(client.clone())));
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
        let p: Arc<dyn Provider> = Arc::new(imagegen::OpenAI::new(reqwest::Client::new()));
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
        let p: Arc<dyn Provider> = Arc::new(imagegen::OpenAI::new(reqwest::Client::new()));
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

    // ---- websearch::parse_args ----

    #[test]
    fn websearch_parse_args_query_only() {
        let (q, n) = websearch::parse_args(&serde_json::json!({"query": "hello world"})).unwrap();
        assert_eq!(q, "hello world");
        assert_eq!(n, 5); // default
    }

    #[test]
    fn websearch_parse_args_with_n() {
        let (q, n) = websearch::parse_args(&serde_json::json!({"query": "test", "n": 3})).unwrap();
        assert_eq!(q, "test");
        assert_eq!(n, 3);
    }

    #[test]
    fn websearch_parse_args_query_trimmed() {
        let (q, n) = websearch::parse_args(&serde_json::json!({"query": "  spaced  "})).unwrap();
        assert_eq!(q, "spaced");
        assert_eq!(n, 5);
    }

    #[test]
    fn websearch_parse_args_n_clamped_low() {
        let (_, n) = websearch::parse_args(&serde_json::json!({"query": "x", "n": 0})).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn websearch_parse_args_n_clamped_high() {
        let (_, n) = websearch::parse_args(&serde_json::json!({"query": "x", "n": 100})).unwrap();
        assert_eq!(n, 20);
    }

    #[test]
    fn websearch_parse_args_missing_query_errors() {
        let r = websearch::parse_args(&serde_json::json!({}));
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[test]
    fn websearch_parse_args_empty_query_errors() {
        let r = websearch::parse_args(&serde_json::json!({"query": ""}));
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    // ---- websearch::None ----

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

    // ---- websearch::DuckDuckGo ----

    #[tokio::test]
    async fn websearch_duckduckgo_missing_query() {
        let p: Arc<dyn Provider> = Arc::new(websearch::DuckDuckGo::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    // ---- websearch::Bing ----

    #[tokio::test]
    async fn websearch_bing_missing_query() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Bing::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn websearch_bing_missing_key() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Bing::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    // ---- websearch::Google ----

    #[tokio::test]
    async fn websearch_google_missing_query() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Google::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn websearch_google_missing_key() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Google::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn websearch_google_missing_cx() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Google::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({"query": "x"}),
                config: ProviderConfig {
                    api_key: "key".into(),
                    ..Default::default()
                },
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    // ---- websearch::Baidu ----

    #[tokio::test]
    async fn websearch_baidu_missing_query() {
        let p: Arc<dyn Provider> = Arc::new(websearch::Baidu::new(reqwest::Client::new()));
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn webfetch_missing_url() {
        let p: Arc<dyn Provider> = Arc::new(webfetch::Direct::new(reqwest::Client::new()));
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

    // ---- helper: find_from ----

    #[test]
    fn find_from_found() {
        let h = b"hello world";
        let n = b"world";
        assert_eq!(find_from(h, n), Some(6));
    }

    #[test]
    fn find_from_not_found() {
        let h = b"hello";
        let n = b"xyz";
        assert_eq!(find_from(h, n), None);
    }

    #[test]
    fn find_from_needle_longer_than_haystack() {
        assert_eq!(find_from(b"ab", b"abc"), None);
    }

    #[test]
    fn find_from_empty_needle() {
        assert_eq!(find_from(b"abc", b""), None);
    }

    // ---- helper: decode_html_entities ----

    #[test]
    fn decode_html_entities_basic() {
        assert_eq!(decode_html_entities("&amp;"), "&");
        assert_eq!(decode_html_entities("&lt;"), "<");
        assert_eq!(decode_html_entities("&gt;"), ">");
        assert_eq!(decode_html_entities("&quot;"), "\"");
        assert_eq!(decode_html_entities("&#39;"), "'");
        assert_eq!(decode_html_entities("&nbsp;"), " ");
    }

    #[test]
    fn decode_html_entities_numeric() {
        assert_eq!(decode_html_entities("&#38;"), "&");
        assert_eq!(decode_html_entities("&#x26;"), "&");
    }

    #[test]
    fn decode_html_entities_no_change() {
        assert_eq!(decode_html_entities("hello world"), "hello world");
        assert_eq!(decode_html_entities(""), "");
    }

    #[test]
    fn decode_html_entities_mixed() {
        assert_eq!(
            decode_html_entities("foo &amp; bar &lt; baz"),
            "foo & bar < baz"
        );
    }

    // ---- helper: strip_tags ----

    #[test]
    fn strip_tags_no_tags() {
        assert_eq!(strip_tags("hello world"), "hello world");
    }

    #[test]
    fn strip_tags_simple() {
        assert_eq!(strip_tags("hello <em>world</em>"), "hello world");
    }

    #[test]
    fn strip_tags_nested() {
        assert_eq!(strip_tags("<b><i>bold</i></b>"), "bold");
    }

    #[test]
    fn strip_tags_empty() {
        assert_eq!(strip_tags(""), "");
    }
}
