//! Fal image generation.
//!
//! Posts to `https://fal.run/<model-path>`. Auth is `Key <token>`.
//! The model path follows the convention
//! `fal-ai/<model>[/<variant>]`. Returns the first image URL
//! from the response.
use async_trait::async_trait;
use serde_json::json;

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "image_gen";

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
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "fal"
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let prompt = str_field(&req.args, "prompt");
        if prompt.is_empty() {
            return Err(ProviderError::InvalidArgs(
                "imagegen: prompt required".into(),
            ));
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
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
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
