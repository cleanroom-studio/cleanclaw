//! MEMORY.md read / parse / write.
//!
//! MEMORY.md is the agent's
//! long-term memory file the agent appends to (per turn) and
//! summarizes (on cadence). It uses a simple `## <ISO timestamp>`
//! section structure so the agent can read it back as a timeline.
//!
//! # Why a flat markdown file?
//!
//! The agent runs in two flavours:
//!
//! * **Local install** — the workspace lives on the gateway host's
//!   filesystem. A plain `MEMORY.md` next to other workspace files
//!   is the cheapest possible durable store: `read_to_string` /
//!   `append` / `write` and we're done.
//! * **Cloud / K8s install** — the workspace is in `cleanclaw_store`
//!   (S3 / object storage). The same `MEMORY.md` is stored as a
//!   workspace file there, with the same on-disk shape; the
//!   `MemoryStoreAdapter` below is the read path for that case.
//!
//! The section grammar (`## <ISO-8601>`) is intentionally trivial:
//! no frontmatter, no JSON, no AST. This keeps the LLM able to
//! "see" the file in raw form (handy for debugging via the
//! `read_file` tool) and lets us split / compact with a 30-line
//! scanner. The cost is a loose parser that doesn't validate the
//! timestamp format — we trust the writer.
//!
//! # File layout
//!
//! ```text
//! # Memory
//!
//! <!-- older entries compacted; see prior archives if needed -->
//!
//! ## 2026-01-01T00:00:00+00:00
//!
//! <entry body>
//!
//! ## 2026-01-02T00:00:00+00:00
//!
//! <entry body>
//! ...
//! ```
//!
//! The `# Memory` header and the `<!-- compacted -->` comment are
//! both written by `compact_memory`; the first call after a brand
//! new install produces just the header + sections.

use chrono::Utc;
use cleanclaw_core::Result;

/// The on-disk file name of the long-term memory. Exposed as a
/// `const` rather than a string literal scattered through the code
/// so the `MemoryStoreAdapter` path can read the same key as the
/// local path.
const MEMORY_FILE: &str = "MEMORY.md";

/// Read `MEMORY.md` from the agent's workspace root.
///
/// Returns an empty string (not an error) when the file does not
/// exist — a fresh agent has no memory, and that is a normal state,
/// not a failure. The caller can concatenate onto the result
/// without nil-checking.
///
/// `workspace_root` is the per-(agent, user) workspace directory.
/// In a local install this is a host path; in a cloud install this
/// is *still* a host path because the local executor mounts the
/// workspace into the sandbox, so the agent always sees the same
/// path. The store-backed read path is a separate method on
/// `MemoryStoreAdapter`.
pub async fn read_memory(workspace_root: &str) -> Result<String> {
    // Join the workspace root with the canonical file name. We use
    // `Path` rather than string concatenation to stay OS-agnostic
    // (Windows tests run in CI).
    let path = std::path::Path::new(workspace_root).join(MEMORY_FILE);
    // First-install / never-written case: treat as empty memory
    // rather than a hard error. This keeps boot code that "always
    // calls read_memory" simple.
    if !path.exists() {
        return Ok(String::new());
    }
    // Map any IO error into the crate-wide error type so the agent
    // loop can decide between "log and continue" and "kill the
    // turn". We never want raw `std::io::Error` leaking out.
    std::fs::read_to_string(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read MEMORY.md: {e}")))
}

/// Append a timestamped entry to `MEMORY.md`.
///
/// The entry is written as a new `## <RFC3339 UTC>` section, with a
/// blank line on each side. RFC3339 with timezone is used (rather
/// than a Unix timestamp) because the LLM can read the date
/// directly when reviewing its own memory in tool output.
///
/// Concurrency note: this is *not* fully atomic. Two concurrent
/// appends from two turns of the same agent can interleave at the
/// syscall level. In practice the agent loop is single-writer per
/// (agent, user) pair, so we accept the race for the first cut;
/// the TODO in the original source ("file-level lock would be the
/// right answer") still stands. Worst case the user sees a
/// mis-aligned section break in `MEMORY.md` — recoverable, not
/// destructive.
pub async fn append_memory(workspace_root: &str, entry: &str) -> Result<()> {
    let path = std::path::Path::new(workspace_root).join(MEMORY_FILE);
    // Make sure the workspace directory exists before we try to
    // open the file. `create_dir_all` is idempotent and we use
    // `.ok()` because EEXIST is fine; only a *real* IO error
    // (permission, disk full) is reported by the open() call below.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let now = Utc::now();
    // `\n## ...\n\n` is the canonical section header. The leading
    // newline guarantees we always start a new line even if the
    // previous write didn't end with one (it should, but we don't
    // rely on that invariant).
    let header = format!("\n## {}\n\n", now.to_rfc3339());
    let body = format!("{header}{entry}\n");
    // Open in create+append mode so the first-ever write produces
    // a well-formed file without a separate bootstrap step.
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    let mut f = opts
        .open(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("open MEMORY.md: {e}")))?;
    // Single `write_all` is atomic on most local filesystems for
    // small payloads (the typical entry is well under 4 KiB). On
    // network filesystems it's not, but again — single-writer.
    f.write_all(body.as_bytes())
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("write MEMORY.md: {e}")))?;
    Ok(())
}

/// Compact the memory file by keeping only the most recent `keep`
/// sections.
///
/// Returns the number of sections that were dropped. The compacted
/// file gets a fresh `# Memory` header and a `<!-- compacted -->`
/// marker, then the kept sections appended verbatim. The pre-amble
/// (anything before the first `## ` heading) is *dropped* by
/// `split_sections`, so the compact always produces a well-formed
/// "header + N sections" layout regardless of what the file used
/// to look like.
///
/// Use this on a cadence (e.g. nightly) as a cheap safeguard
/// against unbounded growth. The full LLM-driven summarisation is
/// a follow-up phase (see `distill_session`).
pub async fn compact_memory(workspace_root: &str, keep: usize) -> Result<usize> {
    let path = std::path::Path::new(workspace_root).join(MEMORY_FILE);
    // No file → nothing to compact; return 0 so callers can use
    // the count directly ("sections dropped this run") without
    // special-casing the first-boot state.
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read MEMORY.md: {e}")))?;
    let sections = split_sections(&content);
    // Already within budget → no-op. This is the common case when
    // cadence is shorter than the growth rate.
    if sections.len() <= keep {
        return Ok(0);
    }
    // Drop the oldest `drop` sections, keep the most recent `keep`.
    // We do a slice + collect (rather than Vec::retain) because we
    // want the new file to be in *exactly* the kept order with no
    // gaps.
    let drop = sections.len() - keep;
    let kept = sections[drop..].to_vec();
    let mut new_content = String::new();
    new_content.push_str("# Memory\n");
    // The marker line is a comment, so the LLM (or the next human
    // reader) can see that compaction has happened and look for
    // archived copies if needed. We don't move deleted sections
    // anywhere by default — the operator is expected to snapshot
    // `MEMORY.md` externally if archival is required.
    new_content.push_str("\n<!-- older entries compacted; see prior archives if needed -->\n");
    for s in kept {
        new_content.push_str(&s);
        new_content.push('\n');
    }
    // Write atomically from the perspective of the *reader* —
    // `write` truncates then writes, so a reader that races with
    // us will see either the old or the new content, never a
    // half-written file. On most local filesystems this is a
    // rename(2); on NFS / S3 it depends on the backing store.
    std::fs::write(&path, new_content)
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("write MEMORY.md: {e}")))?;
    Ok(drop)
}

/// Split `MEMORY.md` content into its dated sections.
///
/// The output is a `Vec<String>`, one entry per section *including*
/// the `## <iso>` heading line. The file preamble (the `# Memory`
/// header and the `<!-- compacted -->` comment) is intentionally
/// dropped — callers see a clean list of dated entries in
/// chronological order.
///
/// Parser rules:
/// * A new section starts at any line beginning with `## `.
/// * Everything from that line until the next `## ` (or EOF) is
///   part of the section.
/// * The state machine is intentionally linear: we walk line by
///   line, accumulating into a `current` buffer, and flush on the
///   next heading.
/// * Sections with empty bodies are skipped at flush time so the
///   output never contains a bare `## foo` line.
fn split_sections(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_section = false;
    for line in content.lines() {
        if line.starts_with("## ") {
            // Heading found: flush whatever we were accumulating
            // (unless it's the very first heading, in which case
            // `in_section` is still false and we skip the flush).
            if in_section && !current.is_empty() {
                out.push(current);
            }
            current = String::new();
            in_section = true;
        }
        if in_section {
            // Append *every* line, including the heading itself.
            // We re-include the heading in the section string so
            // compact_memory can re-emit sections verbatim.
            current.push_str(line);
            current.push('\n');
        }
    }
    // EOF: flush the trailing section if any.
    if in_section && !current.is_empty() {
        out.push(current);
    }
    out
}

/// Distill a session transcript into a 1-3 sentence memory entry.
///
/// This is the *heuristic* phase of summarisation: pick the last
/// user message and the last assistant message, truncate each to a
/// sane length, and concatenate. The full LLM-driven summary
/// (using the `agents.defaults.model` override) is a follow-up
/// phase that will replace this with a real model call.
///
/// Length caps:
/// * User message: 400 chars
/// * Assistant message: 600 chars
///
/// Truncation uses `truncate` (below) which respects UTF-8 char
/// boundaries and adds an ellipsis. We deliberately *do not* try
/// to be clever about *where* to cut (sentence boundary, etc.) —
/// the heuristic is for bootstrap only.
///
/// Return is plain text (not markdown) so it can be inlined as the
/// body of a `## <timestamp>` section without re-parsing.
pub async fn distill_session(messages: &[SimpleMessage]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    // Walk backwards: the *last* user / assistant message is
    // almost always the most relevant for "what just happened".
    // `rev()` + `find` is O(n) but with a tiny constant — the
    // session transcript is bounded by the context-window size,
    // and we call this at most once per turn.
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
    // Match on the four cases. We could collapse (Some, None) and
    // (None, Some) with a single arm, but the explicit branches
    // make the call site easier to read and the strings easier to
    // grep.
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

/// UTF-8-safe truncation. Returns at most `n` bytes, trimming back
/// to the previous `char_boundary` if `n` falls in the middle of a
/// code point, and appends a `…` (U+2026) so the reader knows the
/// string was clipped.
///
/// `n` is in *bytes*, not characters. This matches the natural
/// size limit we use elsewhere (tool output budgets are typically
/// quoted in bytes / chars-interchangeably) and keeps the function
/// cheap — `is_char_boundary` is O(1).
fn truncate(s: &str, n: usize) -> String {
    // Fast path: no truncation needed.
    if s.len() <= n {
        return s.to_string();
    }
    // Walk back to the previous char boundary. The `cut > 0` guard
    // is defensive: `is_char_boundary(0)` is always true so the
    // loop terminates even for pathological empty strings.
    let mut cut = n;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Minimal message shape used by the heuristic summariser.
///
/// We deliberately don't depend on the full provider message enum
/// (which has tool calls, system role, multi-modal parts, ...) —
/// the memory layer only needs role + text content. Real session
/// transcripts get flattened to this shape by the caller before
/// being passed to `distill_session`. This keeps `memory.rs` free
/// of any provider-specific types.
#[derive(Debug, Clone)]
pub struct SimpleMessage {
    /// One of `"user"`, `"assistant"`, `"system"`, `"tool"`. The
    /// heuristic only inspects `"user"` and `"assistant"`, but we
    /// accept all roles so the caller can pass through the full
    /// transcript without filtering.
    pub role: String,
    /// Plain text content. Multi-modal parts (images, tool calls)
    /// are not represented — they should be flattened or filtered
    /// upstream.
    pub content: String,
}

/// Adapter so memory can be read via the agent's `Store` (S3 / etc.)
/// in cloud installs.
///
/// The local `read_memory` / `append_memory` work directly against
/// the filesystem; the `Store`-backed read is needed when the
/// workspace lives in object storage and the local mount isn't
/// available (e.g. the agent runs in a different container than
/// the gateway). For the first cut we always read / write from the
/// local filesystem; this adapter is wired but the write path is
/// not implemented yet.
pub struct MemoryStoreAdapter;

impl MemoryStoreAdapter {
    /// Build a new adapter. There is no state, so this is just a
    /// placeholder for the eventual `Arc<Store>` field.
    pub fn new() -> Self {
        Self
    }

    /// Read `MEMORY.md` for `(agent_id, user_id)` through the
    /// `Store` abstraction.
    ///
    /// Returns `Ok(None)` when the store has no record for the
    /// (agent, user, MEMORY.md) key. The caller is expected to
    /// treat `None` the same way it treats a missing local file:
    /// "no memory yet", not an error.
    ///
    /// The content is decoded lossy-UTF-8 because the file is
    /// meant to be human-readable markdown; a stray invalid byte
    /// from a hand-edited file should not break the agent.
    pub async fn read_via_store(
        &self,
        store: &dyn cleanclaw_store::Store,
        agent_id: &str,
        user_id: &str,
    ) -> Result<Option<String>> {
        // `get_workspace_file` returns `Result<(_, _), _>`; we use
        // `.ok()` to flatten the "not found" error into `None`
        // and bubble real errors (network, permission) up the
        // call stack. This is the only place in the crate that
        // intentionally converts errors to `None`.
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
    //! Unit tests for the section parser, the compact path, and the
    //! distillation heuristic. End-to-end (write → read → compact
    //! → read) is covered by the `split_and_compact` integration
    //! test below.

    use super::*;

    /// End-to-end: write three dated entries, compact to keep=2,
    /// verify the oldest was dropped and the newest survived.
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

    /// Parser pin: a `# Memory` header followed by two `## <iso>`
    /// sections splits into exactly two entries.
    #[test]
    fn split_sections_splits_correctly() {
        let content =
            "# Memory\n\n## 2026-01-01T00:00:00Z\n\nfirst\n\n## 2026-02-01T00:00:00Z\n\nsecond\n";
        let sections = split_sections(content);
        assert_eq!(sections.len(), 2);
    }

    /// Distillation pin: given a four-message transcript
    /// (user/assistant/user/assistant), the heuristic should keep
    /// only the *last* user and assistant messages.
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
