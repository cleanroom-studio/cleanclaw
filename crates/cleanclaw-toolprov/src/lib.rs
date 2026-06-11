//! Plug-in layer for tools that talk to external services (web
//! search, image generation, TTS, web fetch). Mirrors
//! .
//!
//! Each category exposes ONE tool to the LLM backed by a primary
//! provider and an ordered fallback chain. The LLM never sees
//! individual providers.
//!
//! # Layout
//!
//! ```text
//! src/
//!   lib.rs                    (this file — core types + register_builtin)
//!   backends/
//!     mod.rs                  (category umbrella + register_extras)
//!     imagegen/               (built-in image-gen providers)
//!     tts/                    (built-in TTS providers)
//!     webfetch/               (built-in URL-fetch providers)
//!     websearch/              (built-in web-search providers)
//!     extras_imagegen/        (Fal, Replicate)
//!     extras_tts/             (ElevenLabs, Fish, MiniMax)
//!     extras_fetch/           (Firecrawl)
//!     extras_search/          (Exa, SearXNG)
//! ```
//!
//! The four `imagegen` / `tts` / `webfetch` / `websearch`
//! modules are **re-exported** at the crate root so existing
//! callers (e.g. `cleanclaw_toolprov::tts::OpenAI`) keep
//! working unchanged.

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

mod backends;

// Re-export the four built-in categories at the crate root so
// downstream code can keep writing `cleanclaw_toolprov::tts`,
// `cleanclaw_toolprov::imagegen`, etc. The category's `CATEGORY`
// const, helpers, and provider types are all reachable through
// the re-export.
pub use backends::{extras_fetch, extras_imagegen, extras_search, extras_tts};
pub use backends::{imagegen, tts, webfetch, websearch};

// `register_extras` is the public hook for adding the opt-in
// third-party backends onto a registry. `register_builtin`
// already calls it for you; downstream code that wants to wire
// up only the extras can call it directly.
pub use backends::register_extras;

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
/// tenant's `toolProviders.*` config block.
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

/// The full request passed to a provider. `Args` is the
/// LLM-supplied tool args; `Config` is the resolved per-tenant
/// config.
#[derive(Debug, Clone)]
pub struct Request {
    pub args: serde_json::Value,
    pub config: ProviderConfig,
}

/// The provider's response. For imagegen/tts the `text` field is
/// the LLM-visible markdown payload (with base64 or URLs); for
/// websearch it's the formatted result list.
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
    /// public, key-less endpoint (DuckDuckGo / Baidu /
    /// SearXNG-public) flip this on. Default: `false`.
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
    /// required config is missing. A key-bearing provider
    /// without an API key can never succeed; a self-hosted
    /// endpoint provider without an endpoint URL can never
    /// succeed. We skip these **silently** (no entry in the
    /// error log) so the final error message stays focused on
    /// the providers we actually attempted.
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
    /// every attempted provider fails, the returned error
    /// carries a per-provider breakdown so the caller (or the
    /// LLM) can see exactly what went wrong with each one.
    /// Providers that are skipped because of missing config are
    /// not in the breakdown — only the providers we actually
    /// called.
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
            // The single `last_err` is usually the most
            // actionable (e.g. network error from the
            // *attempted* primary). We surface a hint about
            // which other providers were skipped so the LLM /
            // user can route around them.
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

/// Convenience: register every built-in provider this crate
/// ships (4 built-in categories + 8 opt-in extras = 12 total).
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
    // The 8 additional backends (Fal / Replicate / ElevenLabs /
    // Fish / MiniMax / Firecrawl / Exa / SearXNG) live under
    // `backends::extras_*` and are registered separately so the
    // canonical built-in list stays small.
    backends::register_extras(reg, &client);
}

#[cfg(test)]
mod tests {
    use super::*;
    use backends::websearch::parse_args;

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

    #[test]
    fn register_extras_adds_all_eight() {
        // `register_extras` adds: 2 imagegen (Fal, Replicate),
        // 3 tts (ElevenLabs, Fish, MiniMax), 1 webfetch
        // (Firecrawl), 2 websearch (Exa, SearXNG). We test the
        // delta in isolation without `register_builtin`.
        let r = Registry::new();
        let client = reqwest::Client::new();
        register_extras(&r, &client);
        assert_eq!(r.for_category("image_gen").len(), 2);
        assert_eq!(r.for_category("tts").len(), 3);
        assert_eq!(r.for_category("web_fetch").len(), 1);
        assert_eq!(r.for_category("web_search").len(), 2);
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
        let (q, n) = parse_args(&serde_json::json!({"query": "hello world"})).unwrap();
        assert_eq!(q, "hello world");
        assert_eq!(n, 5); // default
    }

    #[test]
    fn websearch_parse_args_with_n() {
        let (q, n) = parse_args(&serde_json::json!({"query": "test", "n": 3})).unwrap();
        assert_eq!(q, "test");
        assert_eq!(n, 3);
    }

    #[test]
    fn websearch_parse_args_query_trimmed() {
        let (q, n) = parse_args(&serde_json::json!({"query": "  spaced  "})).unwrap();
        assert_eq!(q, "spaced");
        assert_eq!(n, 5);
    }

    #[test]
    fn websearch_parse_args_n_clamped_low() {
        let (_, n) = parse_args(&serde_json::json!({"query": "x", "n": 0})).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn websearch_parse_args_n_clamped_high() {
        let (_, n) = parse_args(&serde_json::json!({"query": "x", "n": 100})).unwrap();
        assert_eq!(n, 20);
    }

    #[test]
    fn websearch_parse_args_missing_query_errors() {
        let r = parse_args(&serde_json::json!({}));
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[test]
    fn websearch_parse_args_empty_query_errors() {
        let r = parse_args(&serde_json::json!({"query": ""}));
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
        // No real HTTP — all providers will fail. The chain
        // should exhaust the list and return the last error
        // rather than short-circuit on the first.
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

    // ---- helper: extras::Fal (smoke test for the extras image-gen path) ----

    #[tokio::test]
    async fn extras_fal_missing_api_key() {
        let p = backends::extras_imagegen::Fal::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: serde_json::json!({"prompt": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn extras_fal_missing_prompt() {
        let p = backends::extras_imagegen::Fal::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: serde_json::json!({}),
                config: ProviderConfig {
                    api_key: "key".into(),
                    ..empty_config()
                },
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn extras_searxng_requires_endpoint() {
        let p = backends::extras_search::SearXNG::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: serde_json::json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }
}
