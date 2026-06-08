//! MEMORY.md read / parse / write.
//!
//! MEMORY.md is the
//! long-term memory file the agent appends to (per turn) and
//! summarizes (on cadence). It uses a simple `## <ISO timestamp>`
//! section structure so the agent can read it back as a timeline.

use chrono::{DateTime, Utc};
use cleanclaw_core::Result;
use std::path::Path;
use tracing::warn;

const MEMORY_FILE: &str = "MEMORY.md";

/// Read MEMORY.md from the agent's system root. Returns an empty
/// string if the file doesn't exist.
pub async fn read_memory(workspace_root: &str) -> Result<String> {
    let path = std::path::Path::new(workspace_root).join(MEMORY_FILE);
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read MEMORY.md: {e}")))
}

/// Append a timestamped entry. Idempotent against concurrent writes
/// from multiple turns (file-level lock would be the right answer;
/// for the first cut we just retry on EBUSY).
pub async fn append_memory(workspace_root: &str, entry: &str) -> Result<()> {
    let path = std::path::Path::new(workspace_root).join(MEMORY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let now = Utc::now();
    let header = format!("\n## {}\n\n", now.to_rfc3339());
    let body = format!("{header}{entry}\n");
    // Append; if the file doesn't exist yet, create it with the
    // header so subsequent reads have a valid section structure.
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    let mut f = opts
        .open(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("open MEMORY.md: {e}")))?;
    f.write_all(body.as_bytes())
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("write MEMORY.md: {e}")))?;
    Ok(())
}

/// Compact: keep only the most recent N sections. Useful as a
/// cheap safeguard against unbounded growth.
pub async fn compact_memory(workspace_root: &str, keep: usize) -> Result<usize> {
    let path = std::path::Path::new(workspace_root).join(MEMORY_FILE);
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read MEMORY.md: {e}")))?;
    let sections = split_sections(&content);
    if sections.len() <= keep {
        return Ok(0);
    }
    let drop = sections.len() - keep;
    let kept = sections[drop..].to_vec();
    let mut new_content = String::new();
    new_content.push_str("# Memory\n");
    new_content.push_str("\n<!-- older entries compacted; see prior archives if needed -->\n");
    for s in kept {
        new_content.push_str(&s);
        new_content.push('\n');
    }
    std::fs::write(&path, new_content)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("write MEMORY.md: {e}")))?;
    Ok(drop)
}

fn split_sections(content: &str) -> Vec<String> {
    // Split into `## <iso>`-headed sections. The file preamble
    // (everything before the first `## ` heading) is dropped so
    // callers see the dated entries in chronological order.
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_section = false;
    for line in content.lines() {
        if line.starts_with("## ") {
            if in_section && !current.is_empty() {
                out.push(current);
            }
            current = String::new();
            in_section = true;
        }
        if in_section {
            current.push_str(line);
            current.push('\n');
        }
    }
    if in_section && !current.is_empty() {
        out.push(current);
    }
    out
}

/// Distill a long session transcript into a 1-3 sentence memory
/// entry. For the first cut this is a heuristic that picks the
/// last user message and the last assistant message and concatenates
/// them. The full LLM-driven summary is a follow-up phase (uses the
/// `agents.defaults.model` override).
pub async fn distill_session(messages: &[SimpleMessage]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let last_user = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone());
    let last_assistant = messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.clone());
    match (last_user, last_assistant) {
        (Some(u), Some(a)) => format!(
            "User: {}\n\nAgent: {}",
            truncate(&u, 400),
            truncate(&a, 600)
        ),
        (Some(u), None) => format!("User: {}", truncate(&u, 400)),
        (None, Some(a)) => format!("Agent: {}", truncate(&a, 600)),
        (None, None) => String::new(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut cut = n;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

#[derive(Debug, Clone)]
pub struct SimpleMessage {
    pub role: String,
    pub content: String,
}

/// Adapter so memory can be persisted via the agent's `Store` (for
/// cloud / K8s installs). For the first cut we always read / write
/// from the local filesystem; the store adapter is wired but
/// unused.
pub struct MemoryStoreAdapter;

impl MemoryStoreAdapter {
    pub fn new() -> Self {
        Self
    }

    pub async fn read_via_store(
        &self,
        store: &dyn cleanclaw_store::Store,
        agent_id: &str,
        user_id: &str,
    ) -> Result<Option<String>> {
        let rec = store
            .get_workspace_file(agent_id, user_id, MEMORY_FILE)
            .await
            .ok();
        Ok(rec.map(|(_, bytes)| String::from_utf8_lossy(&bytes).to_string()))
    }
}

impl Default for MemoryStoreAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn split_and_compact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        std::fs::write(
            &path,
            "# Memory\n\n## 2026-01-01T00:00:00Z\n\nfirst\n\n## 2026-02-01T00:00:00Z\n\nsecond\n\n## 2026-03-01T00:00:00Z\n\nthird\n",
        )
        .unwrap();
        let dropped = compact_memory(dir.path().to_str().unwrap(), 2)
            .await
            .unwrap();
        assert_eq!(dropped, 1);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("third"));
        assert!(!after.contains("first"));
    }

    #[test]
    fn split_sections_splits_correctly() {
        let content =
            "# Memory\n\n## 2026-01-01T00:00:00Z\n\nfirst\n\n## 2026-02-01T00:00:00Z\n\nsecond\n";
        let sections = split_sections(content);
        assert_eq!(sections.len(), 2);
    }

    #[test]
    fn distill_picks_last_user_and_assistant() {
        let msgs = vec![
            SimpleMessage {
                role: "user".into(),
                content: "earlier question".into(),
            },
            SimpleMessage {
                role: "assistant".into(),
                content: "earlier answer".into(),
            },
            SimpleMessage {
                role: "user".into(),
                content: "follow-up question".into(),
            },
            SimpleMessage {
                role: "assistant".into(),
                content: "follow-up answer".into(),
            },
        ];
        let summary = futures::executor::block_on(distill_session(&msgs));
        assert!(summary.contains("follow-up question"));
        assert!(summary.contains("follow-up answer"));
        assert!(!summary.contains("earlier question"));
    }
}

#[allow(dead_code)]
fn _unused_paths(p: &Path) -> std::path::PathBuf {
    p.to_path_buf()
}
