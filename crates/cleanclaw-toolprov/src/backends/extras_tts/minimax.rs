//! MiniMax T2A v2 TTS.
//!
//! `POST https://api.minimaxi.com/v1/t2a_v2` with bearer auth.
//! The body carries a `voice_setting` block; the response is
//! audio hex-encoded into `data.audio`.
use async_trait::async_trait;
use serde_json::{json, Value};

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "tts";

pub struct MiniMax {
    client: reqwest::Client,
}

impl MiniMax {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Provider for MiniMax {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "minimax"
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let text = str_field(&req.args, "text");
        if text.is_empty() {
            return Err(ProviderError::InvalidArgs("tts: text required".into()));
        }
        if req.config.api_key.is_empty() {
            return Err(ProviderError::MissingApiKey("minimax"));
        }
        let model = if req.config.model.is_empty() {
            "speech-02-hd"
        } else {
            req.config.model.as_str()
        };
        let voice_id = if req.config.endpoint.is_empty() {
            "male-qn-jingying"
        } else {
            req.config.endpoint.as_str()
        };
        let body = json!({
            "model": model,
            "text": text,
            "stream": false,
            "voice_setting": {
                "voice_id": voice_id,
                "speed": 1.0,
                "vol": 1.0,
                "pitch": 0,
            },
            "audio_setting": {
                "sample_rate": 32000,
                "bitrate": 128000,
                "format": "mp3",
            },
        });
        let resp = self
            .client
            .post("https://api.minimaxi.com/v1/t2a_v2")
            .bearer_auth(&req.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!("minimax {status}: {txt}")));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        let audio_bytes = v
            .get("data")
            .and_then(|d| d.get("audio"))
            .and_then(|a| a.as_str())
            .map(|hex| {
                // hex-decode (a→10) — short, hand-rolled
                (0..hex.len())
                    .step_by(2)
                    .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                    .collect::<Vec<u8>>()
                    .len()
            })
            .unwrap_or(0);
        Ok(Response::from_text(format!(
            "[minimax] generated {audio_bytes} bytes of mp3 (model={model} voice={voice_id})"
        )))
    }
}
