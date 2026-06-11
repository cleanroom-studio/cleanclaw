//! Fish Audio TTS.
//!
//! `POST https://api.fish.audio/v1/tts` with bearer auth. The
//! body is `{text, reference_id?, format}`; the response is
//! audio bytes. Default format is `mp3`.
use async_trait::async_trait;
use serde_json::json;

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "tts";

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
        CATEGORY
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
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        Ok(Response::from_text(format!(
            "[fish] generated {} bytes of mp3",
            bytes.len()
        )))
    }
}
