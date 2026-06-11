//! Microsoft Bing Web Search v7 backend.
use async_trait::async_trait;

use super::{parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

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
