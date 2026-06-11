//! Replicate image generation.
//!
//! `POST /v1/models/<owner>/<name>/predictions` with
//! `Prefer: wait` for a synchronous response.
use async_trait::async_trait;
use serde_json::{json, Value};

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "image_gen";

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
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "replicate"
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let prompt = str_field(&req.args, "prompt");
        if prompt.is_empty() {
            return Err(ProviderError::InvalidArgs(
                "imagegen: prompt required".into(),
            ));
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
            return Err(ProviderError::Upstream(format!(
                "replicate {status}: {txt}"
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        let image_url = v
            .get("output")
            .and_then(|o| match o {
                Value::Array(a) => a.first().and_then(|x| x.as_str()),
                Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");
        Ok(Response::from_text(format!(
            "[replicate] {model} → {image_url}"
        )))
    }
}
