//! SkillsLearner — observe complex tasks and extract reusable
//! skill patterns. Mirrors
//! .
//!
//! After an agent turn that exceeded the tool-call threshold, the
//! learner asks the LLM to summarize the multi-step pattern into
//! a SKILL.md-shaped suggestion. The runtime stores the
//! suggestion in the workspace's `skills/<slug>/SKILL.md` path
//! where the regular skills loader can pick it up.

use std::path::Path;

use cleanclaw_provider::Message;
use serde::{Deserialize, Serialize};

/// Minimum tool calls before we ask the LLM to extract a skill.
pub const DEFAULT_MIN_TOOL_CALLS: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedSkill {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResponse {
    pub extract: bool,
    #[serde(default)]
    pub skill: ExtractedSkill,
}

/// Suggestions the learner can make about a conversation. When
/// `extract` is false, the conversation didn't warrant a skill —
/// the tool calls were routine. The LLM response is parsed here
/// rather than inside `MaybeExtract` so tests can drive the
/// extraction step without a live provider.
pub fn parse_extraction(raw: &str) -> Result<ExtractionResponse, String> {
    // Strip optional ```json fences.
    let trimmed = raw.trim();
    let body = trimmed
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
        .or_else(|| {
            trimmed
                .strip_prefix("```")
                .and_then(|s| s.strip_suffix("```"))
        })
        .unwrap_or(trimmed);
    serde_json::from_str(body.trim()).map_err(|e| format!("parse extraction: {e}"))
}

/// Slugify a name: lower-case, replace runs of non-alphanumeric
/// with `-`, trim leading/trailing `-`.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed
    }
}

/// Decide whether a turn with `tool_call_count` is worth
/// extracting from. The full MaybeExtract in Go also embeds the
/// conversation messages into a prompt; we keep the heuristic
/// surface here so the caller can decide what to do with it.
pub fn should_extract(tool_call_count: usize, min_tool_calls: usize) -> bool {
    tool_call_count >= min_tool_calls
}

/// Render a `SKILL.md` from an extracted skill. Mirrors the front
/// matter format used by /SKILL.md`.
pub fn render_skill_md(s: &ExtractedSkill) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {}\n", s.name));
    out.push_str(&format!("description: {}\n", s.description));
    out.push_str("---\n\n");
    out.push_str(&s.content);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Persist `s` to `<workspace>/skills/<slug>/SKILL.md`. Returns
/// the absolute path on success. Does NOT mkdir `<workspace>` —
/// the caller is expected to ensure the workspace exists.
pub fn write_to_workspace(s: &ExtractedSkill, workspace: &Path) -> std::io::Result<std::path::PathBuf> {
    let dir = workspace.join("skills").join(&s.slug);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, render_skill_md(s))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_provider::Role;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Find Skills by Query"), "find-skills-by-query");
        assert_eq!(slugify("hello world"), "hello-world");
        assert_eq!(slugify("  trim  me  "), "trim-me");
    }

    #[test]
    fn slugify_falls_back_when_empty() {
        assert_eq!(slugify("///"), "skill");
        assert_eq!(slugify(""), "skill");
    }

    #[test]
    fn parse_extraction_unfenced() {
        let raw = r#"{"extract": true, "skill": {"name": "X", "slug": "x", "description": "d", "content": "c"}}"#;
        let r = parse_extraction(raw).unwrap();
        assert!(r.extract);
        assert_eq!(r.skill.name, "X");
    }

    #[test]
    fn parse_extraction_fenced() {
        let raw = "```json\n{\"extract\": false, \"skill\": {\"name\": \"\", \"slug\": \"\", \"description\": \"\", \"content\": \"\"}}\n```";
        let r = parse_extraction(raw).unwrap();
        assert!(!r.extract);
    }

    #[test]
    fn parse_extraction_invalid() {
        let r = parse_extraction("not json");
        assert!(r.is_err());
    }

    #[test]
    fn should_extract_threshold() {
        assert!(!should_extract(2, DEFAULT_MIN_TOOL_CALLS));
        assert!(should_extract(3, DEFAULT_MIN_TOOL_CALLS));
        assert!(should_extract(10, DEFAULT_MIN_TOOL_CALLS));
    }

    #[test]
    fn render_skill_md_has_frontmatter() {
        let s = ExtractedSkill {
            name: "Foo".into(),
            slug: "foo".into(),
            description: "do foo".into(),
            content: "Step 1: ...\nStep 2: ...\n".into(),
        };
        let md = render_skill_md(&s);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: Foo"));
        assert!(md.contains("description: do foo"));
        assert!(md.contains("Step 1:"));
    }

    #[test]
    fn write_to_workspace_creates_file() {
        let s = ExtractedSkill {
            name: "Foo".into(),
            slug: "foo".into(),
            description: "d".into(),
            content: "c".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = write_to_workspace(&s, dir.path()).unwrap();
        assert!(path.exists());
        let read = std::fs::read_to_string(&path).unwrap();
        assert!(read.contains("name: Foo"));
    }

    #[test]
    fn message_constructs_for_extraction() {
        // Sanity check that the provider's Message type is
        // importable — the real extraction prompt is built by
        // the agent loop using the same Message.
        let m = Message::user("hi");
        assert!(matches!(m.role, Role::User));
    }
}
