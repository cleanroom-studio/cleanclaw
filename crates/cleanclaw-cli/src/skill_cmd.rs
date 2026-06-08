//! `cleanclaw skill …` — list / install / search / update / remove
//! / info skills.
//!
//! The full set:
//!
//! * `ls`          — list installed skills
//! * `show <name>` — print a skill's SKILL.md
//! * `search <q>`  — search the registry (skills.sh) for skills
//! * `install <name> [--source skillssh|clawhub|github] [--repo owner/repo]`
//! * `update <name>` — refresh from the original source
//! * `rm <name>`   — remove an installed skill
//! * `info <name>` — print parsed metadata (name, description, envSpec)

use clap::Subcommand;
use cleanclaw_core::Result;
use cleanclaw_skills::discover;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum SkillCmd {
    /// List installed skills under $CLEANCLAW_HOME/skills.
    Ls,
    /// Show a skill's SKILL.md content.
    Show { name: String },
    /// Search the skills.sh registry for skills matching a query.
    Search { query: String },
    /// Install a skill (registry by default; --repo for github/clawhub).
    Install {
        name: String,
        #[arg(long, default_value = "auto")]
        source: String,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Refresh an installed skill from its original source.
    Update { name: String },
    /// Remove an installed skill.
    Rm { name: String },
    /// Print a skill's parsed metadata.
    Info { name: String },
}

pub async fn run(cmd: SkillCmd) -> Result<()> {
    match cmd {
        SkillCmd::Ls => ls(),
        SkillCmd::Show { name } => show(&name),
        SkillCmd::Search { query } => search(&query).await,
        SkillCmd::Install {
            name,
            source,
            repo,
            path,
        } => install(&name, &source, repo.as_deref(), path.as_deref()),
        SkillCmd::Update { name } => update(&name),
        SkillCmd::Rm { name } => rm(&name),
        SkillCmd::Info { name } => info(&name),
    }
}

fn skills_root() -> PathBuf {
    cleanclaw_config::env::home_dir().join("skills")
}

fn ls() -> Result<()> {
    let root = skills_root();
    if !root.exists() {
        println!("(no skills — drop one into {})", root.display());
        return Ok(());
    }
    let skills = discover(&root);
    if skills.is_empty() {
        println!("(no skills)");
        return Ok(());
    }
    for s in skills {
        println!("{:<24} {}", s.name, s.description);
    }
    Ok(())
}

fn show(name: &str) -> Result<()> {
    let path = skills_root().join(name).join("SKILL.md");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| cleanclaw_core::CleanClawError::NotFound(format!("{name}: {e}")))?;
    print!("{raw}");
    Ok(())
}

async fn search(query: &str) -> Result<()> {
    let hits = cleanclaw_skills::search::search_registry(query, 25).await?;
    if hits.is_empty() {
        println!("(no results for '{query}')");
        return Ok(());
    }
    for h in hits {
        println!("{:<32} {}", h.name, h.description);
        if !h.author.is_empty() || !h.repo.is_empty() {
            let author = if h.author.is_empty() { "?" } else { &h.author };
            let repo = if h.repo.is_empty() { "?" } else { &h.repo };
            println!("    by {author}  ({repo})");
        }
        if !h.tags.is_empty() {
            println!("    tags: {}", h.tags.join(", "));
        }
    }
    Ok(())
}

fn install(
    name: &str,
    source: &str,
    repo: Option<&str>,
    path: Option<&std::path::Path>,
) -> Result<()> {
    let dest = skills_root().join(name);
    if dest.exists() {
        return Err(cleanclaw_core::CleanClawError::Conflict(format!(
            "skill {name} already installed"
        )));
    }
    std::fs::create_dir_all(&dest)?;
    match (source, path, repo) {
        ("local" | "path", Some(p), _) => {
            copy_dir_recursive(p, &dest)?;
        }
        ("github", _, Some(r)) => {
            let client = std::sync::Arc::new(reqwest::Client::new());
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("rt: {e}")))?;
            rt.block_on(cleanclaw_skills::install::install_from_github(
                client, r, name, &dest,
            ))
            .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("install: {e}")))?;
        }
        ("github", _, None) => {
            return Err(cleanclaw_core::CleanClawError::InvalidArgument(
                "--repo <owner/repo> required for source=github".into(),
            ));
        }
        _ => {
            // Default: skills.sh slug
            let client = std::sync::Arc::new(reqwest::Client::new());
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("rt: {e}")))?;
            rt.block_on(cleanclaw_skills::install::install_from_clawhub(
                client, name, &dest,
            ))
            .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("install: {e}")))?;
            let _ = repo; // future hook
        }
    }
    println!("installed {name}");
    Ok(())
}

fn update(name: &str) -> Result<()> {
    let dest = skills_root().join(name);
    if !dest.exists() {
        return Err(cleanclaw_core::CleanClawError::NotFound(format!(
            "skill {name}"
        )));
    }
    // Re-run the install path over the existing dir. The install
    // helpers all check `dest.exists()` first and refuse; the update
    // semantics in the Go CLI is to wipe the dir first.
    std::fs::remove_dir_all(&dest)?;
    install(name, "auto", None, None)?;
    println!("updated {name}");
    Ok(())
}

fn rm(name: &str) -> Result<()> {
    let dest = skills_root().join(name);
    if !dest.exists() {
        return Err(cleanclaw_core::CleanClawError::NotFound(format!(
            "skill {name}"
        )));
    }
    std::fs::remove_dir_all(&dest)?;
    println!("removed {name}");
    Ok(())
}

fn info(name: &str) -> Result<()> {
    let root = skills_root();
    let skills = discover(&root);
    let s = skills
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| cleanclaw_core::CleanClawError::NotFound(format!("skill {name}")))?;
    println!("name:        {}", s.name);
    println!("description: {}", s.description);
    println!("path:        {}", s.path.display());
    println!("enabled:     {}", s.enabled);
    println!("always_load: {}", s.always_load);
    if !s.env.is_empty() {
        println!();
        println!("# env spec");
        for sp in &s.env {
            let flag = if sp.required { " (required)" } else { "" };
            println!("  {}{}", sp.name, flag);
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_root_resolves() {
        let _ = skills_root();
    }

    #[test]
    fn registry_search_url_constant() {
        assert_eq!(
            cleanclaw_skills::search::SKILLS_SH_SEARCH,
            "https://skills.sh/api/search"
        );
    }

    #[test]
    fn urlencode_helper_for_queries() {
        // The same urlencode the search module uses.
        assert_eq!(
            cleanclaw_skills::search::urlencode("hello world"),
            "hello+world"
        );
    }

    #[test]
    fn parse_response_handles_empty() {
        use serde_json::json;
        let hits = cleanclaw_skills::search::parse_response(&json!({})).expect("empty response ok");
        assert!(hits.is_empty());
    }
}
