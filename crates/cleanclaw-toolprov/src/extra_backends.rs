//! Additional tool-provider backends. Mirrors the rest of
//! :
//!
//!   * imagegen/fal         — `https://fal.run/<model>`
//!   * imagegen/replicate   — `https://api.replicate.com/v1/models/<owner>/<name>/predictions`
//!   * tts/elevenlabs       — `https://api.elevenlabs.io/v1/text-to-speech/{voice_id}`
//!   * tts/fish             — `https://api.fish.audio/v1/tts`
//!   * tts/minimax          — `https://api.minimaxi.com/v1/t2a_v2`
//!   * webfetch/firecrawl   — `https://api.firecrawl.dev/v1/scrape`
//!   * websearch/exa        — `https://api.exa.ai/search`
//!   * websearch/searxng    — `<endpoint>/search?q=...&format=json`
//!
//! All 7 follow the same Provider trait as the in-file backends:
//! `category() / name() / execute(Request) -> Result<Response, _>`.

use super::*;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Small helper: pull a `&str` field out of a JSON args blob.
pub fn str_field<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

// =====================================================================
// imagegen/fal
// =====================================================================

/// Fal posts to `https://fal.run/<model-path>`. Auth is
/// `Key <token>`. The model path follows the convention
/// `fal-ai/<model>[/<variant>]`. Returns the first image URL from
/// the response.
pub mod fal_imagegen {
    use super::*;

    pub struct Fal {
        client: reqwest::Client,
    }

    impl Fal {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Fal {
        fn category(&self) -> &'static str {
            "image_gen"
        }
        fn name(&self) -> &'static str {
            "fal"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let prompt = str_field(&req.args, "prompt");
            if prompt.is_empty() {
                return Err(ProviderError::InvalidArgs("imagegen: prompt required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("fal"));
            }
            let model = if req.config.model.is_empty() {
                "fal-ai/flux/schnell"
            } else {
                req.config.model.as_str()
            };
            let url = format!("https://fal.run/{model}");
            let body = json!({
                "prompt": prompt,
                "image_size": "square_hd",
                "num_images": 1,
                "enable_safety_checker": true,
            });
            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Key {}", req.config.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("fal {status}: {txt}")));
            }
            let v: Value = resp.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
            let image_url = v
                .get("images")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|i| i.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            Ok(Response::from_text(format!("[fal] {model} → {image_url}")))
        }
    }
}

// =====================================================================
// imagegen/replicate
// =====================================================================

/// Replicate: `POST /v1/models/<owner>/<name>/predictions` with
/// `Prefer: wait` for a synchronous response.
pub mod replicate_imagegen {
    use super::*;

    pub struct Replicate {
        client: reqwest::Client,
    }

    impl Replicate {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Replicate {
        fn category(&self) -> &'static str {
            "image_gen"
        }
        fn name(&self) -> &'static str {
            "replicate"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let prompt = str_field(&req.args, "prompt");
            if prompt.is_empty() {
                return Err(ProviderError::InvalidArgs("imagegen: prompt required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("replicate"));
            }
            let model = if req.config.model.is_empty() {
                "black-forest-labs/flux-schnell"
            } else {
                req.config.model.as_str()
            };
            let url = format!("https://api.replicate.com/v1/models/{model}/predictions");
            let body = json!({
                "input": {"prompt": prompt},
            });
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&req.config.api_key)
                .header("Prefer", "wait")
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("replicate {status}: {txt}")));
            }
            let v: Value = resp.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
            let image_url = v
                .get("output")
                .and_then(|o| match o {
                    Value::Array(a) => a.first().and_then(|x| x.as_str()),
                    Value::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("");
            Ok(Response::from_text(format!("[replicate] {model} → {image_url}")))
        }
    }
}

// =====================================================================
// tts/elevenlabs
// =====================================================================

/// ElevenLabs posts to `/v1/text-to-speech/{voice_id}` with the
/// API key in the non-standard `xi-api-key` header. Default voice
/// is "Rachel" (long-standing built-in, available on every tier).
pub mod elevenlabs_tts {
    use super::*;

    const DEFAULT_VOICE: &str = "21m00Tcm4TlvDq8ikWAM";
    const DEFAULT_MODEL: &str = "eleven_multilingual_v2";

    pub struct ElevenLabs {
        client: reqwest::Client,
    }

    impl ElevenLabs {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for ElevenLabs {
        fn category(&self) -> &'static str {
            "tts"
        }
        fn name(&self) -> &'static str {
            "elevenlabs"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let text = str_field(&req.args, "text");
            if text.is_empty() {
                return Err(ProviderError::InvalidArgs("tts: text required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("elevenlabs"));
            }
            let model = if req.config.model.is_empty() {
                DEFAULT_MODEL
            } else {
                req.config.model.as_str()
            };
            let voice = if req.config.endpoint.is_empty() {
                DEFAULT_VOICE
            } else {
                req.config.endpoint.as_str()
            };
            let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice}");
            let body = json!({
                "text": text,
                "model_id": model,
                "voice_settings": {"stability": 0.5, "similarity_boost": 0.75},
            });
            let resp = self
                .client
                .post(&url)
                .header("xi-api-key", &req.config.api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "audio/mpeg")
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("elevenlabs {status}: {txt}")));
            }
            let bytes = resp.bytes().await.map_err(|e| ProviderError::Http(e.to_string()))?;
            Ok(Response::from_text(format!(
                "[elevenlabs] generated {} bytes of mpeg (model={model} voice={voice})",
                bytes.len()
            )))
        }
    }
}

// =====================================================================
// tts/fish
// =====================================================================

/// Fish Audio: `POST https://api.fish.audio/v1/tts` with bearer
/// auth. The body is `{text, reference_id?, format}`; the response
/// is audio bytes. Default format is `mp3`.
pub mod fish_tts {
    use super::*;

    pub struct Fish {
        client: reqwest::Client,
    }

    impl Fish {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Fish {
        fn category(&self) -> &'static str {
            "tts"
        }
        fn name(&self) -> &'static str {
            "fish"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let text = str_field(&req.args, "text");
            if text.is_empty() {
                return Err(ProviderError::InvalidArgs("tts: text required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("fish"));
            }
            let reference_id = str_field(&req.args, "reference_id");
            let mut body = json!({
                "text": text,
                "format": "mp3",
            });
            if !reference_id.is_empty() {
                body["reference_id"] = json!(reference_id);
            }
            let resp = self
                .client
                .post("https://api.fish.audio/v1/tts")
                .bearer_auth(&req.config.api_key)
                .header("model", "speech-1.6")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("fish {status}: {txt}")));
            }
            let bytes = resp.bytes().await.map_err(|e| ProviderError::Http(e.to_string()))?;
            Ok(Response::from_text(format!("[fish] generated {} bytes of mp3", bytes.len())))
        }
    }
}

// =====================================================================
// tts/minimax
// =====================================================================

/// MiniMax T2A v2: `POST https://api.minimaxi.com/v1/t2a_v2` with
/// bearer auth. The body carries a `voice_setting` block; the
/// response is audio hex-encoded into `data.audio`.
pub mod minimax_tts {
    use super::*;

    pub struct MiniMax {
        client: reqwest::Client,
    }

    impl MiniMax {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for MiniMax {
        fn category(&self) -> &'static str {
            "tts"
        }
        fn name(&self) -> &'static str {
            "minimax"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let text = str_field(&req.args, "text");
            if text.is_empty() {
                return Err(ProviderError::InvalidArgs("tts: text required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("minimax"));
            }
            let model = if req.config.model.is_empty() {
                "speech-02-hd"
            } else {
                req.config.model.as_str()
            };
            let voice_id = if req.config.endpoint.is_empty() {
                "male-qn-jingying"
            } else {
                req.config.endpoint.as_str()
            };
            let body = json!({
                "model": model,
                "text": text,
                "stream": false,
                "voice_setting": {
                    "voice_id": voice_id,
                    "speed": 1.0,
                    "vol": 1.0,
                    "pitch": 0,
                },
                "audio_setting": {
                    "sample_rate": 32000,
                    "bitrate": 128000,
                    "format": "mp3",
                },
            });
            let resp = self
                .client
                .post("https://api.minimaxi.com/v1/t2a_v2")
                .bearer_auth(&req.config.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("minimax {status}: {txt}")));
            }
            let v: Value = resp.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
            let audio_bytes = v
                .get("data")
                .and_then(|d| d.get("audio"))
                .and_then(|a| a.as_str())
                .map(|hex| {
                    // hex-decode (a→10) — short, hand-rolled
                    (0..hex.len())
                        .step_by(2)
                        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                        .collect::<Vec<u8>>()
                        .len()
                })
                .unwrap_or(0);
            Ok(Response::from_text(format!(
                "[minimax] generated {audio_bytes} bytes of mp3 (model={model} voice={voice_id})"
            )))
        }
    }
}

// =====================================================================
// webfetch/firecrawl
// =====================================================================

/// Firecrawl: `POST https://api.firecrawl.dev/v1/scrape` returns
/// the page as cleaned markdown. Auth is `Authorization: Bearer`.
pub mod firecrawl_fetch {
    use super::*;

    pub struct Firecrawl {
        client: reqwest::Client,
    }

    impl Firecrawl {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Firecrawl {
        fn category(&self) -> &'static str {
            "web_fetch"
        }
        fn name(&self) -> &'static str {
            "firecrawl"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let url = str_field(&req.args, "url");
            if url.is_empty() {
                return Err(ProviderError::InvalidArgs("webfetch: url required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("firecrawl"));
            }
            let body = json!({
                "url": url,
                "formats": ["markdown"],
                "onlyMainContent": true,
            });
            let resp = self
                .client
                .post("https://api.firecrawl.dev/v1/scrape")
                .bearer_auth(&req.config.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("firecrawl {status}: {txt}")));
            }
            let v: Value = resp.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
            let markdown = v
                .get("data")
                .and_then(|d| d.get("markdown"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            let truncated = if markdown.len() > 4000 {
                format!("{}…", &markdown[..4000])
            } else {
                markdown.to_string()
            };
            Ok(Response::from_text(truncated))
        }
    }
}

// =====================================================================
// websearch/exa
// =====================================================================

/// Exa: `POST https://api.exa.ai/search` with bearer auth. The
/// response is a `results` array; we project the top 5 to text.
pub mod exa_search {
    use super::*;

    pub struct Exa {
        client: reqwest::Client,
    }

    impl Exa {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for Exa {
        fn category(&self) -> &'static str {
            "web_search"
        }
        fn name(&self) -> &'static str {
            "exa"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let q = str_field(&req.args, "query");
            if q.is_empty() {
                return Err(ProviderError::InvalidArgs("websearch: query required".into()));
            }
            if req.config.api_key.is_empty() {
                return Err(ProviderError::MissingApiKey("exa"));
            }
            let body = json!({
                "query": q,
                "numResults": 5,
                "useAutoprompt": false,
                "type": "neural",
            });
            let resp = self
                .client
                .post("https://api.exa.ai/search")
                .bearer_auth(&req.config.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("exa {status}: {txt}")));
            }
            let v: Value = resp.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
            let results = v.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
            let mut out = String::new();
            for (i, r) in results.iter().take(5).enumerate() {
                let title = r.get("title").and_then(|t| t.as_str()).unwrap_or("");
                let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
                out.push_str(&format!("{}. {}\n   {}\n", i + 1, title, url));
            }
            if out.is_empty() {
                return Err(ProviderError::NoResults("exa"));
            }
            Ok(Response::from_text(out))
        }
    }
}

// =====================================================================
// websearch/searxng
// =====================================================================

/// SearXNG: `<endpoint>/search?q=<query>&format=json`. No auth.
/// `endpoint` is required (the operator must self-host).
pub mod searxng_search {
    use super::*;

    pub struct SearXNG {
        client: reqwest::Client,
    }

    impl SearXNG {
        pub fn new(client: reqwest::Client) -> Self {
            Self { client }
        }
    }

    #[async_trait]
    impl Provider for SearXNG {
        fn category(&self) -> &'static str {
            "web_search"
        }
        fn name(&self) -> &'static str {
            "searxng"
        }
        async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
            let q = str_field(&req.args, "query");
            if q.is_empty() {
                return Err(ProviderError::InvalidArgs("websearch: query required".into()));
            }
            let endpoint = if req.config.endpoint.is_empty() {
                return Err(ProviderError::InvalidArgs(
                    "searxng: endpoint required in config".into(),
                ));
            } else {
                req.config.endpoint.trim_end_matches('/')
            };
            let url = format!("{}/search?q={}&format=json&categories=general", endpoint, urlencode(q));
            let resp = self
                .client
                .get(&url)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!("searxng {status}: {txt}")));
            }
            let v: Value = resp.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
            let results = v.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
            let mut out = String::new();
            for (i, r) in results.iter().take(5).enumerate() {
                let title = r.get("title").and_then(|t| t.as_str()).unwrap_or("");
                let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
                out.push_str(&format!("{}. {}\n   {}\n", i + 1, title, url));
            }
            if out.is_empty() {
                return Err(ProviderError::NoResults("searxng"));
            }
            Ok(Response::from_text(out))
        }
    }
}

/// Tiny URL-encoder for the `q` parameter. Avoids pulling in the
/// `urlencoding` crate for a single call site.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

// =====================================================================
// register_extras
// =====================================================================

/// Register the 7 additional backends on a registry. Call after
/// `register_builtin` so the original 4 stay registered too.
pub fn register_extras(reg: &Registry, client: &reqwest::Client) {
    reg.register(Arc::new(fal_imagegen::Fal::new(client.clone())));
    reg.register(Arc::new(replicate_imagegen::Replicate::new(client.clone())));
    reg.register(Arc::new(elevenlabs_tts::ElevenLabs::new(client.clone())));
    reg.register(Arc::new(fish_tts::Fish::new(client.clone())));
    reg.register(Arc::new(minimax_tts::MiniMax::new(client.clone())));
    reg.register(Arc::new(firecrawl_fetch::Firecrawl::new(client.clone())));
    reg.register(Arc::new(exa_search::Exa::new(client.clone())));
    reg.register(Arc::new(searxng_search::SearXNG::new(client.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> ProviderConfig {
        ProviderConfig::default()
    }

    #[test]
    fn register_extras_adds_all_seven() {
        // The extras add: 2 imagegen (Fal, Replicate), 3 tts
        // (ElevenLabs, Fish, MiniMax), 1 webfetch (Firecrawl),
        // 2 websearch (Exa, SearXNG). Calling only `register_extras`
        // means the builtins (OpenAI/Brave/Direct/Jina/None) are
        // NOT registered — we test the delta in isolation.
        let r = Registry::new();
        let client = reqwest::Client::new();
        register_extras(&r, &client);
        assert_eq!(r.for_category("image_gen").len(), 2);
        assert_eq!(r.for_category("tts").len(), 3);
        assert_eq!(r.for_category("web_fetch").len(), 1);
        assert_eq!(r.for_category("web_search").len(), 2);
    }

    #[test]
    fn str_field_extracts_text() {
        let v = json!({"prompt": "hello", "missing": null});
        assert_eq!(str_field(&v, "prompt"), "hello");
        assert_eq!(str_field(&v, "missing"), "");
        assert_eq!(str_field(&v, "nope"), "");
    }

    #[test]
    fn urlencode_handles_specials() {
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencode("safe-chars_~.are-ok"), "safe-chars_~.are-ok");
    }

    #[tokio::test]
    async fn missing_api_key_errors_cleanly() {
        let p = fal_imagegen::Fal::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({"prompt": "test"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::MissingApiKey(_))));
    }

    #[tokio::test]
    async fn missing_prompt_errors_invalid_args() {
        let p = fal_imagegen::Fal::new(reqwest::Client::new());
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
    async fn searxng_requires_endpoint() {
        let p = searxng_search::SearXNG::new(reqwest::Client::new());
        let r = p
            .execute(Request {
                args: json!({"query": "x"}),
                config: empty_config(),
            })
            .await;
        assert!(matches!(r, Err(ProviderError::InvalidArgs(_))));
    }
}
