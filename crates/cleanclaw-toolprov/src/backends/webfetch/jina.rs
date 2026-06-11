//! Jina Reader backend.
//!
//! Hits `https://r.jina.ai/<url>` with bearer auth and asks for
//! markdown. Jina strips ads, navigation, and inline JS so the
//! result is much more model-friendly than a raw `direct` fetch.
//! Falls back to `direct` in the chain when the Jina key is not
//! configured.
use async_trait::async_trait;

use super::{parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

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
