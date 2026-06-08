//! Echo plugin for CleanClaw. Mirrors
//! .
//!
//! Minimal example demonstrating the JSON-RPC plugin protocol.
//! Provides a single `echo` tool that returns whatever text is sent.

use async_trait::async_trait;
use cleanclaw_plugin_runtime::{
    args_object, run_plugin, Plugin, PluginError, ToolDef, ToolResult,
};
use serde_json::Value;
use std::sync::Arc;

struct EchoPlugin;

#[async_trait]
impl Plugin for EchoPlugin {
    fn id(&self) -> &str {
        "cleanclaw-plugin-demo"
    }

    async fn tool_list(&self) -> Result<Vec<ToolDef>, PluginError> {
        Ok(vec![ToolDef {
            name: "echo".into(),
            description: "Return whatever text is sent.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to echo back." }
                },
                "required": ["text"]
            }),
            source: "plugin".into(),
        }])
    }

    async fn tool_execute(
        &self,
        name: &str,
        args: Value,
    ) -> Result<ToolResult, PluginError> {
        match name {
            "echo" => {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ToolResult {
                    output: text,
                    error: None,
                })
            }
            other => Ok(ToolResult {
                output: String::new(),
                error: Some(format!("unknown tool: {other}")),
            }),
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    run_plugin(Arc::new(EchoPlugin)).await
}

// Re-export for use in tests.
pub fn _args_object() -> Value {
    args_object(&[("text".to_string(), Value::String("hi".into()))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_plugin_runtime::InProcPluginClient;

    #[tokio::test]
    async fn inproc_list_and_execute() {
        let c = InProcPluginClient::spawn(EchoPlugin);
        let tools: Vec<ToolDef> =
            serde_json::from_value(c.call("tool.list", Value::Null).await.unwrap()).unwrap();
        assert_eq!(tools[0].name, "echo");
        let r: ToolResult = serde_json::from_value(
            c.call(
                "tool.execute",
                serde_json::json!({"name": "echo", "args": {"text": "hello"}}),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(r.output, "hello");
        assert!(r.error.is_none());
    }

    #[tokio::test]
    async fn initialize_returns_ok() {
        let c = InProcPluginClient::spawn(EchoPlugin);
        let v = c.call("initialize", Value::Null).await.unwrap();
        assert_eq!(v["status"], "ok");
    }
}
