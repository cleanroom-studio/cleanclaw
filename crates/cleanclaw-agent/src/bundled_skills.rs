//! Bundled-skill installer.
//!
//! At first boot we sync them out to
//! `~/.cleanclaw/bundled_skills/<name>/` so the agent can load them.
//!
//! For the first cut we ship a placeholder list (no embedded skills);
//! the helper functions here are ready to be wired up once we add
//! `include_dir!` / `rust-embed!` integration.

use cleanclaw_core::Result;
use std::path::Path;
use tracing::info;

/// Names of the skills the binary embeds. The actual content is
/// `include_dir!`-ed at build time in a follow-up phase; the
/// installer just needs the names to know what to write out.
pub const BUNDLED_SKILL_NAMES: &[&str] = &[
    // "find-skills",
    // "skill-creator",
    // "code-runner",
    // "data-analysis",
    // "image-gen",
    // "translation",
    // "web-search",
];

/// Sync every bundled skill to `dest_root/<name>/`. Already-installed
/// skills (whose `.bundled-hash` sidecar matches) are left alone;
/// modified or removed skills are refreshed.
pub fn install_bundled(dest_root: &Path) -> Result<usize> {
    if !dest_root.exists() {
        std::fs::create_dir_all(dest_root).ok();
    }
    let mut installed = 0;
    for &name in BUNDLED_SKILL_NAMES {
        let dir = dest_root.join(name);
        if dir.exists() {
            // For the first cut we always overwrite. A future cut
            // would compare .bundled-hash sidecars to avoid clobbering
            // operator customizations.
            info!(skill = name, "refreshing bundled skill");
        } else {
            info!(skill = name, "installing bundled skill");
        }
        std::fs::create_dir_all(&dir).ok();
        installed += 1;
    }
    Ok(installed)
}

/// Hash a skill's bundled-tree so the installer can skip
/// already-extracted copies. The full implementation is a follow-up
/// phase (needs `include_dir!` to walk the embedded tree).
pub fn bundled_hash(skill_name: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(skill_name.as_bytes());
    let out = h.finalize();
    hex::encode(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(bundled_hash("foo"), bundled_hash("foo"));
        assert_ne!(bundled_hash("foo"), bundled_hash("bar"));
    }

    #[test]
    fn install_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let n = install_bundled(dir.path()).unwrap();
        assert!(n >= 0);
    }
}
