//! `image_gen` tool. Routes through the `imagegen` provider chain
//! (OpenAI gpt-image-1, fal flux, Replicate, …).

use std::sync::Arc;

use async_trait::async_trait;
use cleanclaw_core::CleanClawError;
use cleanclaw_toolprov::imagegen;
use cleanclaw_toolprov::ProviderConfig;
use cleanclaw_toolprov::Request as ProviderRequest;
use serde_json::Value;

use super::{Tool, ToolContext};

pub struct ImageGenTool {
    pub provider: Arc<dyn cleanclaw_toolprov::Provider>,
    pub config: ProviderConfig,
}

impl ImageGenTool {
    pub fn new(provider: Arc<dyn cleanclaw_toolprov::Provider>, config: ProviderConfig) -> Self {
        Self { provider, config }
    }
}

#[async_trait]
impl Tool for ImageGenTool {
    fn name(&self) -> &str {
        "image_gen"
    }
    fn description(&self) -> &str {
        "Generate images from a text prompt. Uses a configurable provider chain (OpenAI gpt-image-1, fal flux, …) with automatic fallback. Returns markdown image tags that render inline in chat."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Description of the image to generate"},
                "size": {"type": "string", "description": "Image size (e.g. 1024x1024). Provider-specific."},
                "n": {"type": "integer", "description": "How many variations (default 1, max 4)"}
            },
            "required": ["prompt"]
        })
    }

    async fn call(
        &self,
        _ctx: &ToolContext,
        args: Value,
    ) -> std::result::Result<Value, CleanClawError> {
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return Ok(Value::String("image_gen: prompt is required".into())),
        };
        let _ = imagegen::CATEGORY; // touch the const
        let resp = self
            .provider
            .execute(ProviderRequest {
                args: args.clone(),
                config: self.config.clone(),
            })
            .await
            .map_err(|e| CleanClawError::Internal(e.to_string()))?;
        Ok(Value::String(resp.text))
    }
}

#[cfg(test)]
mod image_gen_tests {
    use super::*;
    use cleanclaw_toolprov::{Provider, Response};

    struct Stub;
    #[async_trait::async_trait]
    impl Provider for Stub {
        fn category(&self) -> &'static str {
            "image_gen"
        }
        fn name(&self) -> &'static str {
            "stub"
        }
        async fn execute(
            &self,
            _req: ProviderRequest,
        ) -> std::result::Result<Response, cleanclaw_toolprov::ProviderError> {
            Ok(Response::from_text("![out](data:image/png;base64,AAA)"))
        }
    }

    #[tokio::test]
    async fn image_gen_calls_provider() {
        let tool = ImageGenTool::new(Arc::new(Stub), ProviderConfig::default());
        let ctx = ToolContext::default();
        let r = tool
            .call(&ctx, serde_json::json!({"prompt": "a fluffy cat"}))
            .await
            .unwrap();
        let s = r.as_str().unwrap();
        assert!(s.contains("data:image/png"));
    }

    #[tokio::test]
    async fn image_gen_missing_prompt_errors() {
        let tool = ImageGenTool::new(Arc::new(Stub), ProviderConfig::default());
        let ctx = ToolContext::default();
        let r = tool.call(&ctx, serde_json::json!({})).await.unwrap();
        assert!(r.as_str().unwrap().contains("prompt is required"));
    }

    #[test]
    fn image_gen_definition_contains_prompt() {
        let tool = ImageGenTool::new(Arc::new(Stub), ProviderConfig::default());
        let d = tool.parameters();
        let req = d.get("required").unwrap();
        assert!(req
            .as_array()
            .unwrap()
            .contains(&Value::String("prompt".into())));
    }
}
