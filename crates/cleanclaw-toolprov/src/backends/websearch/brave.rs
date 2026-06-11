//! Brave Search backend.
use async_trait::async_trait;

use super::{parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

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
