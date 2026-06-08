//! `install_skill` and `search_skills` tools — agent-initiated skill
//! install from the public registry.
//!
//! For
//! the first cut, install lands in the agent's local skills dir from
//! either (a) a local path (caller pre-staged it) or (b) a URL the
//! caller pre-resolved. The full ClawHub + skills.sh integration is a
//! follow-up phase.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

pub struct InstallSkillTool {
    pub skills_root: PathBuf,
    pub on_reload: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Deserialize)]
struct InstallArgs {
    name: String,
    /// Optional: local path to a skill folder to copy in.
    #[serde(default)]
    path: Option<String>,
    /// Optional: a URL the caller has already resolved to a tarball /
    /// directory; we don't fetch here.
    #[serde(default)]
    url: Option<String>,
}

#[async_trait]
impl Tool for InstallSkillTool {
    fn name(&self) -> &str {
        "install_skill"
    }
    fn description(&self) -> &str {
        "Install a skill into THIS agent's private skills directory. Provide `path` for a local skill folder, or `url` for an already-resolved source. After install the agent picks it up on the next turn."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Skill name/slug"},
                "path": {"type": "string", "description": "Optional local path to copy from"},
                "url": {"type": "string", "description": "Optional URL to fetch from"}
            },
            "required": ["name"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: InstallArgs = serde_json::from_value(args)?;
        if a.name.is_empty() {
            return Err(CleanClawError::InvalidArgument("name is required".into()));
        }
        let dest = self.skills_root.join(&a.name);
        if dest.exists() {
            return Err(CleanClawError::Conflict(format!(
                "skill {} already installed",
                a.name
            )));
        }
        std::fs::create_dir_all(&dest).map_err(|e| {
            CleanClawError::Internal(format!("create skills dir: {e}"))
        })?;

        if let Some(path) = a.path {
            copy_dir_recursive(&PathBuf::from(path), &dest)?;
        } else if let Some(url) = a.url {
            // For the first cut we require a local path; the URL
            // path can be filled in once the HTTP-fetcher is wired.
            return Err(CleanClawError::NotImplemented(format!(
                "install_skill(url=...) not yet supported in this build; use path=. Got url={url:?}"
            )));
        } else {
            return Err(CleanClawError::InvalidArgument(
                "install_skill: one of path or url is required".into(),
            ));
        }

        if let Some(cb) = &self.on_reload {
            cb();
        }
        Ok(json!({"installed": a.name, "path": dest.to_string_lossy()}))
    }
}

fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    if !src.is_dir() {
        return Err(CleanClawError::InvalidArgument(format!(
            "install_skill: {src:?} is not a directory"
        )));
    }
    std::fs::create_dir_all(dst).map_err(|e| CleanClawError::Internal(format!("mkdir: {e}")))?;
    for entry in std::fs::read_dir(src).map_err(|e| CleanClawError::Internal(format!("readdir: {e}")))? {
        let entry = entry.map_err(|e| CleanClawError::Internal(e.to_string()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| CleanClawError::Internal(format!("copy: {e}")))?;
        }
    }
    Ok(())
}

pub struct SearchSkillsTool;

#[async_trait]
impl Tool for SearchSkillsTool {
    fn name(&self) -> &str {
        "search_skills"
    }
    fn description(&self) -> &str {
        "Search the public skill registries. Stub in this build — pass a `path` to install_skill to add a local skill."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value> {
        Ok(json!({
            "results": [],
            "hint": "Public registry search isn't wired in this build. Use install_skill(path=...) to add a local skill."
        }))
    }
}
