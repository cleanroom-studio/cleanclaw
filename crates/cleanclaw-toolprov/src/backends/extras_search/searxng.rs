//! SearXNG web search.
//!
//! `<endpoint>/search?q=<query>&format=json`. No auth.
//! `endpoint` is required (the operator must self-host).
use async_trait::async_trait;
use serde_json::Value;

use super::{str_field, urlencode};
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "web_search";

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
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "searxng"
    }
    // Self-hosted: the operator must supply the instance URL
    // via the `endpoint` config field. The chain will silently
    // skip this provider when the field is empty so we don't
    // surface a misleading "endpoint required" error from the
    // *last* provider when the user just hasn't configured
    // it.
    fn needs_endpoint(&self) -> bool {
        true
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let q = str_field(&req.args, "query");
        if q.is_empty() {
            return Err(ProviderError::InvalidArgs(
                "websearch: query required".into(),
            ));
        }
        let endpoint = if req.config.endpoint.is_empty() {
            return Err(ProviderError::InvalidArgs(
                "searxng: endpoint required in config".into(),
            ));
        } else {
            req.config.endpoint.trim_end_matches('/')
        };
        let url = format!(
            "{}/search?q={}&format=json&categories=general",
            endpoint,
            urlencode(q)
        );
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
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        let results = v
            .get("results")
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
            return Err(ProviderError::NoResults("searxng"));
        }
        Ok(Response::from_text(out))
    }
}
