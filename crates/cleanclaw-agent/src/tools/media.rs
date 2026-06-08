//! `image_gen` and `tts` tools — multimodal generation.
//!
//! and
//! . These are placeholders
//! that surface a clear "no provider configured" error — the full
//! tool-provider chain implementation lands in a follow-up phase.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct ImageGenTool;

#[allow(dead_code)]
#[derive(Deserialize)]
struct ImageArgs {
    prompt: String,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[async_trait]
impl Tool for ImageGenTool {
    fn name(&self) -> &str {
        "image_gen"
    }
    fn description(&self) -> &str {
        "Generate an image from a text prompt. The file is saved to `output` (default: workspace/gen-<ts>.png)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "output": {"type": "string"},
                "width": {"type": "integer"},
                "height": {"type": "integer"}
            },
            "required": ["prompt"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
        Err(CleanClawError::NotImplemented(
            "image_gen: no provider configured. Add one in the dashboard's Tools page.".into(),
        ))
    }
}

pub struct TtsTool;

#[allow(dead_code)]
#[derive(Deserialize)]
struct TtsArgs {
    text: String,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    voice: Option<String>,
}

#[async_trait]
impl Tool for TtsTool {
    fn name(&self) -> &str {
        "tts"
    }
    fn description(&self) -> &str {
        "Synthesize speech from text. The file is saved to `output` (default: workspace/tts-<ts>.mp3)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "output": {"type": "string"},
                "voice": {"type": "string"}
            },
            "required": ["text"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
        Err(CleanClawError::NotImplemented(
            "tts: no provider configured.".into(),
        ))
    }
}
