//! Default workspace files. `.
//!
//! When the agent runtime initialises a new agent workspace, it
//! seeds the working directory with these four markdown files:
//!
//!   * `AGENTS.md`  — agent configuration / system-prompt anchor
//!   * `SOUL.md`    — persona / tone
//!   * `TOOLS.md`   — tools reference (auto-updated by the agent loop)
//!   * `USER.md`    — user profile (auto-updated by auto_persist)
//!
//! All four are embedded at compile time via `include_str!` so the
//! runtime never has to fetch defaults from disk.

/// The default `AGENTS.md` content.
pub const AGENTS_MD: &str = include_str!("../files/AGENTS.md");

/// The default `SOUL.md` content.
pub const SOUL_MD: &str = include_str!("../files/SOUL.md");

/// The default `TOOLS.md` content.
pub const TOOLS_MD: &str = include_str!("../files/TOOLS.md");

/// The default `USER.md` content.
pub const USER_MD: &str = include_str!("../files/USER.md");

/// The four default filenames in the order the runtime seeds
/// them. The order matters: `AGENTS.md` is the system-prompt
/// anchor and gets loaded first.
pub const DEFAULT_FILES: &[(&str, &str)] = &[
    ("AGENTS.md", AGENTS_MD),
    ("SOUL.md", SOUL_MD),
    ("TOOLS.md", TOOLS_MD),
    ("USER.md", USER_MD),
];

/// `seed_to` writes the four default files into `dir`, skipping
/// any that already exist (so user customizations are preserved).
pub fn seed_to(dir: &std::path::Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut written = 0;
    for (name, content) in DEFAULT_FILES {
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        std::fs::write(path, content)?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_nonempty() {
        for (name, content) in DEFAULT_FILES {
            assert!(!content.trim().is_empty(), "{name}: empty");
        }
    }

    #[test]
    fn defaults_have_headings() {
        for (name, content) in DEFAULT_FILES {
            assert!(
                content.starts_with("# "),
                "{name}: missing top-level heading"
            );
        }
    }

    #[test]
    fn seed_to_writes_only_missing() {
        let dir = tempdir();
        let n1 = seed_to(&dir).unwrap();
        assert_eq!(n1, 4);
        let n2 = seed_to(&dir).unwrap();
        assert_eq!(n2, 0);
        // Round-trip one file back to verify the content.
        let s = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert!(s.contains("CleanClaw") || s.contains("Agent"));
    }

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "cleanclaw-ws-{}-{}",
            std::process::id(),
            cleanclaw_core::IdGen::new().next("t")
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
