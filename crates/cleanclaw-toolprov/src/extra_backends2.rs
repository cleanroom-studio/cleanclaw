//! Second batch of tool-provider backends. Mirrors the remaining
//! files in :
//!
//!   * imagegen/openai    — DALL·E 3 via chat.completions image gen
//!   * imagegen/none      — explicit no-op (chain terminates here)
//!   * tts/openai         — `/v1/audio/speech`
//!   * tts/none           — explicit no-op
//!   * webfetch/direct    — net/http GET + HTML strip + truncate
//!   * webfetch/jina      — `https://r.jina.ai/<url>` reader API
//!   * websearch/brave    — `https://api.search.brave.com/res/v1/web/search`
//!   * websearch/none     — explicit no-op
//!
//! All 8 follow the same Provider trait as `extra_backends.rs`:
//! `category() / name() / execute(Request) -> Result<Response, _>`.

use super::*;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::extra_backends::str_field;

// =====================================================================
// imagegen/openai
// =====================================================================

/// OpenAI image generation via `POST /v1/images/generations` with
/// DALL·E 3. Auth is `Authorization: Bearer`. Returns the first
/// generated image URL.
pub mod openai_imagegen {
    use super::*;

    pub struct OpenAi {
        client: reqwest::Client,
    }

    impl OpenAi {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for OpenAi {
        fn category(&self) -> &'static str {
            "image_gen"
        }
        fn name(&self) -> &'static str {
            "openai"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let prompt = str_field(&req.args, "prompt");
            if prompt.is_empty() {
                return Err(ProviderError::InvalidArgs(
                    "imagegen: prompt required".into(),
                ));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("openai"));
            }
            let model = if req.config.model.is_empty() {
                "dall-e-3"
            } else {
                req.config.model.as_str()
            };
            let endpoint = if req.config.endpoint.is_empty() {
                "https://api.openai.com/v1/images/generations"
            } else {
                req.config.endpoint.as_str()
            };
            let body = json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "size": "1024x1024",
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
                return Err(ProviderError::Upstream(format!("openai {status}: {txt}")));
            }
            let v: Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            let url = v
                .get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|i| i.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            Ok(Response::from_text(format!("[openai] {model} → {url}")))
        }
    }
}

// =====================================================================
// imagegen/none — explicit no-op (chain terminator)
// =====================================================================

/// `none` is a chain terminator. When a fallback chain reaches this
/// provider, the chain stops and surfaces a "no provider available"
/// error to the agent — without it the agent would keep retrying
/// through a different configured chain and burn more rounds.
pub mod none_imagegen {
    use super::*;

    pub struct None;

    impl None {
        pub fn new() -> Self {
            Self
        }
    }

    #[async_trait]
    impl Provider for None {
        fn category(&self) -> &'static str {
            "image_gen"
        }
        fn name(&self) -> &'static str {
            "none"
        }
        async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
            Err(ProviderError::NotConfigured(
                "imagegen: no provider configured".into(),
            ))
        }
    }
}

// =====================================================================
// tts/openai
// =====================================================================

/// OpenAI TTS via `POST /v1/audio/speech`. Returns audio bytes;
/// the agent runtime projects the byte count to text (IM channels
/// pipe the raw bytes through their media-upload path).
pub mod openai_tts {
    use super::*;

    pub struct OpenAi {
        client: reqwest::Client,
    }

    impl OpenAi {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for OpenAi {
        fn category(&self) -> &'static str {
            "tts"
        }
        fn name(&self) -> &'static str {
            "openai"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let text = str_field(&req.args, "text");
            if text.is_empty() {
                return Err(ProviderError::InvalidArgs("tts: text required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("openai"));
            }
            let model = if req.config.model.is_empty() {
                "tts-1"
            } else {
                req.config.model.as_str()
            };
            let voice = if req.config.endpoint.is_empty() {
                "alloy"
            } else {
                req.config.endpoint.as_str()
            };
            let endpoint = req
                .config
                .options
                .get("base_url")
                .map(|s| s.as_str())
                .unwrap_or("https://api.openai.com/v1/audio/speech");
            let body = json!({
                "model": model,
                "input": text,
                "voice": voice,
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
                return Err(ProviderError::Upstream(format!(
                    "openai-tts {status}: {txt}"
                )));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            Ok(Response::from_text(format!(
                "[openai-tts] generated {} bytes of mp3 (model={model} voice={voice})",
                bytes.len()
            )))
        }
    }
}

// =====================================================================
// tts/none — chain terminator (same rationale as imagegen/none)
// =====================================================================

pub mod none_tts {
    use super::*;

    pub struct None;

    impl None {
        pub fn new() -> Self {
            Self
        }
    }

    #[async_trait]
    impl Provider for None {
        fn category(&self) -> &'static str {
            "tts"
        }
        fn name(&self) -> &'static str {
            "none"
        }
        async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
            Err(ProviderError::NotConfigured(
                "tts: no provider configured".into(),
            ))
        }
    }
}

// =====================================================================
// webfetch/direct — no-key built-in fetcher
// =====================================================================

/// `direct` is the no-key built-in fetcher: net/http GET with a
/// recognisable User-Agent + 30s timeout. Returns the raw response
/// body truncated to 4KB. Promotes 429/5xx to `ProviderError::Retry`
/// so the chain falls through.
pub mod direct_fetch {
    use super::*;

    const TIMEOUT_SECS: u64 = 30;
    const MAX_BODY: usize = 4096;
    const USER_AGENT: &str = "CleanClaw/1.0 (AI Agent Web Fetcher)";

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
            "web_fetch"
        }
        fn name(&self) -> &'static str {
            "direct"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let url = str_field(&req.args, "url");
            if url.is_empty() {
                return Err(ProviderError::InvalidArgs("webfetch: url required".into()));
            }
            let resp = self
                .client
                .get(url)
                .header("User-Agent", USER_AGENT)
                .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
                .send()
                .await
                .map_err(|e| ProviderError::Retry(format!("direct fetch: {e}")))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if status.as_u16() == 429 || status.is_server_error() {
                return Err(ProviderError::Retry(format!("direct HTTP {status}")));
            }
            if !status.is_success() {
                return Err(ProviderError::Upstream(format!("direct HTTP {status}")));
            }
            let truncated = if body.len() > MAX_BODY {
                format!("{}…", &body[..MAX_BODY])
            } else {
                body
            };
            Ok(Response::from_text(truncated))
        }
    }
}

// =====================================================================
// webfetch/jina
// =====================================================================

/// Jina reader: `GET https://r.jina.ai/<url>` with bearer auth.
/// Returns cleaned markdown; the agent can pipe the result
/// directly into a context block. Up to 4KB truncated.
pub mod jina_fetch {
    use super::*;

    const MAX_BODY: usize = 4096;

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
            "web_fetch"
        }
        fn name(&self) -> &'static str {
            "jina"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let url = str_field(&req.args, "url");
            if url.is_empty() {
                return Err(ProviderError::InvalidArgs("webfetch: url required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("jina"));
            }
            let endpoint = format!("https://r.jina.ai/{url}");
            let resp = self
                .client
                .get(&endpoint)
                .bearer_auth(&req.config.api_key)
                .header("X-Return-Format", "markdown")
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("jina {status}: {txt}")));
            }
            let body = resp
                .text()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            let truncated = if body.len() > MAX_BODY {
                format!("{}…", &body[..MAX_BODY])
            } else {
                body
            };
            Ok(Response::from_text(truncated))
        }
    }
}

// =====================================================================
// websearch/brave
// =====================================================================

/// Brave Search: `GET https://api.search.brave.com/res/v1/web/search`
/// with `X-Subscription-Token: <key>`. Returns the top 5 results
/// as numbered title + url lines.
pub mod brave_search {
    use super::*;

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
            "web_search"
        }
        fn name(&self) -> &'static str {
            "brave"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let q = str_field(&req.args, "query");
            if q.is_empty() {
                return Err(ProviderError::InvalidArgs(
                    "websearch: query required".into(),
                ));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("brave"));
            }
            let resp = self
                .client
                .get("https://api.search.brave.com/res/v1/web/search")
                .header("X-Subscription-Token", &req.config.api_key)
                .query(&[("q", q), ("count", "5")])
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("brave {status}: {txt}")));
            }
            let v: Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Decode(e.to_string()))?;
            let results = v
                .get("web")
                .and_then(|w| w.get("results"))
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            let mut out = String::new();
            for (i, r) in results.iter().take(5).enumerate() {
                let title = r.get("title").and_then(|t| t.as_str()).unwrap_or("");
                let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
                out.push_str(&format!("{}. {}\n   {}\n", i + 1, title, url));
            }
            if out.is_empty() {
                return Err(ProviderError::NoResults("brave"));
            }
            Ok(Response::from_text(out))
        }
    }
}

// =====================================================================
// websearch/none — chain terminator
// =====================================================================

pub mod none_search {
    use super::*;

    pub struct None;

    impl None {
        pub fn new() -> Self {
            Self
        }
    }

    #[async_trait]
    impl Provider for None {
        fn category(&self) -> &'static str {
            "web_search"
        }
        fn name(&self) -> &'static str {
            "none"
        }
        async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
            Err(ProviderError::NotConfigured(
                "websearch: no provider configured".into(),
            ))
        }
    }
}

// =====================================================================
// register_extras2
// =====================================================================

/// Register the 8 additional backends. Call after `register_extras`
/// so the original 7 stay registered too.
pub fn register_extras2(reg: &Registry, client: &reqwest::Client) {
    reg.register(Arc::new(openai_imagegen::OpenAi::new(client.clone())));
    reg.register(Arc::new(none_imagegen::None::new()));
    reg.register(Arc::new(openai_tts::OpenAi::new(client.clone())));
    reg.register(Arc::new(none_tts::None::new()));
    reg.register(Arc::new(direct_fetch::Direct::new(client.clone())));
    reg.register(Arc::new(jina_fetch::Jina::new(client.clone())));
    reg.register(Arc::new(brave_search::Brave::new(client.clone())));
    reg.register(Arc::new(none_search::None::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> ProviderConfig {
        ProviderConfig::default()
    }

    #[test]
    fn register_extras2_adds_all_eight() {
        let r = Registry::new();
        let client = reqwest::Client::new();
        register_extras2(&r, &client);
        assert_eq!(r.for_category("image_gen").len(), 2); // openai + none
        assert_eq!(r.for_category("tts").len(), 2); // openai + none
        assert_eq!(r.for_category("web_fetch").len(), 2); // direct + jina
        assert_eq!(r.for_category("web_search").len(), 2); // brave + none
    }

    #[tokio::test]
    async fn none_imagegen_returns_not_configured() {
        let p = none_imagegen::None::new();
        let r = p
            .execute(Request {
                args: json!({"prompt": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::NotConfigured(_))));
    }

    #[tokio::test]
    async fn none_tts_returns_not_configured() {
        let p = none_tts::None::new();
        let r = p
            .execute(Request {
                args: json!({"text": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::NotConfigured(_))));
    }

    #[tokio::test]
    async fn none_search_returns_not_configured() {
        let p = none_search::None::new();
        let r = p
            .execute(Request {
                args: json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::NotConfigured(_))));
    }

    #[tokio::test]
    async fn openai_imagegen_requires_prompt() {
        let p = openai_imagegen::OpenAi::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({}),
                config: ProviderConfig {
                    api_key: "key".into(),
                    ..empty_config()
                },
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn openai_imagegen_requires_api_key() {
        let p = openai_imagegen::OpenAi::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({"prompt": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn openai_tts_requires_text() {
        let p = openai_tts::OpenAi::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({}),
                config: ProviderConfig {
                    api_key: "key".into(),
                    ..empty_config()
                },
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn jina_fetch_requires_api_key() {
        let p = jina_fetch::Jina::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({"url": "https://example.com"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn direct_fetch_requires_url() {
        let p = direct_fetch::Direct::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn brave_search_requires_api_key() {
        let p = brave_search::Brave::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }
}
