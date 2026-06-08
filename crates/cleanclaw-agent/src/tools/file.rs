//! File system tools: read_file, write_file, edit_file, list_dir.
//!
//! Routes paths
//! to one of three roots based on the path shape:
//!   - `systemRoot` for identity files (SOUL.md, IDENTITY.md, …)
//!   - `userRoot` (the workspace) for ordinary user artifacts
//!   - `userSkillsRoot` for `skills/<name>/...` chat-time skill writes
//!
//! Identity files are gated behind `caller_is_admin` so non-owners can't
//! read or modify the agent's SOUL.md / IDENTITY.md via the chat surface.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

/// Files that the agent treats as "system" (identity / config). They
/// route through `systemRoot` and are gated on `caller_is_admin`.
pub const SYSTEM_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "BOOTSTRAP.md",
    "MEMORY.md",
    "HEARTBEAT.md",
    "AGENTS.md",
    "TOOLS.md",
    "agent.json",
];

pub fn is_system_file(path: &str) -> bool {
    let clean = Path::new(path);
    // Only single-segment paths are treated as system files. Nested
    // paths like "notes/SOUL.md" stay in the user root.
    let mut comps = clean.components();
    if let (Some(Component::Normal(first)), None) = (comps.next(), comps.next()) {
        let name = first.to_string_lossy();
        return SYSTEM_FILES.iter().any(|f| *f == name);
    }
    false
}

pub fn is_skill_path(path: &str) -> bool {
    if Path::new(path).is_absolute() {
        return false;
    }
    let clean = path.replace('\\', "/");
    clean != "skills" && clean.starts_with("skills/")
}

pub fn root_for_path(
    path: &str,
    system_root: &str,
    user_root: &str,
    user_skills_root: &str,
) -> Root {
    if Path::new(path).is_absolute() {
        return Root::Absolute;
    }
    let clean = path.replace('\\', "/");
    if is_skill_path(&clean) {
        if !user_skills_root.is_empty() {
            return Root::System(user_skills_root.to_string());
        }
        return Root::System(system_root.to_string());
    }
    if is_system_file(&clean) {
        return Root::System(system_root.to_string());
    }
    Root::System(user_root.to_string())
}

#[derive(Debug, Clone)]
pub enum Root {
    Absolute,
    System(String),
}

impl Root {
    pub fn resolve(&self, path: &str) -> PathBuf {
        match self {
            Root::Absolute => PathBuf::from(path),
            Root::System(root) => {
                let clean = path.trim_start_matches('/');
                PathBuf::from(root).join(clean)
            }
        }
    }
}

// ---- shared arg validation ----------------------------------------------

fn validate_file_target_path(path: &str) -> Result<()> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(CleanClawError::InvalidArgument(
            "path is required and must include a filename".into(),
        ));
    }
    if trimmed.ends_with('/') || trimmed.ends_with(std::path::MAIN_SEPARATOR) {
        return Err(CleanClawError::InvalidArgument(format!(
            "path {path:?} ends in a separator; include a filename at the end"
        )));
    }
    let clean = Path::new(trimmed);
    if let Ok(c) = clean.canonicalize() {
        if c.is_dir() {
            return Err(CleanClawError::InvalidArgument(format!(
                "path {path:?} is a directory, not a file; include a filename"
            )));
        }
    }
    match clean.components().count() {
        1 if matches!(
            clean.components().next(),
            Some(Component::CurDir) | Some(Component::ParentDir) | Some(Component::RootDir)
        ) =>
        {
            Err(CleanClawError::InvalidArgument(format!(
                "path {path:?} is a directory, not a file; include a filename"
            )))
        }
        _ => Ok(()),
    }
}

// ---- read_file -----------------------------------------------------------

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a file. Use the optional `limit` / `start_line` to read a slice."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "start_line": {"type": "integer", "description": "0-based start line (default 0)"},
                "limit": {"type": "integer", "description": "Max lines to return (default 200)"}
            },
            "required": ["path"]
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: ReadFileArgs = serde_json::from_value(args)?;
        let p = a.path.trim();
        validate_file_target_path(p)?;
        let path = resolve(ctx, p)?;
        if !path.exists() {
            return Err(CleanClawError::NotFound(format!("read_file: {p}")));
        }
        if path.is_dir() {
            return Err(CleanClawError::InvalidArgument(format!(
                "read_file: {p:?} is a directory; use list_dir"
            )));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| CleanClawError::Internal(format!("read_file {p}: {e}")))?;
        let lines: Vec<&str> = content.lines().collect();
        let start = a.start_line.unwrap_or(0);
        let limit = a.limit.unwrap_or(200);
        let slice: Vec<&str> = lines.iter().skip(start).take(limit).copied().collect();
        Ok(json!({
            "path": p,
            "content": slice.join("\n"),
            "total_lines": lines.len(),
            "truncated": lines.len() > start + limit,
        }))
    }
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    start_line: Option<usize>,
    limit: Option<usize>,
}

// ---- write_file ----------------------------------------------------------

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file (creates parent directories as needed). For partial edits to existing files, prefer edit_file — it's cheaper, can't drop unrelated content, and validates the replacement was applied."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: WriteFileArgs = serde_json::from_value(args)?;
        let p = a.path.trim();
        validate_file_target_path(p)?;
        let path = resolve(ctx, p)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CleanClawError::Internal(format!("create_dir_all {parent:?}: {e}")))?;
        }
        std::fs::write(&path, a.content.as_bytes())
            .map_err(|e| CleanClawError::Internal(format!("write_file {p}: {e}")))?;
        Ok(json!({"path": p, "wrote_bytes": a.content.len()}))
    }
}

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

// ---- edit_file -----------------------------------------------------------

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Edit a file by replacing an exact substring. old_string must match a unique substring unless replace_all is true; new_string must differ. Read the file first if unsure of the exact text."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_string": {"type": "string"},
                "new_string": {"type": "string"},
                "replace_all": {"type": "boolean", "default": false}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: EditFileArgs = serde_json::from_value(args)?;
        let p = a.path.trim();
        validate_file_target_path(p)?;
        let path = resolve(ctx, p)?;
        if !path.exists() {
            return Err(CleanClawError::NotFound(format!("edit_file: {p}")));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| CleanClawError::Internal(format!("edit_file {p}: {e}")))?;
        let (new_content, count) =
            apply_edit(p, &content, &a.old_string, &a.new_string, a.replace_all)?;
        std::fs::write(&path, new_content.as_bytes())
            .map_err(|e| CleanClawError::Internal(format!("edit_file {p}: {e}")))?;
        Ok(json!({"path": p, "replacements": count}))
    }
}

#[derive(Deserialize)]
struct EditFileArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

fn apply_edit(
    path: &str,
    content: &str,
    old_str: &str,
    new_str: &str,
    replace_all: bool,
) -> Result<(String, usize)> {
    if old_str.is_empty() {
        return Err(CleanClawError::InvalidArgument(
            "edit_file: old_string is empty (use write_file to create a file)".into(),
        ));
    }
    if old_str == new_str {
        return Err(CleanClawError::InvalidArgument(
            "edit_file: new_string must differ from old_string".into(),
        ));
    }
    let count = content.matches(old_str).count();
    if count == 0 {
        return Err(CleanClawError::InvalidArgument(format!(
            "edit_file: old_string not found in {path} — re-read the file and copy the exact text (whitespace/indentation matters)"
        )));
    }
    if count > 1 && !replace_all {
        return Err(CleanClawError::InvalidArgument(format!(
            "edit_file: old_string matches {count} locations in {path} — provide more surrounding context to make it unique, or set replace_all=true"
        )));
    }
    let new_content = if replace_all {
        content.replace(old_str, new_str)
    } else {
        content.replacen(old_str, new_str, 1)
    };
    Ok((new_content, if replace_all { count } else { 1 }))
}

// ---- list_dir ------------------------------------------------------------

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        "List the files and directories under a path. Defaults to the working directory."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory path; empty = working directory"}
            }
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: ListDirArgs = serde_json::from_value(args)?;
        let p = a.path.as_deref().unwrap_or(".").trim();
        let path = resolve(ctx, p)?;
        if !path.exists() {
            return Err(CleanClawError::NotFound(format!("list_dir: {p}")));
        }
        if !path.is_dir() {
            return Err(CleanClawError::InvalidArgument(format!(
                "list_dir: {p:?} is not a directory"
            )));
        }
        let mut entries: Vec<Value> = Vec::new();
        for entry in std::fs::read_dir(&path)
            .map_err(|e| CleanClawError::Internal(format!("list_dir {p:?}: {e}")))?
        {
            let entry = entry.map_err(|e| CleanClawError::Internal(e.to_string()))?;
            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            entries.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "is_dir": is_dir,
                "size": size,
            }));
        }
        entries.sort_by(|a, b| {
            let ad = a.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
            let bd = b.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
            bd.cmp(&ad).then(
                a.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or("")),
            )
        });
        Ok(json!({"path": p, "entries": entries}))
    }
}

#[derive(Deserialize)]
struct ListDirArgs {
    path: Option<String>,
}

// ---- path resolution -----------------------------------------------------

/// Decide where a path lands for the given tool context, then return
/// the absolute path to use for filesystem IO.
//
/// Identity files are gated on `caller_is_admin` (set by the agent
/// loop from the resolved chatter's role). When the caller is not
/// admin, attempts to read or write identity files surface a polite
/// refusal — the model sees the message, the chatter never gets the
/// agent's SOUL.md / IDENTITY.md.
fn resolve(ctx: &ToolContext, path: &str) -> Result<PathBuf> {
    if is_system_file(path) && !ctx.is_admin {
        return Err(CleanClawError::Forbidden);
    }
    let user_skills_root = ctx.workspace_root.clone();
    let system_root = ctx.workspace_root.clone();
    let user_root = ctx.workspace_root.clone();
    let resolved = root_for_path(path, &system_root, &user_root, &user_skills_root).resolve(path);
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx(admin: bool) -> ToolContext {
        ToolContext {
            is_admin: admin,
            workspace_root: "/tmp/cwtest".into(),
            extra: Arc::new(HashMap::new()),
            ..Default::default()
        }
    }

    use std::sync::Arc;

    #[test]
    fn detects_system_files() {
        assert!(is_system_file("SOUL.md"));
        assert!(is_system_file("IDENTITY.md"));
        assert!(is_system_file("MEMORY.md"));
        assert!(!is_system_file("notes/SOUL.md"));
        assert!(!is_system_file("report.md"));
    }

    #[test]
    fn detects_skill_paths() {
        assert!(is_skill_path("skills/foo/SKILL.md"));
        assert!(is_skill_path("skills/code-runner/scripts/run.py"));
        assert!(!is_skill_path("skills"));
        assert!(!is_skill_path("/abs/skills/x"));
    }

    #[test]
    fn root_for_routes_correctly() {
        let r = root_for_path("SOUL.md", "/sys", "/user", "/user_skills");
        match r {
            Root::System(s) => assert_eq!(s, "/sys"),
            _ => panic!(),
        }
        let r = root_for_path("skills/foo/SKILL.md", "/sys", "/user", "/user_skills");
        match r {
            Root::System(s) => assert_eq!(s, "/user_skills"),
            _ => panic!(),
        }
        let r = root_for_path("report.md", "/sys", "/user", "/user_skills");
        match r {
            Root::System(s) => assert_eq!(s, "/user"),
            _ => panic!(),
        }
    }

    #[test]
    fn system_file_blocked_for_non_admin() {
        let c = ctx(false);
        let r = resolve(&c, "SOUL.md");
        assert!(matches!(r, Err(CleanClawError::Forbidden)));
    }

    #[test]
    fn system_file_allowed_for_admin() {
        let c = ctx(true);
        let r = resolve(&c, "SOUL.md");
        assert!(r.is_ok());
    }

    #[test]
    fn non_system_file_allowed_for_everyone() {
        let c = ctx(false);
        let r = resolve(&c, "report.md");
        assert!(r.is_ok());
    }

    #[test]
    fn apply_edit_uniqueness() {
        let content = "foo foo foo";
        let res = apply_edit("f", content, "foo", "bar", false);
        assert!(res.is_err());
        let res = apply_edit("f", content, "foo", "bar", true).unwrap();
        assert_eq!(res.0, "bar bar bar");
        assert_eq!(res.1, 3);
    }

    #[test]
    fn apply_edit_not_found() {
        let res = apply_edit("f", "hello", "world", "earth", false);
        assert!(res.is_err());
    }

    #[test]
    fn apply_edit_empty_old() {
        let res = apply_edit("f", "hello", "", "world", false);
        assert!(res.is_err());
    }

    #[test]
    fn apply_edit_same_strings() {
        let res = apply_edit("f", "hello", "hello", "hello", false);
        assert!(res.is_err());
    }
}
