//! ElevenLabs TTS.
//!
//! Posts to `/v1/text-to-speech/{voice_id}` with the API key in
//! the non-standard `xi-api-key` header. Default voice is
//! "Rachel" (long-standing built-in, available on every tier).
use async_trait::async_trait;
use serde_json::json;

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const DEFAULT_VOICE: &str = "21m00Tcm4TlvDq8ikWAM";
const DEFAULT_MODEL: &str = "eleven_multilingual_v2";
const CATEGORY: &str = "tts";

pub struct ElevenLabs {
    client: reqwest::Client,
}

impl ElevenLabs {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Provider for ElevenLabs {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "elevenlabs"
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let text = str_field(&req.args, "text");
        if text.is_empty() {
            return Err(ProviderError::InvalidArgs("tts: text required".into()));
        }
        if req.config.api_key.is_empty() {
            return Err(ProviderError::MissingApiKey("elevenlabs"));
        }
        let model = if req.config.model.is_empty() {
            DEFAULT_MODEL
        } else {
            req.config.model.as_str()
        };
        let voice = if req.config.endpoint.is_empty() {
            DEFAULT_VOICE
        } else {
            req.config.endpoint.as_str()
        };
        let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice}");
        let body = json!({
            "text": text,
            "model_id": model,
            "voice_settings": {"stability": 0.5, "similarity_boost": 0.75},
        });
        let resp = self
            .client
            .post(&url)
            .header("xi-api-key", &req.config.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "audio/mpeg")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!(
                "elevenlabs {status}: {txt}"
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        Ok(Response::from_text(format!(
            "[elevenlabs] generated {} bytes of mpeg (model={model} voice={voice})",
            bytes.len()
        )))
    }
}
