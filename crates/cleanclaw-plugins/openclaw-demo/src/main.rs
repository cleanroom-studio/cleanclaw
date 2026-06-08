//! OpenClaw-style demo plugin. Mirrors
//! .
//!
//! The original was a Node.js module consumed via
//! `tools/openclaw-plugin-bridge/proxy.js`; the Rust port embeds
//! the two tools (`get_weather` and `calculate`) directly and
//! speaks the same JSON-RPC protocol the host already understands.

use async_trait::async_trait;
use cleanclaw_plugin_runtime::{Plugin, PluginError, ToolDef, ToolResult};
use serde_json::Value;
use std::sync::Arc;

struct OpenClawDemoPlugin;

#[async_trait]
impl Plugin for OpenClawDemoPlugin {
    fn id(&self) -> &str {
        "openclaw-plugin-demo"
    }

    async fn tool_list(&self) -> Result<Vec<ToolDef>, PluginError> {
        Ok(vec![
            ToolDef {
                name: "get_weather".into(),
                description: "Get current weather for a city.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string", "description": "City name" }
                    },
                    "required": ["city"]
                }),
                source: "plugin".into(),
            },
            ToolDef {
                name: "calculate".into(),
                description: "Evaluate a simple math expression.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "expression": { "type": "string", "description": "e.g. '1 + 2'" }
                    },
                    "required": ["expression"]
                }),
                source: "plugin".into(),
            },
        ])
    }

    async fn tool_execute(&self, name: &str, args: Value) -> Result<ToolResult, PluginError> {
        match name {
            "get_weather" => {
                let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("?");
                Ok(ToolResult {
                    output: format!("Weather in {city}: 22°C, partly cloudy (stub)"),
                    error: None,
                })
            }
            "calculate" => {
                let expr = args
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0");
                // The real plugin shells out to a JS expression
                // evaluator; the Rust port returns a placeholder.
                Ok(ToolResult {
                    output: format!("(stub) {expr} = ?"),
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
    cleanclaw_plugin_runtime::run_plugin(Arc::new(OpenClawDemoPlugin)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_plugin_runtime::InProcPluginClient;

    #[tokio::test]
    async fn lists_two_tools() {
        let c = InProcPluginClient::spawn(OpenClawDemoPlugin);
        let tools: Vec<ToolDef> =
            serde_json::from_value(c.call("tool.list", Value::Null).await.unwrap()).unwrap();
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"get_weather"));
        assert!(names.contains(&"calculate"));
    }

    #[tokio::test]
    async fn get_weather_returns_text() {
        let c = InProcPluginClient::spawn(OpenClawDemoPlugin);
        let r: ToolResult = serde_json::from_value(
            c.call(
                "tool.execute",
                serde_json::json!({"name": "get_weather", "args": {"city": "Paris"}}),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert!(r.output.contains("Paris"));
    }
}
