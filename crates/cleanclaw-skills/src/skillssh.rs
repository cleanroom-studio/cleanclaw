//! skills.sh registry backend.
//!
//! skills.sh is a public search-only registry: it indexes skill
//! folders hosted in arbitrary GitHub repos and returns the
//! source `<owner>/<repo>` + the in-repo subpath. The actual
//! download goes through codeload.github.com.
//!
//! Pipeline:
//!   1. `search_skills_sh(q)` — GET skills.sh/api/search?q=<q>
//!   2. `pick_skills_sh_exact(results, name)` — pick the best match
//!      (exact `skillId` wins; otherwise most-installed)
//!   3. `install_from_skills_sh(r, target_dir)` — probe the source
//!      repo's tarball (main → master → API default branch) for the
//!      in-tarball subpath of the skill folder, then extract.
//!
//! The probe is cheap: skills.sh skills live at arbitrary depths
//! inside the source repo, so we can't hardcode a path. We scan
//! the tarball streaming until we find `<topdir>/<...>/<skillId>/SKILL.md`.
//!
//! All HTTP calls share a 60s timeout client. Errors at the
//! search/probe stage are silent no-ops at the call site
//! (callers can fall through to the local-installed list).

use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tar::Archive;
use thiserror::Error;

const SKILLS_SH_BASE_URL: &str = "https://skills.sh";

fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("cleanclaw/1.0")
        .timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client")
}

#[derive(Debug, Error)]
pub enum SkillsShError {
    #[error("http: {0}")]
    Http(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShResult {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub source: String,
    pub installs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkill {
    pub name: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    skills: Vec<SkillsShResult>,
}

/// Search the skills.sh public search endpoint. Returns the raw
/// results — caller is expected to call `pick_skills_sh_exact`
/// to find the best match.
pub async fn search_skills_sh(query: &str) -> Result<Vec<SkillsShResult>, SkillsShError> {
    let client = default_http_client();
    let url = format!("{}/api/search?q={}", SKILLS_SH_BASE_URL, urlencoded(query));
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| SkillsShError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(SkillsShError::Http(format!("HTTP {}", resp.status())));
    }
    let body: SearchResponse = resp
        .json()
        .await
        .map_err(|e| SkillsShError::Decode(e.to_string()))?;
    Ok(body.skills)
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '/' => "%2F".to_string(),
            '?' => "%3F".to_string(),
            '&' => "%26".to_string(),
            '=' => "%3D".to_string(),
            '+' => "%2B".to_string(),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// Pick the result that best matches `name`. Exact `skill_id`
/// match wins; otherwise the most-installed entry. Returns
/// `None` when results is empty.
pub fn pick_skills_sh_exact(results: &[SkillsShResult], name: &str) -> Option<SkillsShResult> {
    if results.is_empty() {
        return None;
    }
    // Exact skillId match wins outright.
    for r in results {
        if r.skill_id == name {
            return Some(r.clone());
        }
    }
    // Otherwise, the most-installed entry.
    results.iter().max_by_key(|r| r.installs).cloned()
}

/// Find the in-tarball subpath of a skill folder by scanning a
/// gzipped tarball stream for `<topdir>/<...>/<skillID>/SKILL.md`.
/// Returns the path relative to the top-level dir, or `""` if
/// not found.
fn find_skill_dir_in_tarball(data: &[u8], skill_id: &str) -> Result<String, SkillsShError> {
    let gz = GzDecoder::new(data);
    let mut archive = Archive::new(gz);
    let suffix = format!("/{}/SKILL.md", skill_id);
    for entry in archive.entries().map_err(SkillsShError::Io)? {
        let entry = entry.map_err(SkillsShError::Io)?;
        let path = entry
            .path()
            .map_err(SkillsShError::Io)?
            .into_owned();
        let path_str = path.to_string_lossy().to_string();
        // Strip the top-level "<repo>-<sha>/" prefix.
        let first = match path.components().next() {
            Some(c) => c.as_os_str().to_string_lossy().to_string(),
            None => continue,
        };
        let stripped = match path_str.strip_prefix(&first) {
            Some(s) => s.trim_start_matches('/'),
            None => continue,
        };
        if stripped.ends_with(&suffix) || stripped == format!("{}/SKILL.md", skill_id) {
            // Return the directory part (everything before /SKILL.md).
            if let Some(idx) = stripped.rfind("/SKILL.md") {
                return Ok(stripped[..idx].to_string());
            }
        }
    }
    Ok(String::new())
}

/// Install a skills.sh result into `<target_dir>/<skillID>/`.
/// Mirrors the Go `InstallFromSkillsSh` — fetches the source
/// repo's tarball (trying main → master → GitHub API default
/// branch), probes the tarball to discover the in-tarball
/// subpath of the skill folder, and extracts just that folder.
pub async fn install_from_skills_sh(
    result: &SkillsShResult,
    target_dir: &Path,
) -> Result<InstalledSkill, SkillsShError> {
    if result.skill_id.is_empty() || result.source.is_empty() {
        return Err(SkillsShError::Invalid(
            "skills.sh result missing skillId/source".into(),
        ));
    }
    let parts: Vec<&str> = result.source.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(SkillsShError::Invalid(format!(
            "skills.sh source {:?} is not owner/repo",
            result.source
        )));
    }
    let owner = parts[0];
    let mut repo = parts[1];
    // The "source" field sometimes contains a repo-internal
    // subpath appended to owner/repo (e.g. "claude-office-skills/skills").
    // GitHub repos only have two-segment slugs, so split again
    // and treat the rest as a prefix hint.
    let prefix_hint = if let Some(idx) = repo.find('/') {
        let hint = repo[idx + 1..].to_string();
        repo = &repo[..idx];
        hint
    } else {
        String::new()
    };

    let client = default_http_client();
    // Build the ref list. Try API default branch first, then
    // main / master as fallbacks.
    let mut refs: Vec<String> = vec!["main".into(), "master".into()];
    if let Ok(default_branch) = github_default_branch(&client, owner, repo).await {
        if !default_branch.is_empty() && default_branch != "main" && default_branch != "master" {
            refs.insert(0, default_branch);
        }
    }

    let mut last_err: Option<SkillsShError> = None;
    for r in &refs {
        let tar_url = format!(
            "https://codeload.github.com/{}/{}/tar.gz/refs/heads/{}",
            owner, repo, r
        );
        let resp = client
            .get(&tar_url)
            .send()
            .await
            .map_err(|e| SkillsShError::Http(e.to_string()));
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = Some(SkillsShError::Http(format!("HTTP {}", resp.status())));
            continue;
        }
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                last_err = Some(SkillsShError::Http(e.to_string()));
                continue;
            }
        };
        // Probe the tarball to find the in-tarball subpath of the
        // skill folder.
        let subpath = match find_skill_dir_in_tarball(&bytes, &result.skill_id) {
            Ok(s) if !s.is_empty() => s,
            _ if !prefix_hint.is_empty() => format!("{}/{}", prefix_hint, result.skill_id),
            _ => {
                last_err = Some(SkillsShError::NotFound(format!(
                    "skill {:?} not found in {}/{}@{}",
                    result.skill_id, owner, repo, r
                )));
                continue;
            }
        };
        let dest = target_dir.join(&result.skill_id);
        let n = extract_skill_from_tar(&bytes, &subpath, &dest)?;
        if n == 0 {
            last_err = Some(SkillsShError::NotFound(format!(
                "extracted no files from {} (subpath {:?})",
                tar_url, subpath
            )));
            continue;
        }
        return Ok(InstalledSkill {
            name: result.skill_id.clone(),
            dir: dest,
        });
    }
    Err(last_err.unwrap_or_else(|| SkillsShError::NotFound(format!("{}/{}", owner, repo))))
}

/// Ask the GitHub API for the repo's default branch. Returns
/// `""` on any error (rate limit, private repo, etc.) — callers
/// fall back to the well-known conventions. Best-effort only.
async fn github_default_branch(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<String, SkillsShError> {
    let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| SkillsShError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Ok(String::new());
    }
    #[derive(Deserialize)]
    struct Body {
        default_branch: String,
    }
    let body: Body = resp
        .json()
        .await
        .map_err(|e| SkillsShError::Decode(e.to_string()))?;
    Ok(body.default_branch)
}

/// Extract a single subpath from a gzipped tarball (downloads
/// once; the tar reader streams the entries).
fn extract_skill_from_tar(data: &[u8], subpath: &str, dest: &Path) -> Result<usize, SkillsShError> {
    let gz = GzDecoder::new(data);
    let mut archive = Archive::new(gz);
    std::fs::create_dir_all(dest)?;
    let subpath = subpath.trim_end_matches('/');
    let mut count = 0;
    for entry in archive.entries().map_err(SkillsShError::Io)? {
        let mut entry = entry.map_err(SkillsShError::Io)?;
        let path = entry
            .path()
            .map_err(SkillsShError::Io)?
            .into_owned();
        let path_str = path.to_string_lossy().to_string();
        // Strip top-level dir.
        let first = match path.components().next() {
            Some(c) => c.as_os_str().to_string_lossy().to_string(),
            None => continue,
        };
        let stripped = match path_str.strip_prefix(&first) {
            Some(s) => s.trim_start_matches('/'),
            None => continue,
        };
        if !subpath.is_empty() {
            let prefix = format!("{}/", subpath);
            if !stripped.starts_with(&prefix) {
                continue;
            }
            let rel = &stripped[prefix.len()..];
            if rel.is_empty() {
                continue;
            }
            // Reject path-traversal.
            if rel.contains("..") {
                return Err(SkillsShError::Invalid(format!(
                    "tar entry {:?} escapes subpath",
                    rel
                )));
            }
            let out = dest.join(rel);
            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&out)?;
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::io::copy(&mut entry, &mut std::fs::File::create(&out)?)?;
                count += 1;
            }
        }
    }
    Ok(count)
}

/// List installed skills under `dir` — subdirs containing a
/// `SKILL.md` are reported.
pub fn list_installed(dir: &Path) -> Result<Vec<InstalledSkill>, SkillsShError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let skill_md = entry.path().join("SKILL.md");
        if skill_md.exists() {
            out.push(InstalledSkill {
                name: entry.file_name().to_string_lossy().to_string(),
                dir: entry.path(),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::Builder;

    #[test]
    fn pick_exact_match_wins() {
        let results = vec![
            SkillsShResult {
                id: "owner/repo/pdf".into(),
                skill_id: "pdf".into(),
                name: "PDF tools".into(),
                source: "owner/repo".into(),
                installs: 10,
            },
            SkillsShResult {
                id: "owner/repo/pdf2".into(),
                skill_id: "pdf2".into(),
                name: "PDF 2".into(),
                source: "owner/repo".into(),
                installs: 1000,
            },
        ];
        let picked = pick_skills_sh_exact(&results, "pdf").unwrap();
        assert_eq!(picked.skill_id, "pdf");
        let picked2 = pick_skills_sh_exact(&results, "missing").unwrap();
        assert_eq!(picked2.skill_id, "pdf2"); // most-installed
    }

    #[test]
    fn pick_empty_returns_none() {
        let empty: Vec<SkillsShResult> = vec![];
        assert!(pick_skills_sh_exact(&empty, "x").is_none());
    }

    #[test]
    fn urlencoded_handles_special_chars() {
        let q = urlencoded("foo bar");
        assert!(q.contains("%20"));
    }

    #[test]
    fn list_installed_finds_skill_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let skill_a = dir.path().join("a");
        std::fs::create_dir_all(&skill_a).unwrap();
        std::fs::write(skill_a.join("SKILL.md"), "---").unwrap();
        let skill_b = dir.path().join("b");
        std::fs::create_dir_all(&skill_b).unwrap();
        std::fs::write(skill_b.join("SKILL.md"), "---").unwrap();
        // c/ has no SKILL.md → not a skill.
        let not_skill = dir.path().join("c");
        std::fs::create_dir_all(&not_skill).unwrap();
        let list = list_installed(dir.path()).unwrap();
        let names: Vec<_> = list.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(!names.contains(&"c"));
    }

    #[test]
    fn list_installed_missing_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let list = list_installed(&dir.path().join("missing")).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn find_skill_dir_in_tarball_returns_subpath() {
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("t.tar.gz");
        let f = std::fs::File::create(&tar_path).unwrap();
        let gz = GzEncoder::new(f, Compression::default());
        let mut b = Builder::new(gz);
        let data = b"hi";
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        b.append_data(&mut h, "repo-SHA/skills/pdf/SKILL.md", &data[..])
            .unwrap();
        let gz = b.into_inner().unwrap();
        let _ = gz.finish().unwrap();
        let data = std::fs::read(&tar_path).unwrap();
        let sub = find_skill_dir_in_tarball(&data, "pdf").unwrap();
        assert_eq!(sub, "skills/pdf");
    }

    #[test]
    fn find_skill_dir_returns_empty_when_not_present() {
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("t.tar.gz");
        let f = std::fs::File::create(&tar_path).unwrap();
        let gz = GzEncoder::new(f, Compression::default());
        let mut b = Builder::new(gz);
        let data = b"hi";
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        b.append_data(&mut h, "repo-SHA/README.md", &data[..])
            .unwrap();
        let gz = b.into_inner().unwrap();
        let _ = gz.finish().unwrap();
        let data = std::fs::read(&tar_path).unwrap();
        let sub = find_skill_dir_in_tarball(&data, "pdf").unwrap();
        assert!(sub.is_empty());
    }
}
