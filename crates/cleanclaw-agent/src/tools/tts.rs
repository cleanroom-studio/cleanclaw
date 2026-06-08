//! `tts` tool. Routes through the `tts` provider chain (OpenAI
//! TTS, ElevenLabs, Edge, …). Mirrors
//! .

use std::sync::Arc;

use async_trait::async_trait;
use cleanclaw_core::CleanClawError;
use cleanclaw_toolprov::tts;
use cleanclaw_toolprov::ProviderConfig;
use cleanclaw_toolprov::Request as ProviderRequest;
use serde_json::Value;

use super::{Tool, ToolContext};

pub struct TtsTool {
    pub provider: Arc<dyn cleanclaw_toolprov::Provider>,
    pub config: ProviderConfig,
}

impl TtsTool {
    pub fn new(
        provider: Arc<dyn cleanclaw_toolprov::Provider>,
        config: ProviderConfig,
    ) -> Self {
        Self { provider, config }
    }
}

#[async_trait]
impl Tool for TtsTool {
    fn name(&self) -> &str {
        "tts"
    }
    fn description(&self) -> &str {
        "Synthesize speech from text. Provider chain (OpenAI TTS, ElevenLabs, Edge, …)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to synthesize."},
                "voice": {"type": "string", "description": "Voice id (provider-specific)."}
            },
            "required": ["text"]
        })
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> std::result::Result<Value, CleanClawError> {
        let text = match args.get("text").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return Ok(Value::String("tts: text is required".into())),
        };
        let _ = tts::CATEGORY; // touch the const
        let resp = self
            .provider
            .execute(ProviderRequest {
                args: args.clone(),
                config: self.config.clone(),
            })
            .await
            .map_err(|e| CleanClawError::Internal(e.to_string()))?;
        Ok(Value::String(format!(
            "[tts] synthesized {} chars: {}",
            text.len(),
            resp.text
        )))
    }
}

#[cfg(test)]
mod tts_tests {
    use super::*;
    use cleanclaw_toolprov::{Provider, Response};

    struct Stub;
    #[async_trait::async_trait]
    impl Provider for Stub {
        fn category(&self) -> &'static str { "tts" }
        fn name(&self) -> &'static str { "stub" }
        async fn execute(
            &self,
            _req: ProviderRequest,
        ) -> std::result::Result<Response, cleanclaw_toolprov::ProviderError> {
            Ok(Response::from_text("ok"))
        }
    }

    #[tokio::test]
    async fn tts_returns_summary() {
        let tool = TtsTool::new(Arc::new(Stub), ProviderConfig::default());
        let ctx = ToolContext::default();
        let r = tool
            .call(&ctx, serde_json::json!({"text": "hello world"}))
            .await
            .unwrap();
        let s = r.as_str().unwrap();
        assert!(s.contains("[tts]"));
        assert!(s.contains("11")); // "hello world".len()
    }

    #[tokio::test]
    async fn tts_missing_text_errors() {
        let tool = TtsTool::new(Arc::new(Stub), ProviderConfig::default());
        let ctx = ToolContext::default();
        let r = tool.call(&ctx, serde_json::json!({})).await.unwrap();
        assert!(r.as_str().unwrap().contains("text is required"));
    }

    #[test]
    fn tts_definition_required() {
        let tool = TtsTool::new(Arc::new(Stub), ProviderConfig::default());
        let d = tool.parameters();
        let req = d.get("required").unwrap();
        assert!(req.as_array().unwrap().contains(&Value::String("text".into())));
    }
}
