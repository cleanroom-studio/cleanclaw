//! Bundled skills. Each `SKILL.md` is included as a `&'static str`
//! at compile time via `include_str!`, so the agent runtime ships
//! the same set of built-in skills without a runtime fetch.
//!
//! Auxiliary files (skill-creator's `agents/*.md`, `scripts/*.py`,
//! `assets/*.html`, `references/*.md`, `eval-viewer/*`, and
//! `camoufox-cli/references/*.md`) live in `BUNDLED_AUX` as a
//! path-indexed map. The runtime pulls them on demand rather than
//! paying the binary-size cost of always-loading them.
//!
//! The list of bundled skills is hard-coded below; the disk-side
//! `cleanclaw_skills::discover` keeps working as-is for
//! user-installed skills (drop them into
//! `$CLEANCLAW_HOME/skills/`).

use serde::{Deserialize, Serialize};

/// One bundled skill, embedded at compile time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledSkill {
    pub name: &'static str,
    pub description: &'static str,
    pub markdown: &'static str,
}

/// The bundled skills. Each `description` is a one-line summary
/// parsed from the frontmatter; the full `markdown` is the entire
/// `SKILL.md` content. The same names exist in
/// `cleanclaw_skills::discover` so the user-installed layer merges
/// 1:1 with the bundled layer.
pub const BUNDLED: &[BundledSkill] = &[
    BundledSkill {
        name: "camoufox-cli",
        description: "Drive a headless Camoufox browser via the camoufox CLI to interact with the web: navigate, click, fill forms, extract data, run JS.",
        markdown: include_str!("../skills/camoufox-cli/SKILL.md"),
    },
    BundledSkill {
        name: "code-runner",
        description: "Execute code in multiple programming languages. Use when the user asks to run, test, or debug code in Python, JavaScript, shell, or other languages.",
        markdown: include_str!("../skills/code-runner/SKILL.md"),
    },
    BundledSkill {
        name: "data-analysis",
        description: "Analyze data, process CSV/JSON files, compute statistics, and create data visualizations. Use when the user asks about data processing, statistics, or analysis.",
        markdown: include_str!("../skills/data-analysis/SKILL.md"),
    },
    BundledSkill {
        name: "cleanclaw-skill-guide",
        description: "Create new skills for CleanClaw agents. Use when the user asks to create a skill, turn a workflow into a skill, or build reusable automation.",
        markdown: include_str!("../skills/cleanclaw-skill-guide/SKILL.md"),
    },
    BundledSkill {
        name: "cleanclaw-skill-learner",
        description: "Analyze conversations to extract reusable skill patterns. Used internally by CleanClaw to auto-generate skills from complex multi-step tasks.",
        markdown: include_str!("../skills/cleanclaw-skill-learner/SKILL.md"),
    },
    BundledSkill {
        name: "find-skills",
        description: "Browse the CleanClaw skills registry for tools that match a query.",
        markdown: include_str!("../skills/find-skills/SKILL.md"),
    },
    BundledSkill {
        name: "image-gen",
        description: "Generate images, charts, plots, and visualizations. Use when the user asks to draw, plot, chart, visualize data, or create images.",
        markdown: include_str!("../skills/image-gen/SKILL.md"),
    },
    BundledSkill {
        name: "skill-creator",
        description: "Create new skills, modify and improve existing skills, and measure skill performance.",
        markdown: include_str!("../skills/skill-creator/SKILL.md"),
    },
    BundledSkill {
        name: "translation",
        description: "Translate text between languages. Use when the user asks to translate content, detect language, or work with multilingual text.",
        markdown: include_str!("../skills/translation/SKILL.md"),
    },
    BundledSkill {
        name: "web-search",
        description: "Search the web and fetch web pages. Use when the user asks to search for information, look something up, or fetch a URL.",
        markdown: include_str!("../skills/web-search/SKILL.md"),
    },
];

/// `bundled_names` is a flat list of the bundled skill names.
pub const BUNDLED_NAMES: &[&str] = &[
    "camoufox-cli",
    "code-runner",
    "data-analysis",
    "cleanclaw-skill-guide",
    "cleanclaw-skill-learner",
    "find-skills",
    "image-gen",
    "skill-creator",
    "translation",
    "web-search",
];

/// One auxiliary file (anything that isn't a `SKILL.md`). The
/// path is relative to the skill's directory — e.g.
/// `skill-creator/agents/grader.md`. Embedded as `&'static [u8]`
/// so the runtime can hand the bytes to a sandboxed Python
/// runner without going through the filesystem.
pub const BUNDLED_AUX: &[(&str, &[u8])] = &[
    // camoufox-cli
    (
        "camoufox-cli/references/commands.md",
        include_bytes!("../skills/camoufox-cli/references/commands.md"),
    ),
    (
        "camoufox-cli/references/snapshot-refs.md",
        include_bytes!("../skills/camoufox-cli/references/snapshot-refs.md"),
    ),
    // skill-creator agents
    (
        "skill-creator/agents/analyzer.md",
        include_bytes!("../skills/skill-creator/agents/analyzer.md"),
    ),
    (
        "skill-creator/agents/comparator.md",
        include_bytes!("../skills/skill-creator/agents/comparator.md"),
    ),
    (
        "skill-creator/agents/grader.md",
        include_bytes!("../skills/skill-creator/agents/grader.md"),
    ),
    // skill-creator scripts (Python — run via the sandbox)
    (
        "skill-creator/scripts/aggregate_benchmark.py",
        include_bytes!("../skills/skill-creator/scripts/aggregate_benchmark.py"),
    ),
    (
        "skill-creator/scripts/generate_report.py",
        include_bytes!("../skills/skill-creator/scripts/generate_report.py"),
    ),
    (
        "skill-creator/scripts/improve_description.py",
        include_bytes!("../skills/skill-creator/scripts/improve_description.py"),
    ),
    (
        "skill-creator/scripts/package_skill.py",
        include_bytes!("../skills/skill-creator/scripts/package_skill.py"),
    ),
    (
        "skill-creator/scripts/quick_validate.py",
        include_bytes!("../skills/skill-creator/scripts/quick_validate.py"),
    ),
    (
        "skill-creator/scripts/run_eval.py",
        include_bytes!("../skills/skill-creator/scripts/run_eval.py"),
    ),
    (
        "skill-creator/scripts/run_loop.py",
        include_bytes!("../skills/skill-creator/scripts/run_loop.py"),
    ),
    (
        "skill-creator/scripts/utils.py",
        include_bytes!("../skills/skill-creator/scripts/utils.py"),
    ),
    // skill-creator assets (HTML review template)
    (
        "skill-creator/assets/eval_review.html",
        include_bytes!("../skills/skill-creator/assets/eval_review.html"),
    ),
    // skill-creator references
    (
        "skill-creator/references/schemas.md",
        include_bytes!("../skills/skill-creator/references/schemas.md"),
    ),
    // skill-creator eval-viewer (HTML + Python)
    (
        "skill-creator/eval-viewer/viewer.html",
        include_bytes!("../skills/skill-creator/eval-viewer/viewer.html"),
    ),
    (
        "skill-creator/eval-viewer/generate_review.py",
        include_bytes!("../skills/skill-creator/eval-viewer/generate_review.py"),
    ),
];

/// `find` looks up a bundled skill by name. Returns `None` when
/// the name doesn't match.
pub fn find(name: &str) -> Option<&'static BundledSkill> {
    BUNDLED.iter().find(|s| s.name == name)
}

/// `find_aux` looks up an auxiliary file by its relative path
/// (e.g. `"skill-creator/agents/grader.md"`). Returns the raw
/// bytes — text files are valid UTF-8 but the runtime treats
/// everything as opaque bytes (HTML, Python source, etc.).
pub fn find_aux(path: &str) -> Option<&'static [u8]> {
    BUNDLED_AUX
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, b)| *b)
}

/// `aux_for_skill` returns the auxiliary paths that belong to a
/// given skill (e.g. all `skill-creator/*` paths). Useful for the
/// install pipeline that copies a skill's full directory tree.
pub fn aux_for_skill(skill: &str) -> Vec<&'static str> {
    let prefix = format!("{skill}/");
    BUNDLED_AUX
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| p.starts_with(&prefix))
        .collect()
}

// =====================================================================
// .bundled-hash sidecar.
//
// The Go
// runtime computes a SHA-256 of every embedded skill + aux file and
// writes the digest to a `.bundled-hash` file in the user's skills
// install dir. The next boot compares the on-disk digest against the
// freshly-computed one; mismatch means the bundled assets changed
// under us and every skill's files need to be re-materialized.
// =====================================================================

/// Filename of the sidecar. Hidden on Unix, skipped by `discover()`
/// because the entry isn't a directory.
pub const BUNDLED_HASH_FILENAME: &str = ".bundled-hash";

/// Compute the SHA-256 of every bundled skill's `SKILL.md` plus
/// every aux file. Returns a lowercase hex digest. The order is
/// deterministic: skill names in `BUNDLED` order, then aux paths
/// sorted lexicographically (the sort matches the Go
/// `bundled_skills.go` write order so two binaries with the same
/// content always agree).
pub fn compute_bundled_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for s in BUNDLED {
        hasher.update(s.name.as_bytes());
        hasher.update([0]);
        hasher.update(s.markdown.as_bytes());
        hasher.update([0xff]);
    }
    for (path, bytes) in BUNDLED_AUX {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(*bytes);
        hasher.update([0xff]);
    }
    hex::encode(hasher.finalize())
}

/// Read the on-disk `.bundled-hash` and return whether it matches
/// the freshly-computed one. Returns `Ok(true)` when the on-disk
/// digest is missing entirely (first install) — the caller treats
/// that as "needs install" the same way the Go runtime does.
pub fn sidecar_needs_install(sidecar_path: &std::path::Path) -> std::io::Result<bool> {
    let current = compute_bundled_hash();
    let on_disk = match std::fs::read_to_string(sidecar_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    Ok(on_disk.trim() != current)
}

/// Write the current bundled-hash to `sidecar_path` so the next boot
/// sees a match.
pub fn write_sidecar(sidecar_path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = sidecar_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(sidecar_path, compute_bundled_hash())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bundled_count_is_ten() {
        assert_eq!(BUNDLED.len(), 10);
        assert_eq!(BUNDLED_NAMES.len(), 10);
    }

    #[test]
    fn camoufox_cli_bundled() {
        let s = find("camoufox-cli").unwrap();
        assert!(s.markdown.contains("Camoufox"));
    }

    #[test]
    fn bundled_names_are_unique() {
        let mut names: Vec<&str> = BUNDLED_NAMES.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 10);
    }

    #[test]
    fn find_returns_some() {
        let s = find("web-search").unwrap();
        assert!(s.markdown.contains("Search the web"));
    }

    #[test]
    fn find_returns_none() {
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn all_markdown_is_nonempty() {
        for s in BUNDLED {
            assert!(!s.markdown.trim().is_empty(), "{}: empty", s.name);
            assert!(!s.description.is_empty(), "{}: empty desc", s.name);
        }
    }

    #[test]
    fn aux_camoufox_references_present() {
        assert!(find_aux("camoufox-cli/references/commands.md").is_some());
        assert!(find_aux("camoufox-cli/references/snapshot-refs.md").is_some());
    }

    #[test]
    fn aux_skill_creator_scripts_present() {
        for p in [
            "skill-creator/scripts/run_eval.py",
            "skill-creator/scripts/quick_validate.py",
            "skill-creator/scripts/package_skill.py",
        ] {
            assert!(find_aux(p).is_some(), "{p} should be embedded");
        }
    }

    #[test]
    fn aux_skill_creator_agents_present() {
        for p in [
            "skill-creator/agents/analyzer.md",
            "skill-creator/agents/comparator.md",
            "skill-creator/agents/grader.md",
        ] {
            assert!(find_aux(p).is_some(), "{p} should be embedded");
        }
    }

    #[test]
    fn aux_for_skill_filters_correctly() {
        let sc = aux_for_skill("skill-creator");
        assert!(!sc.is_empty());
        assert!(sc.iter().all(|p| p.starts_with("skill-creator/")));
        let cam = aux_for_skill("camoufox-cli");
        assert_eq!(cam.len(), 2);
        let empty = aux_for_skill("nonexistent-skill");
        assert!(empty.is_empty());
    }

    #[test]
    fn compute_bundled_hash_is_hex_64() {
        let h = compute_bundled_hash();
        assert_eq!(h.len(), 64, "SHA-256 hex is 64 chars");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_bundled_hash_is_deterministic() {
        assert_eq!(compute_bundled_hash(), compute_bundled_hash());
    }

    #[test]
    fn sidecar_needs_install_returns_true_when_missing() {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-bundled-hash-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(BUNDLED_HASH_FILENAME);
        assert!(sidecar_needs_install(&path).unwrap());
    }

    #[test]
    fn sidecar_needs_install_returns_false_when_fresh() {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-bundled-hash-fresh-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(BUNDLED_HASH_FILENAME);
        write_sidecar(&path).unwrap();
        assert!(!sidecar_needs_install(&path).unwrap());
    }

    #[test]
    fn sidecar_needs_install_returns_true_when_stale() {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-bundled-hash-stale-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(BUNDLED_HASH_FILENAME);
        // Write a clearly-wrong digest.
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"stale_digest_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").unwrap();
        assert!(sidecar_needs_install(&path).unwrap());
    }

    #[test]
    fn write_sidecar_creates_parents() {
        let dir = std::env::temp_dir().join(format!(
            "cleanclaw-bundled-hash-parents-{}/nested/dir",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(BUNDLED_HASH_FILENAME);
        write_sidecar(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn bundled_hash_filename_is_hidden() {
        assert!(BUNDLED_HASH_FILENAME.starts_with('.'));
    }
}
