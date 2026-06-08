//! S3 skill mirroring. Mirrors the per-agent + per-user + global
//! skill trees from the local disk to the workspace `Store` so
//! multi-pod deployments see the same installed skills.
//!
//! Key layout (matches ):
//!
//!   <owner>/skills/<skillName>/<relFile>
//!
//! Where `owner` is the agent ID, `_global` for the platform-wide
//! directory, or `_user_<uid>` for per-user skills.
//!
//! The workspace `Store` scopes every key by `(agent_id,
//! project_id, session_id)`. We map:
//!   - agent_id    → owner (agent ID, `_global`, or `_user_<uid>`)
//!   - project_id  → "skills" (so skills don't collide with
//!     workspaces / attachments / session data)
//!   - session_id  → "" (skills are session-independent)
//!   - key         → "<skillName>/<relFile>"
//!
//! `sync_skill_up` uploads every file under
//! `<root_dir>/<skill_name>/` to the workspace store.
//! `hydrate_skills_down` downloads them back to disk.
//! `mirror_skills_up` and `delete_skill_up` cover the per-skill
//! lifecycle from the CleanClaw reference.

use std::path::{Path, PathBuf};

use cleanclaw_workspace::Store;
use thiserror::Error;
use tokio::fs;
use tracing::{info, warn};

/// The "agent ID" used as the prefix for globally-installed skills.
/// Real agent names are validated to be lower alphanumeric + hyphens,
/// so the leading underscore keeps this namespace separate.
pub const GLOBAL_SKILL_OWNER: &str = "_global";
const USER_SKILL_OWNER_PREFIX: &str = "_user_";
const SKILLS_PROJECT: &str = "skills";

/// Returns the workspace scope key for a chatter's per-user skills.
/// Empty `user_id` returns "" so legacy / single-user installs can
/// short-circuit.
pub fn user_skill_owner(user_id: &str) -> String {
    if user_id.is_empty() {
        String::new()
    } else {
        format!("{USER_SKILL_OWNER_PREFIX}{user_id}")
    }
}

#[derive(Debug, Error)]
pub enum ObjectStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("workspace: {0}")]
    Workspace(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("invalid skill name: {0}")]
    InvalidSkillName(String),
}

pub type ObjectStoreResult<T> = Result<T, ObjectStoreError>;

/// Upload every file under `<root_dir>/<skill_name>/` to the
/// workspace store under `<owner>/skills/<skill_name>/`.
/// Symlinks are skipped (avoids duplicating targets).
/// Existing keys are overwritten. Returns the number of files
/// uploaded.
pub async fn sync_skill_up(
    ws: &dyn Store,
    owner: &str,
    skill_name: &str,
    root_dir: &Path,
) -> ObjectStoreResult<usize> {
    validate_skill_name(skill_name)?;
    let skill_dir = root_dir.join(skill_name);
    let info = fs::metadata(&skill_dir).await?;
    if !info.is_dir() {
        return Err(ObjectStoreError::NotADirectory(
            skill_dir.display().to_string(),
        ));
    }

    let mut uploaded = 0;
    let mut stack = vec![skill_dir.clone()];
    while let Some(dir) = stack.pop() {
        let mut rd = fs::read_dir(&dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if file_type.is_symlink() {
                continue;
            }
            let rel = path
                .strip_prefix(&skill_dir)
                .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
            let key = format!(
                "{}/{}",
                skill_name,
                rel.to_string_lossy().replace('\\', "/")
            );
            let bytes = fs::read(&path).await?;
            // `put` takes a `Bytes` and a content_type. Skills are
            // typically text/markdown or small text files; pass an
            // empty content_type and let the store pick the default.
            ws.put(
                owner,
                SKILLS_PROJECT,
                "",
                &key,
                bytes::Bytes::from(bytes),
                "",
            )
            .await
            .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
            uploaded += 1;
        }
    }
    info!(
        owner = %owner,
        skill = %skill_name,
        files = uploaded,
        "synced skill up to object store"
    );
    Ok(uploaded)
}

/// Download every object under `<owner>/skills/<skill_name>/` from
/// the workspace store into `<root_dir>/<skill_name>/`. Overwrites
/// existing local files. Returns the number of files written.
pub async fn hydrate_skills_down(
    ws: &dyn Store,
    owner: &str,
    skill_name: &str,
    root_dir: &Path,
) -> ObjectStoreResult<usize> {
    validate_skill_name(skill_name)?;
    let prefix = format!("{skill_name}/");
    let objs = ws
        .list(owner, SKILLS_PROJECT, "")
        .await
        .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
    let mut written = 0;
    for obj in objs {
        if !obj.path.starts_with(&prefix) {
            continue;
        }
        let bytes = ws
            .get(owner, SKILLS_PROJECT, "", &obj.path)
            .await
            .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
        let rel = obj.path.trim_start_matches(&prefix);
        let dst = root_dir.join(skill_name).join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&dst, &bytes).await?;
        written += 1;
    }
    info!(
        owner = %owner,
        skill = %skill_name,
        files = written,
        "hydrated skill down from object store"
    );
    Ok(written)
}

/// Convenience: `sync_skill_up` for a single skill. Same semantics —
/// returns the number of files uploaded.
pub async fn mirror_skills_up(
    ws: &dyn Store,
    owner: &str,
    skill_name: &str,
    root_dir: &Path,
) -> ObjectStoreResult<usize> {
    sync_skill_up(ws, owner, skill_name, root_dir).await
}

/// Remove every object under `<owner>/skills/<skill_name>/` from
/// the workspace store. Used on skill uninstall. Returns the
/// number of objects deleted.
pub async fn delete_skill_up(
    ws: &dyn Store,
    owner: &str,
    skill_name: &str,
) -> ObjectStoreResult<usize> {
    validate_skill_name(skill_name)?;
    let prefix = format!("{skill_name}/");
    let objs = ws
        .list(owner, SKILLS_PROJECT, "")
        .await
        .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
    let mut deleted = 0;
    for obj in objs {
        if !obj.path.starts_with(&prefix) {
            continue;
        }
        ws.delete(owner, SKILLS_PROJECT, "", &obj.path)
            .await
            .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
        deleted += 1;
    }
    info!(
        owner = %owner,
        skill = %skill_name,
        files = deleted,
        "deleted skill from object store"
    );
    Ok(deleted)
}

/// List every skill name installed under `<owner>/skills/`.
/// Returns the set of distinct `<skill_name>` prefixes — one entry
/// per skill bundle, regardless of how many files each contains.
pub async fn list_skill_names(ws: &dyn Store, owner: &str) -> ObjectStoreResult<Vec<String>> {
    let objs = ws
        .list(owner, SKILLS_PROJECT, "")
        .await
        .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
    let mut names: Vec<String> = objs
        .into_iter()
        .filter_map(|o| o.path.split('/').next().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Hydrate a list of skills by name. Equivalent to calling
/// `hydrate_skills_down` once per name. Used at boot to
/// restore per-agent skill trees.
pub async fn hydrate_many(
    ws: &dyn Store,
    owner: &str,
    names: &[String],
    root_dir: &Path,
) -> ObjectStoreResult<usize> {
    let mut total = 0;
    for n in names {
        match hydrate_skills_down(ws, owner, n, root_dir).await {
            Ok(n) => total += n,
            Err(e) => warn!(error = %e, skill = %n, "hydrate_many: skill hydrate failed"),
        }
    }
    Ok(total)
}

/// Same as `sync_skill_up` but takes an explicit file list
/// instead of walking the directory. Used when the caller
/// already has a list of files (e.g. from a tarball extract).
pub async fn sync_files_up(
    ws: &dyn Store,
    owner: &str,
    skill_name: &str,
    files: &[(PathBuf, Vec<u8>)],
) -> ObjectStoreResult<usize> {
    validate_skill_name(skill_name)?;
    let mut uploaded = 0;
    for (rel, bytes) in files {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let key = format!("{skill_name}/{rel_str}");
        // Defense-in-depth: even though `validate_skill_name`
        // already rejected the skill name, a path-traversal in
        // `rel_str` could still escape. Strip leading ".." and
        // absolute path prefixes here.
        if rel_str.contains("..") || rel_str.starts_with('/') {
            return Err(ObjectStoreError::Workspace(format!(
                "unsafe path in skill file: {rel_str}"
            )));
        }
        ws.put(
            owner,
            SKILLS_PROJECT,
            "",
            &key,
            bytes::Bytes::from(bytes.clone()),
            "",
        )
        .await
        .map_err(|e| ObjectStoreError::Workspace(e.to_string()))?;
        uploaded += 1;
    }
    info!(
        owner = %owner,
        skill = %skill_name,
        files = uploaded,
        "synced skill files up to object store"
    );
    Ok(uploaded)
}

fn validate_skill_name(name: &str) -> ObjectStoreResult<()> {
    if name.is_empty() {
        return Err(ObjectStoreError::InvalidSkillName("(empty)".into()));
    }
    if name.contains('/') || name.contains("..") || name.starts_with('.') {
        return Err(ObjectStoreError::InvalidSkillName(name.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_skill_owner_is_constant() {
        // Real agent names are alphanumeric + hyphens; the leading
        // underscore keeps this namespace separate.
        assert_eq!(GLOBAL_SKILL_OWNER, "_global");
        assert!(GLOBAL_SKILL_OWNER.starts_with('_'));
    }

    #[test]
    fn user_skill_owner_prefixes_with_underscore_user() {
        assert_eq!(user_skill_owner("u1"), "_user_u1");
        assert_eq!(user_skill_owner(""), "");
    }

    #[test]
    fn validate_skill_name_accepts_normal_names() {
        assert!(validate_skill_name("web-search").is_ok());
        assert!(validate_skill_name("skill_creator").is_ok());
        assert!(validate_skill_name("data-analysis").is_ok());
    }

    #[test]
    fn validate_skill_name_rejects_path_traversal() {
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("../etc/passwd").is_err());
        assert!(validate_skill_name("foo/bar").is_err());
        assert!(validate_skill_name(".hidden").is_err());
    }

    #[test]
    fn skills_project_constant() {
        // The project_id under which every skill object lives.
        // The CleanClaw reference uses a `skills/` key prefix; we
        // get the same namespace separation via the workspace
        // triple (agent_id, project_id, session_id).
        assert_eq!(SKILLS_PROJECT, "skills");
    }

    #[tokio::test]
    async fn user_skill_owner_empty_short_circuits() {
        // Calling user_skill_owner("") returns "" — callers can
        // gate on this to skip per-user install paths on
        // single-user installs.
        let owner = user_skill_owner("");
        assert!(owner.is_empty());
    }

    #[test]
    fn owner_prefixes_dont_collide_with_real_names() {
        // Real agent names are validated to be lower-alphanumeric
        // + hyphens — a leading underscore is never valid.
        assert!(!GLOBAL_SKILL_OWNER.chars().next().unwrap().is_alphanumeric());
        assert!(user_skill_owner("alice").starts_with("_user_"));
    }

    mod integration {
        use super::*;
        use cleanclaw_workspace::LocalFs;
        use tempfile::tempdir;

        fn workspace_store(root: &Path) -> std::sync::Arc<LocalFs> {
            std::sync::Arc::new(LocalFs::new(root))
        }

        #[tokio::test]
        async fn sync_then_hydrate_round_trip() {
            let src_dir = tempdir().unwrap();
            let dst_dir = tempdir().unwrap();
            let ws_dir = tempdir().unwrap();
            let ws = workspace_store(ws_dir.path());

            // Build a skill on disk in src_dir.
            let skill_root = src_dir.path();
            let skill_dir = skill_root.join("web-search");
            std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                b"---\nname: web-search\n---\nBody",
            )
            .unwrap();
            std::fs::write(skill_dir.join("scripts/run.sh"), b"#!/bin/sh\necho hi").unwrap();

            // Upload.
            let n = sync_skill_up(&*ws, "u1", "web-search", skill_root)
                .await
                .unwrap();
            assert_eq!(n, 2);

            // Hydrate into a fresh dst_dir and verify both files
            // are byte-identical.
            let n = hydrate_skills_down(&*ws, "u1", "web-search", dst_dir.path())
                .await
                .unwrap();
            assert_eq!(n, 2);
            let got = std::fs::read(dst_dir.path().join("web-search/SKILL.md")).unwrap();
            assert_eq!(got, b"---\nname: web-search\n---\nBody");
            let got = std::fs::read(dst_dir.path().join("web-search/scripts/run.sh")).unwrap();
            assert_eq!(got, b"#!/bin/sh\necho hi");
        }

        #[tokio::test]
        async fn delete_skill_removes_only_that_skill() {
            let ws_dir = tempdir().unwrap();
            let ws = workspace_store(ws_dir.path());

            // Upload two skills.
            for skill in &["web-search", "data-analysis"] {
                sync_files_up(
                    &*ws,
                    "u1",
                    skill,
                    &[(PathBuf::from("SKILL.md"), b"body".to_vec())],
                )
                .await
                .unwrap();
            }
            assert_eq!(
                list_skill_names(&*ws, "u1").await.unwrap(),
                vec!["data-analysis".to_string(), "web-search".to_string()]
            );

            // Delete one.
            let n = delete_skill_up(&*ws, "u1", "web-search").await.unwrap();
            assert_eq!(n, 1);
            assert_eq!(
                list_skill_names(&*ws, "u1").await.unwrap(),
                vec!["data-analysis".to_string()]
            );
        }

        #[tokio::test]
        async fn hydrate_skips_other_skills_files() {
            // When the workspace contains two skills under the
            // same owner, hydrate_skills_down for one of them
            // must not touch the other.
            let ws_dir = tempdir().unwrap();
            let dst_dir = tempdir().unwrap();
            let ws = workspace_store(ws_dir.path());

            sync_files_up(
                &*ws,
                "u1",
                "web-search",
                &[(PathBuf::from("SKILL.md"), b"ws body".to_vec())],
            )
            .await
            .unwrap();
            sync_files_up(
                &*ws,
                "u1",
                "data-analysis",
                &[(PathBuf::from("SKILL.md"), b"da body".to_vec())],
            )
            .await
            .unwrap();

            let n = hydrate_skills_down(&*ws, "u1", "web-search", dst_dir.path())
                .await
                .unwrap();
            assert_eq!(n, 1);
            // Only web-search is on disk; data-analysis is not.
            assert!(dst_dir.path().join("web-search/SKILL.md").exists());
            assert!(!dst_dir.path().join("data-analysis").exists());
        }

        #[tokio::test]
        async fn sync_files_up_rejects_path_traversal() {
            let ws_dir = tempdir().unwrap();
            let ws = workspace_store(ws_dir.path());
            let r = sync_files_up(
                &*ws,
                "u1",
                "web-search",
                &[(PathBuf::from("../etc/passwd"), b"pwned".to_vec())],
            )
            .await;
            assert!(r.is_err());
        }

        #[tokio::test]
        async fn hydrate_many_swallows_per_skill_errors() {
            // A bad skill name in the list should be logged but
            // not block the rest.
            let ws_dir = tempdir().unwrap();
            let dst_dir = tempdir().unwrap();
            let ws = workspace_store(ws_dir.path());
            sync_files_up(
                &*ws,
                "u1",
                "web-search",
                &[(PathBuf::from("SKILL.md"), b"ok".to_vec())],
            )
            .await
            .unwrap();
            let n = hydrate_many(
                &*ws,
                "u1",
                &[
                    "web-search".to_string(),
                    "../evil".to_string(), // rejected
                ],
                dst_dir.path(),
            )
            .await
            .unwrap();
            // web-search still hydrated despite the bad entry.
            assert!(n >= 1);
        }
    }
}
