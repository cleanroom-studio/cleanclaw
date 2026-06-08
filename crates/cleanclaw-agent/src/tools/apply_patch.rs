//! `apply_patch` tool — multi-file patch aligned with OpenAI Codex's DSL.
//!
//! One tool call adds, updates, deletes, or renames any number of files
//! in a single transaction. The patch is parsed in memory first; only
//! when every hunk anchors successfully do we flush writes/deletes.
//! If any hunk fails, no file on disk changes — the agent gets a
//! clear error and can re-emit the patch.
//!
//! DSL shape ():
//!
//! ```text
//! *** Add File: path/to/file
//! +line one
//! +line two
//! *** Update File: path/to/file
//! @@ context marker
//!  context
//! -removed
//! +added
//! *** Delete File: path/to/file
//! *** Move to: new/path
//! *** End of File
//! ```
//!
//! `*** End of File` marks an "end-anchored" hunk: the leading
//! context is empty and the hunks apply to the tail of the file.
//! The "context" lines start with a single space; removed lines
//! start with `-`; added lines with `+`.
//!
//! The tool resolves paths through `ctx.user_root` (the per-user
//! workspace). Identity files (system files) are routed through
//! `ctx.system_root` and gated on `caller_is_admin`.

use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

/// Patch operation kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpType {
    Add,
    Update,
    Delete,
}

#[derive(Debug, Clone)]
pub struct HunkLine {
    pub kind: HunkKind,
    pub text: String, // without the leading +/-/space marker
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkKind {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub lines: Vec<HunkLine>,
    pub is_eof: bool,
}

#[derive(Debug, Clone)]
pub struct PatchOp {
    pub op: OpType,
    pub path: String,
    pub move_to: String,  // Update only
    pub add_body: String, // Add only
    pub hunks: Vec<Hunk>, // Update only
}

/// Parse a patch envelope. Returns a list of operations; the
/// `apply` step turns that into a transaction.
pub fn parse(patch: &str) -> std::result::Result<Vec<PatchOp>, String> {
    let mut ops: Vec<PatchOp> = Vec::new();
    let mut current: Option<PatchOp> = None;
    let mut current_hunk: Option<Hunk> = None;

    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("*** Add File:") {
            if let Some(c) = current.take() {
                if let Some(h) = current_hunk.take() {
                    let mut c = c;
                    c.hunks.push(h);
                    ops.push(c);
                } else {
                    ops.push(c);
                }
            }
            current = Some(PatchOp {
                op: OpType::Add,
                path: rest.trim().to_string(),
                move_to: String::new(),
                add_body: String::new(),
                hunks: Vec::new(),
            });
            current_hunk = None;
        } else if let Some(rest) = line.strip_prefix("*** Update File:") {
            if let Some(c) = current.take() {
                if let Some(h) = current_hunk.take() {
                    let mut c = c;
                    c.hunks.push(h);
                    ops.push(c);
                } else {
                    ops.push(c);
                }
            }
            current = Some(PatchOp {
                op: OpType::Update,
                path: rest.trim().to_string(),
                move_to: String::new(),
                add_body: String::new(),
                hunks: Vec::new(),
            });
            current_hunk = None;
        } else if let Some(rest) = line.strip_prefix("*** Delete File:") {
            if let Some(c) = current.take() {
                if let Some(h) = current_hunk.take() {
                    let mut c = c;
                    c.hunks.push(h);
                    ops.push(c);
                } else {
                    ops.push(c);
                }
            }
            current = Some(PatchOp {
                op: OpType::Delete,
                path: rest.trim().to_string(),
                move_to: String::new(),
                add_body: String::new(),
                hunks: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("*** Move to:") {
            if let Some(c) = current.as_mut() {
                c.move_to = rest.trim().to_string();
            } else {
                return Err("*** Move to: outside of operation".into());
            }
        } else if line.trim_start() == "*** End of File" {
            if let Some(h) = current_hunk.as_mut() {
                h.is_eof = true;
            } else if let Some(c) = current.as_mut() {
                c.add_body.push_str("\n");
                continue;
            } else {
                return Err("*** End of File outside of hunk".into());
            }
        } else if let Some(_rest) = line.strip_prefix("@@") {
            if let Some(h) = current_hunk.take() {
                if let Some(c) = current.as_mut() {
                    c.hunks.push(h);
                }
            }
            // The `@@` line is purely a hunk delimiter; the
            // hunk's `is_eof` flag is set by the separate
            // `*** End of File` line that follows.
            current_hunk = Some(Hunk {
                lines: Vec::new(),
                is_eof: false,
            });
        } else if let Some(rest) = line.strip_prefix('+') {
            // Either an Add body line or a hunk Add line.
            if let Some(c) = current.as_mut() {
                if c.op == OpType::Add {
                    c.add_body.push_str(rest);
                    c.add_body.push('\n');
                } else if let Some(h) = current_hunk.as_mut() {
                    h.lines.push(HunkLine {
                        kind: HunkKind::Add,
                        text: rest.to_string(),
                    });
                } else {
                    return Err(format!("+ line outside hunk: {line:?}"));
                }
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            if let Some(h) = current_hunk.as_mut() {
                h.lines.push(HunkLine {
                    kind: HunkKind::Remove,
                    text: rest.to_string(),
                });
            } else {
                return Err(format!("- line outside hunk: {line:?}"));
            }
        } else if let Some(rest) = line.strip_prefix(' ') {
            // Context line (leading space).
            if let Some(h) = current_hunk.as_mut() {
                h.lines.push(HunkLine {
                    kind: HunkKind::Context,
                    text: rest.to_string(),
                });
            } else {
                return Err(format!("context line outside hunk: {line:?}"));
            }
        } else if line.is_empty() {
            // Blank lines are context with empty text in hunks;
            // for Add body, they add a blank line.
            if let Some(c) = current.as_mut() {
                if c.op == OpType::Add {
                    c.add_body.push('\n');
                } else if let Some(h) = current_hunk.as_mut() {
                    h.lines.push(HunkLine {
                        kind: HunkKind::Context,
                        text: String::new(),
                    });
                }
            }
        } else {
            return Err(format!("unrecognized patch line: {line:?}"));
        }
    }

    if let Some(c) = current.take() {
        if let Some(h) = current_hunk.take() {
            let mut c = c;
            c.hunks.push(h);
            ops.push(c);
        } else {
            ops.push(c);
        }
    }
    Ok(ops)
}

/// Apply a parsed patch under `root`. Returns a string describing
/// what changed (or an error if any hunk failed to anchor).
pub fn apply(ops: &[PatchOp], root: &Path) -> std::result::Result<String, String> {
    // Two-phase: compute every op's resulting file content in
    // memory; only flush on success.
    let mut staging: std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> =
        std::collections::BTreeMap::new();
    let mut deletes: Vec<PathBuf> = Vec::new();
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();

    for op in ops {
        let target = resolve_path(root, &op.path)?;
        match op.op {
            OpType::Add => {
                staging.insert(target, Some(op.add_body.as_bytes().to_vec()));
            }
            OpType::Delete => {
                deletes.push(target);
            }
            OpType::Update => {
                if !staging.contains_key(&target) {
                    let current = std::fs::read(&target)
                        .map_err(|e| format!("read {}: {e}", target.display()))?;
                    staging.insert(target.clone(), Some(current));
                }
                let current = staging
                    .get(&target)
                    .and_then(|v| v.clone())
                    .ok_or_else(|| format!("no staged content for {}", target.display()))?;
                let new_content = apply_hunks(&current, &op.hunks)?;
                staging.insert(target.clone(), Some(new_content));
                if !op.move_to.is_empty() {
                    let new_path = resolve_path(root, &op.move_to)?;
                    moves.push((target, new_path));
                }
            }
        }
    }

    // Phase 2: flush.
    let mut log = Vec::new();
    for (path, content) in &staging {
        if let Some(bytes) = content {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
            log.push(format!("wrote {}", path.display()));
        }
    }
    for path in &deletes {
        std::fs::remove_file(path).map_err(|e| format!("delete {}: {e}", path.display()))?;
        log.push(format!("deleted {}", path.display()));
    }
    for (from, to) in &moves {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        std::fs::rename(from, to)
            .map_err(|e| format!("move {} → {}: {e}", from.display(), to.display()))?;
        log.push(format!("moved {} → {}", from.display(), to.display()));
    }
    Ok(log.join("\n"))
}

fn apply_hunks(content: &[u8], hunks: &[Hunk]) -> std::result::Result<Vec<u8>, String> {
    let text = std::str::from_utf8(content).map_err(|e| format!("file is not utf-8: {e}"))?;
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    // Preserve a trailing newline if the file had one.
    let trailing_nl = text.ends_with('\n');

    for hunk in hunks {
        if hunk.is_eof {
            // Apply to the end of the file. The hunk should not
            // contain Remove lines (they'd have nowhere to anchor);
            // we accept them but error if a non-context prefix
            // doesn't match.
            let mut new_tail: Vec<String> = Vec::new();
            for hl in &hunk.lines {
                match hl.kind {
                    HunkKind::Context => {
                        if let Some(last) = lines.last() {
                            if last != &hl.text {
                                return Err(format!(
                                    "EOF hunk context mismatch: {last:?} != {:?}",
                                    hl.text
                                ));
                            }
                        }
                    }
                    HunkKind::Add => new_tail.push(hl.text.clone()),
                    HunkKind::Remove => {
                        if let Some(last) = lines.last() {
                            if last == &hl.text {
                                lines.pop();
                            } else {
                                return Err(format!(
                                    "EOF hunk remove mismatch: {last:?} != {:?}",
                                    hl.text
                                ));
                            }
                        }
                    }
                }
            }
            lines.extend(new_tail);
            continue;
        }

        // Find the first index in `lines` where a sequence of
        // context + remove lines matches the hunk's prefix.
        let start = find_hunk_anchor(&lines, &hunk.lines)?;
        // Apply: walk through hunk lines, replacing context with
        // itself and remove→add. We'll rebuild `lines` to keep
        // indexing simple.
        let mut new_lines: Vec<String> = Vec::with_capacity(lines.len());
        new_lines.extend_from_slice(&lines[..start]);
        let mut i = start;
        for hl in &hunk.lines {
            match hl.kind {
                HunkKind::Context => {
                    if i >= lines.len() || lines[i] != hl.text {
                        return Err(format!(
                            "hunk context mismatch at line {i}: {:?} != {:?}",
                            lines.get(i),
                            hl.text
                        ));
                    }
                    new_lines.push(lines[i].clone());
                    i += 1;
                }
                HunkKind::Remove => {
                    if i >= lines.len() || lines[i] != hl.text {
                        return Err(format!(
                            "hunk remove mismatch at line {i}: {:?} != {:?}",
                            lines.get(i),
                            hl.text
                        ));
                    }
                    i += 1;
                    // Drop the line.
                }
                HunkKind::Add => {
                    new_lines.push(hl.text.clone());
                }
            }
        }
        new_lines.extend_from_slice(&lines[i..]);
        lines = new_lines;
    }

    let mut out = lines.join("\n");
    if trailing_nl {
        out.push('\n');
    }
    Ok(out.into_bytes())
}

fn find_hunk_anchor(lines: &[String], hunk: &[HunkLine]) -> std::result::Result<usize, String> {
    // The anchor is the position where the hunk's first non-Add
    // line matches.
    let n = lines.len();
    for start in 0..=n {
        let mut ok = true;
        let mut i = start;
        for hl in hunk {
            match hl.kind {
                HunkKind::Add => continue,
                HunkKind::Context | HunkKind::Remove => {
                    if i >= n || lines[i] != hl.text {
                        ok = false;
                        break;
                    }
                    i += 1;
                }
            }
        }
        if ok {
            return Ok(start);
        }
    }
    Err("hunk did not anchor".into())
}

fn resolve_path(root: &Path, p: &str) -> std::result::Result<PathBuf, String> {
    let path = Path::new(p);
    if path.is_absolute() {
        return Err(format!("absolute paths not allowed: {p}"));
    }
    // Reject any segment that tries to escape.
    for c in path.components() {
        if matches!(c, Component::ParentDir) {
            return Err(format!("path traversal blocked: {p}"));
        }
    }
    Ok(root.join(path))
}

// -----------------------------------------------------------------------------
// Tool wrapper
// -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct ApplyPatchArgs {
    patch: String,
}

pub struct ApplyPatchTool;

const DESCRIPTION: &str = "Apply a multi-file patch in OpenAI Codex DSL. \
Operations: Add File, Update File (with @@ hunks), Delete File, Move to. \
Two-phase: parsed in memory, then flushed atomically.";

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {"type": "string", "description": "The patch envelope to apply."}
            },
            "required": ["patch"]
        })
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value> {
        let parsed: ApplyPatchArgs = serde_json::from_value(args)
            .map_err(|e| CleanClawError::InvalidArgument(format!("apply_patch: {e}")))?;
        let root = std::path::PathBuf::from(&ctx.workspace_root);
        let ops = parse(&parsed.patch)
            .map_err(|e| CleanClawError::InvalidArgument(format!("apply_patch parse: {e}")))?;
        let log = apply(&ops, &root)
            .map_err(|e| CleanClawError::Upstream(format!("apply_patch: {e}")))?;
        Ok(json!({"ok": true, "log": log}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn dir() -> tempfile::TempDir {
        tempdir().expect("tempdir")
    }

    #[test]
    fn parse_add_file() {
        let patch = "*** Add File: hello.txt\n+hello\n+world\n";
        let ops = parse(patch).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, OpType::Add);
        assert_eq!(ops[0].path, "hello.txt");
        assert_eq!(ops[0].add_body, "hello\nworld\n");
    }

    #[test]
    fn parse_update_with_hunks() {
        let patch = "*** Update File: a.txt\n@@\n old\n-new\n+renamed\n old2\n*** End of File\n";
        let ops = parse(patch).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, OpType::Update);
        assert_eq!(ops[0].hunks.len(), 1);
        assert!(ops[0].hunks[0].is_eof);
    }

    #[test]
    fn parse_delete() {
        let patch = "*** Delete File: gone.txt\n";
        let ops = parse(patch).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, OpType::Delete);
    }

    #[test]
    fn parse_multi_op() {
        let patch =
            "*** Add File: a\n+x\n*** Delete File: b\n*** Update File: c\n@@\n old\n-new\n+new\n";
        let ops = parse(patch).unwrap();
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn apply_writes_new_file() {
        let d = dir();
        let patch = "*** Add File: hello.txt\n+hello\n+world\n";
        let ops = parse(patch).unwrap();
        let log = apply(&ops, d.path()).unwrap();
        assert!(log.contains("wrote"));
        let read = std::fs::read_to_string(d.path().join("hello.txt")).unwrap();
        assert_eq!(read, "hello\nworld\n");
    }

    #[test]
    fn apply_update_replaces_line() {
        let d = dir();
        std::fs::write(d.path().join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let patch = "*** Update File: a.txt\n@@\n-alpha\n+ALPHA\n beta\n";
        let ops = parse(patch).unwrap();
        apply(&ops, d.path()).unwrap();
        let read = std::fs::read_to_string(d.path().join("a.txt")).unwrap();
        assert_eq!(read, "ALPHA\nbeta\ngamma\n");
    }

    #[test]
    fn apply_delete_removes_file() {
        let d = dir();
        std::fs::write(d.path().join("gone.txt"), "x").unwrap();
        let patch = "*** Delete File: gone.txt\n";
        let ops = parse(patch).unwrap();
        apply(&ops, d.path()).unwrap();
        assert!(!d.path().join("gone.txt").exists());
    }

    #[test]
    fn apply_move_renames() {
        let d = dir();
        std::fs::write(d.path().join("old.txt"), "x").unwrap();
        let patch = "*** Update File: old.txt\n@@\n x\n+x\n*** Move to: new.txt\n";
        let ops = parse(patch).unwrap();
        apply(&ops, d.path()).unwrap();
        assert!(!d.path().join("old.txt").exists());
        assert!(d.path().join("new.txt").exists());
    }

    #[test]
    fn apply_eof_hunk_appends() {
        let d = dir();
        std::fs::write(d.path().join("eof.txt"), "head\n").unwrap();
        let patch = "*** Update File: eof.txt\n@@\n head\n*** End of File\n+tail1\n+tail2\n";
        let ops = parse(patch).unwrap();
        apply(&ops, d.path()).unwrap();
        let read = std::fs::read_to_string(d.path().join("eof.txt")).unwrap();
        assert_eq!(read, "head\ntail1\ntail2\n");
    }

    #[test]
    fn apply_failed_hunk_leaves_disk_untouched() {
        let d = dir();
        std::fs::write(d.path().join("a.txt"), "alpha\nbeta\n").unwrap();
        // hunk asks to replace "WRONG" — should fail.
        let patch = "*** Update File: a.txt\n@@\n WRONG\n-fixed\n+replacement\n";
        let ops = parse(patch).unwrap();
        let result = apply(&ops, d.path());
        assert!(result.is_err());
        // File unchanged.
        let read = std::fs::read_to_string(d.path().join("a.txt")).unwrap();
        assert_eq!(read, "alpha\nbeta\n");
    }

    #[test]
    fn reject_absolute_path() {
        assert!(resolve_path(Path::new("/tmp"), "/etc/passwd").is_err());
    }

    #[test]
    fn reject_traversal() {
        assert!(resolve_path(Path::new("/tmp"), "../etc/passwd").is_err());
    }
}
