//! OpenAI TTS backend.
//!
//! Hits `POST /v1/audio/speech` with bearer auth. The response is
//! raw MP3 bytes; we return a summary string (model + voice + byte
//! count) because the abstract `Response` only carries text. The
//! Go-side runtime is responsible for piping the bytes through
//! the IM channel's media-upload path.
use async_trait::async_trait;
use serde_json::json;

use super::{parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

/// OpenAI TTS.
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
        let (text, voice) = parse_args(&req.args)?;
        if req.config.api_key.is_empty() {
            return Err(ProviderError::MissingApiKey("openai-tts"));
        }
        let model = if req.config.model.is_empty() {
            "tts-1"
        } else {
            req.config.model.as_str()
        };
        let voice = if voice.is_empty() {
            "alloy"
        } else {
            voice.as_str()
        };
        let endpoint = if req.config.endpoint.is_empty() {
            "https://api.openai.com/v1/audio/speech"
        } else {
            req.config.endpoint.as_str()
        };
        let body = json!({
            "model": model,
            "input": text,
            "voice": voice,
            "response_format": "mp3",
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
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        // The Go side returns a workspace.Store Put path; for the
        // abstract provider we just hand the LLM a summary
        // (caller wraps it into a workspace artifact).
        Ok(Response::from_text(format!(
            "[tts] generated {} bytes of audio (model={model} voice={voice})",
            bytes.len()
        )))
    }
}
