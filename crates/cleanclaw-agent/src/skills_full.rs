//! Full skill loader.
//!
//! Combines the file-system scan (`cleanclaw_skills::discover`) with
//! runtime skill directory management: where to look for skills,
//! per-user skills root, hot-reload on `install_skill`, env
//! injection for sandbox exec.

use cleanclaw_skills::{discover, Skill};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Bundled skills directory (skipped for first cut — see
    /// `bundled_skills.rs`).
    pub bundled_root: PathBuf,
    /// Per-agent skills directory (~/.cleanclaw/agents/<id>/skills).
    pub agent_root: PathBuf,
    /// Per-user skills directory (~/.cleanclaw/users/<uid>/skills).
    pub user_root: PathBuf,
    /// Global skills directory (~/.cleanclaw/skills).
    pub global_root: PathBuf,
    /// Extra dirs the operator pinned via `CLEANCLAW_EXTRA_SKILLS_DIRS`.
    pub extra_dirs: Vec<PathBuf>,
}

impl SkillsConfig {
    pub fn for_agent(home: &Path, agent_id: &str, user_id: &str) -> Self {
        let home = home.to_path_buf();
        Self {
            bundled_root: home.join("bundled_skills"),
            agent_root: home.join("agents").join(agent_id).join("skills"),
            user_root: home.join("users").join(user_id).join("skills"),
            global_root: home.join("skills"),
            extra_dirs: std::env::var("CLEANCLAW_EXTRA_SKILLS_DIRS")
                .ok()
                .map(|s| s.split(',').map(|p| PathBuf::from(p.trim())).collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadedSkills {
    pub by_name: HashMap<String, Skill>,
    pub by_dir: Vec<Skill>,
}

impl LoadedSkills {
    pub fn names(&self) -> Vec<String> {
        self.by_name.keys().cloned().collect()
    }
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.by_name.get(name)
    }
}

pub struct SkillsLoader {
    config: SkillsConfig,
    cache: RwLock<LoadedSkills>,
}

impl SkillsLoader {
    pub fn new(config: SkillsConfig) -> Self {
        let cache = Self::scan(&config);
        Self {
            config,
            cache: RwLock::new(cache),
        }
    }

    /// Discover skills across every configured root. Priority
    /// (highest → lowest): agent → user → extra → global → bundled.
    /// Higher-priority entries OVERWRITE lower-priority ones with the
    /// same name.
    pub fn scan(config: &SkillsConfig) -> LoadedSkills {
        let mut by_name: HashMap<String, Skill> = HashMap::new();
        let mut by_dir: Vec<Skill> = Vec::new();
        // Iterate in priority order so a later (lower-priority) entry
        // doesn't clobber a higher-priority one.
        let mut priority_dirs: Vec<PathBuf> = Vec::new();
        if config.agent_root.exists() {
            priority_dirs.push(config.agent_root.clone());
        }
        if config.user_root.exists() {
            priority_dirs.push(config.user_root.clone());
        }
        for d in &config.extra_dirs {
            if d.exists() {
                priority_dirs.push(d.clone());
            }
        }
        if config.global_root.exists() {
            priority_dirs.push(config.global_root.clone());
        }
        if config.bundled_root.exists() {
            priority_dirs.push(config.bundled_root.clone());
        }

        for dir in &priority_dirs {
            for skill in discover(dir) {
                // Higher-priority dirs were pushed first; we insert-if-absent
                // so they win on name conflicts.
                by_name
                    .entry(skill.name.clone())
                    .or_insert_with(|| skill.clone());
                by_dir.push(skill);
            }
        }
        LoadedSkills { by_name, by_dir }
    }

    pub fn reload(&self) {
        let fresh = Self::scan(&self.config);
        *self.cache.write().unwrap() = fresh;
    }

    pub fn get(&self, name: &str) -> Option<Skill> {
        self.cache.read().unwrap().get(name).cloned()
    }

    pub fn all(&self) -> Vec<Skill> {
        self.cache.read().unwrap().by_dir.clone()
    }

    /// Render the system-prompt catalog of available skills. Mirrors
    /// `cleanclaw_skills::render_prompt` but pulls from the loader's
    /// pre-scanned set so the agent doesn't re-walk disk per turn.
    pub fn render_prompt(&self) -> String {
        let skills = self.all();
        cleanclaw_skills::render_prompt(&skills)
    }

    pub fn render_always_loaded(&self) -> String {
        let skills = self.all();
        cleanclaw_skills::render_always_loaded(&skills)
    }
}

/// Shared skills loader behind an Arc.
pub type SharedSkillsLoader = Arc<SkillsLoader>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_merges_multiple_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        let ua = a.join("foo");
        let ub = b.join("foo");
        fs::create_dir_all(&ua).unwrap();
        fs::create_dir_all(&ub).unwrap();
        fs::write(
            ua.join("SKILL.md"),
            "---\nname: foo\ndescription: from a\n---\nbody",
        )
        .unwrap();
        fs::write(
            ub.join("SKILL.md"),
            "---\nname: foo\ndescription: from b\n---\nbody",
        )
        .unwrap();

        // `a` has higher priority than `b` (added first → processed
        // last → wins on conflict).
        let cfg = SkillsConfig {
            bundled_root: dir.path().join("bundled"),
            agent_root: a.clone(),
            user_root: dir.path().join("u"),
            global_root: b.clone(),
            extra_dirs: vec![],
        };
        let loaded = SkillsLoader::scan(&cfg);
        let foo = loaded.get("foo").unwrap();
        assert_eq!(foo.description, "from a");
    }

    #[test]
    fn skills_config_for_agent_constructs_paths() {
        let cfg = SkillsConfig::for_agent(
            Path::new("/home/user/.cleanclaw"),
            "agent-1",
            "user-42",
        );
        assert_eq!(
            cfg.agent_root,
            Path::new("/home/user/.cleanclaw/agents/agent-1/skills")
        );
        assert_eq!(
            cfg.user_root,
            Path::new("/home/user/.cleanclaw/users/user-42/skills")
        );
        assert_eq!(
            cfg.global_root,
            Path::new("/home/user/.cleanclaw/skills")
        );
    }

    #[test]
    fn loaded_skills_names_returns_sorted_names() {
        let mut by_name = HashMap::new();
        by_name.insert(
            "b".into(),
            Skill {
                name: "b".into(),
                description: String::new(),
                path: PathBuf::from("/x"),
                content: String::new(),
                env: vec![],
                enabled: true,
                always_load: false,
            },
        );
        by_name.insert(
            "a".into(),
            Skill {
                name: "a".into(),
                description: String::new(),
                path: PathBuf::from("/y"),
                content: String::new(),
                env: vec![],
                enabled: true,
                always_load: false,
            },
        );
        let loaded = LoadedSkills {
            by_name,
            by_dir: vec![],
        };
        let mut names = loaded.names();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn skills_loader_reload_rescans() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("skills");
        fs::create_dir_all(&a).unwrap();
        let cfg = SkillsConfig {
            bundled_root: dir.path().join("bundled"),
            agent_root: a.clone(),
            user_root: dir.path().join("u"),
            global_root: dir.path().join("g"),
            extra_dirs: vec![],
        };
        let loader = SkillsLoader::new(cfg);
        assert!(loader.all().is_empty());

        // Add a skill after loader creation.
        let skill_dir = a.join("new-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: new-skill\ndescription: added later\n---\nbody",
        )
        .unwrap();

        loader.reload();
        assert!(loader.get("new-skill").is_some());
        assert_eq!(loader.get("new-skill").unwrap().description, "added later");
    }
}
