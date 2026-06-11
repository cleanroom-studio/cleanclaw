//! Firecrawl URL fetch.
//!
//! `POST https://api.firecrawl.dev/v1/scrape` returns the page
//! as cleaned markdown. Auth is `Authorization: Bearer`.
use async_trait::async_trait;
use serde_json::{json, Value};

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "web_fetch";

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
        CATEGORY
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
            return Err(ProviderError::Upstream(format!(
                "firecrawl {status}: {txt}"
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
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
