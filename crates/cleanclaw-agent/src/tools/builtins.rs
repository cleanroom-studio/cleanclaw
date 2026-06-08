//! Built-in tools — kept lean for the first cut: `current_time`,
//! `echo`, `http_get`. Sandboxed `exec` / `read_file` / `write_file` land
//! in a later phase once the sandbox layer is wired.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use chrono::Utc;
use cleanclaw_core::{CleanClawError, Result};
use serde_json::{json, Value};

pub struct CurrentTime;

#[async_trait]
impl Tool for CurrentTime {
    fn name(&self) -> &str {
        "current_time"
    }
    fn description(&self) -> &str {
        "Return the current UTC time in RFC 3339 format."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
        })
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
        Ok(json!({ "now": Utc::now().to_rfc3339() }))
    }
}

pub struct Echo;

#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Return the input string verbatim. Useful for debugging."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CleanClawError::InvalidArgument("echo.text required".into()))?;
        Ok(json!({ "echo": text }))
    }
}

/// Minimal HTTP GET — returns status + first 4 KiB of body. Real
/// `web_fetch` (with retries, content extraction, image handling) lands
/// in the tool-providers crate.
pub struct HttpGet;

#[async_trait]
impl Tool for HttpGet {
    fn name(&self) -> &str {
        "http_get"
    }
    fn description(&self) -> &str {
        "Fetch a URL and return the first 4 KiB of the response body."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CleanClawError::InvalidArgument("http_get.url required".into()))?;
        let resp = reqwest::get(url)
            .await
            .map_err(|e| CleanClawError::Upstream(format!("http_get: {e}")))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| CleanClawError::Upstream(format!("http_get body: {e}")))?;
        let truncated = body.chars().take(4096).collect::<String>();
        Ok(json!({ "status": status, "body": truncated }))
    }
}

/// Register every built-in tool on the given registry.
pub fn register_builtins(reg: &mut super::ToolRegistry) {
    reg.register(Arc::new(CurrentTime));
    reg.register(Arc::new(Echo));
    reg.register(Arc::new(HttpGet));
    // File system
    reg.register(Arc::new(super::file::ReadFileTool));
    reg.register(Arc::new(super::file::WriteFileTool));
    reg.register(Arc::new(super::file::EditFileTool));
    reg.register(Arc::new(super::file::ListDirTool));
    // Shell
    reg.register(Arc::new(super::exec::ExecTool));
    // Web
    reg.register(Arc::new(super::web::WebFetchTool));
    reg.register(Arc::new(super::web::WebSearchTool));
    // Skills
    reg.register(Arc::new(super::load_skill::LoadSkillTool));
    reg.register(Arc::new(super::skill_install::SearchSkillsTool));
    // Media (stubs)
    reg.register(Arc::new(super::media::ImageGenTool));
    reg.register(Arc::new(super::media::TtsTool));
    // Patches — multi-file transactional updates
    reg.register(Arc::new(super::apply_patch::ApplyPatchTool));
}

use std::sync::Arc;
