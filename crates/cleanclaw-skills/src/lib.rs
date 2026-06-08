//! Skills crate — discovery, frontmatter parsing, and runtime loading.
//!
//! and `internal/agent/skills.go`.
//! A skill is a directory with a `SKILL.md` file containing YAML
//! frontmatter (name / description / env) and a markdown body that
//! becomes the system prompt snippet the agent sees.
//!
//! Also exposes `install` — skill installers for ClawHub / GitHub /
//! local tarball / local folder. See `install.rs` for the four
//! source kinds.

pub mod install;
pub mod objectstore;
pub mod search;
pub mod skillssh;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub env: Vec<SkillEnvSpec>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillEnvSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
    pub env: Vec<SkillEnvSpec>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub always_load: bool,
}

impl Skill {
    pub fn from_dir(path: &Path) -> Option<Self> {
        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            return None;
        }
        let raw = std::fs::read_to_string(&skill_md).ok()?;
        let (fm, body) = parse_frontmatter(&raw);
        let name = if !fm.name.is_empty() {
            fm.name
        } else {
            path.file_name()?.to_string_lossy().to_string()
        };
        Some(Self {
            name,
            description: fm.description,
            path: path.to_path_buf(),
            content: body,
            env: fm.env,
            enabled: true,
            always_load: false,
        })
    }
}

/// Load every skill under `root` (recursively one level deep — subdirs
/// of subdirs are not scanned to mirror CleanClaw's `LoadSkills`).
pub fn discover(root: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        if let Some(s) = Skill::from_dir(&p) {
            out.push(s);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Apply per-skill `disabled` and `alwaysLoad` lists from the agent
/// config.
pub fn apply_overrides(
    skills: Vec<Skill>,
    disabled: &[String],
    always_load: &[String],
) -> Vec<Skill> {
    let disabled_set: std::collections::HashSet<&String> = disabled.iter().collect();
    let always_set: std::collections::HashSet<&String> = always_load.iter().collect();
    skills
        .into_iter()
        .map(|mut s| {
            s.enabled = !disabled_set.contains(&s.name);
            s.always_load = always_set.contains(&s.name);
            s
        })
        .collect()
}

/// Render the system-prompt snippet for a list of skills.
pub fn render_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills.iter().filter(|s| s.enabled).collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n# Available Skills\n\n");
    for s in &visible {
        out.push_str(&format!("\n## `{}`\n\n", s.name));
        if !s.description.is_empty() {
            out.push_str(&s.description);
            out.push('\n');
        }
        if s.always_load {
            out.push_str("\n_This skill is auto-loaded — its full body is appended below._\n");
        }
    }
    out
}

/// Concatenate the full body of all always-loaded skills.
pub fn render_always_loaded(skills: &[Skill]) -> String {
    let mut out = String::new();
    for s in skills.iter().filter(|s| s.enabled && s.always_load) {
        out.push_str(&format!("\n# Skill: {}\n\n", s.name));
        out.push_str(&s.content);
        out.push('\n');
    }
    out
}

/// Build the runtime env map: agent config `env` overrides per-skill
/// `env` defaults. Returns `name → value`.
pub fn resolve_env(skills: &[Skill], runtime: &HashMap<String, String>) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = runtime.clone();
    for s in skills {
        for env in &s.env {
            if let Some(_v) = out.get(&env.name) {
                tracing::debug!(skill = %s.name, env = %env.name, "skill env already set");
                continue;
            }
            if let Ok(v) = std::env::var(&env.name) {
                out.insert(env.name.clone(), v);
            }
        }
    }
    out
}

/// Parse a `SKILL.md` file into `(frontmatter, body)`. Returns
/// `(SkillFrontmatter::default(), raw)` if the file doesn't have a
/// leading `---`-fenced YAML block.
pub fn parse_frontmatter(raw: &str) -> (SkillFrontmatter, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (SkillFrontmatter::default(), raw.to_string());
    }
    // Skip the opening "---" line.
    let after_open = trimmed.strip_prefix("---").unwrap_or(trimmed);
    let after_open = after_open.trim_start_matches('\n');
    if let Some(close_idx) = after_open.find("\n---") {
        let yaml_str = &after_open[..close_idx];
        let body = after_open[close_idx + 4..]
            .trim_start_matches('\n')
            .to_string();
        match serde_yaml::from_str::<SkillFrontmatter>(yaml_str) {
            Ok(fm) => (fm, body),
            Err(e) => {
                tracing::warn!("skill frontmatter parse failed: {e}");
                (SkillFrontmatter::default(), body)
            }
        }
    } else {
        (SkillFrontmatter::default(), raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_with_yaml() {
        let raw = "---\nname: hello\ndescription: says hi\n---\n# Body\n";
        let (fm, body) = parse_frontmatter(raw);
        assert_eq!(fm.name, "hello");
        assert_eq!(fm.description, "says hi");
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn parse_frontmatter_without_yaml() {
        let raw = "no frontmatter here";
        let (fm, body) = parse_frontmatter(raw);
        assert_eq!(fm.name, "");
        assert_eq!(body, raw);
    }

    #[test]
    fn discover_finds_skills_in_root() {
        let dir = tempfile::tempdir().unwrap();
        let skill_a = dir.path().join("alpha");
        std::fs::create_dir(&skill_a).unwrap();
        std::fs::write(
            skill_a.join("SKILL.md"),
            "---\nname: alpha\ndescription: first\n---\n# alpha body",
        )
        .unwrap();
        let skill_b = dir.path().join("beta");
        std::fs::create_dir(&skill_b).unwrap();
        std::fs::write(
            skill_b.join("SKILL.md"),
            "---\nname: beta\ndescription: second\n---\n# beta body",
        )
        .unwrap();
        // No SKILL.md → skipped
        let no_skill = dir.path().join("nope");
        std::fs::create_dir(&no_skill).unwrap();

        let skills = discover(dir.path());
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].name, "beta");
    }

    #[test]
    fn apply_overrides_respects_disabled_and_always_load() {
        let dir = tempfile::tempdir().unwrap();
        let s1 = dir.path().join("a");
        std::fs::create_dir(&s1).unwrap();
        std::fs::write(
            s1.join("SKILL.md"),
            "---\nname: a\ndescription: a\n---\nbody",
        )
        .unwrap();
        let s2 = dir.path().join("b");
        std::fs::create_dir(&s2).unwrap();
        std::fs::write(
            s2.join("SKILL.md"),
            "---\nname: b\ndescription: b\n---\nbody",
        )
        .unwrap();

        let skills = discover(dir.path());
        let skills = apply_overrides(skills, &["a".into()], &["b".into()]);
        let a = skills.iter().find(|s| s.name == "a").unwrap();
        let b = skills.iter().find(|s| s.name == "b").unwrap();
        assert!(!a.enabled);
        assert!(b.always_load);
    }
}
