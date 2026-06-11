//! OpenAI image generation backend.
//!
//! Hits `POST /v1/images/generations` with bearer auth. The
//! response shape depends on the model:
//!
//!   * `dall-e-3` returns `data: [{ url }]`;
//!   * `gpt-image-1` returns `data: [{ b64_json }]`.
//!
//! We detect both and render whichever the upstream hands back.
use async_trait::async_trait;
use serde_json::json;

use super::{parse_args, render_b64, render_urls, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

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
        let body = json!({
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
