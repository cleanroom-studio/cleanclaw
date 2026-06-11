//! Google Programmable Search Engine (Custom Search JSON API).
//!
//! The `cx` (search-engine id) is taken from the `endpoint`
//! config field (formatted as `cx=<id>`) so we don't have to
//! extend the `ProviderConfig` struct just for this.
use async_trait::async_trait;

use super::{parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

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
