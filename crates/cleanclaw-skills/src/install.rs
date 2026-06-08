//! Skill installers.
//!
//! Three source kinds are supported today:
//!   - **ClawHub** (`clawhub.ai`): registry-style install by slug
//!   - **GitHub**: `owner/repo` (with optional subpath) → download
//!     the codeload tarball, extract
//!   - **Local tarball**: a `.tar.gz` already on disk
//!
//! Plus a path-only `install_local_folder` for `install …
//! --from-path <dir>` CLI invocations. Objectstore (S3) and
//! skills.sh sources are stubbed and out of scope.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tar::Archive;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("path traversal blocked: {0}")]
    Traversal(String),
    #[error("not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub source: String,
    pub name: String,
    pub version: String,
    pub installed_at: PathBuf,
    pub files_written: usize,
}

const CLAWHUB_BASE_URL: &str = "https://clawhub.ai";

fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("cleanclaw/1.0")
        .timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client")
}

fn sanitize_path_component(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Install a skill from ClawHub by slug.
pub async fn install_from_clawhub(
    client: Arc<reqwest::Client>,
    slug: &str,
    target_dir: &Path,
) -> std::result::Result<InstallResult, InstallError> {
    if slug.is_empty() {
        return Err(InstallError::Invalid("slug required".into()));
    }
    let safe_slug = sanitize_path_component(slug);
    // Fetch metadata to discover the latest version + tarball URL.
    let meta_url = format!("{CLAWHUB_BASE_URL}/api/v1/skills/{slug}");
    let meta: SkillInfo = client
        .get(&meta_url)
        .send()
        .await
        .map_err(|e| InstallError::Http(e.to_string()))?
        .error_for_status()
        .map_err(|e| InstallError::Http(e.to_string()))?
        .json()
        .await
        .map_err(|e| InstallError::Http(e.to_string()))?;
    let tarball_url = if !meta.tarball_url.is_empty() {
        meta.tarball_url.clone()
    } else {
        format!("{CLAWHUB_BASE_URL}/api/v1/skills/{slug}/download")
    };
    let bytes = client
        .get(&tarball_url)
        .send()
        .await
        .map_err(|e| InstallError::Http(e.to_string()))?
        .error_for_status()
        .map_err(|e| InstallError::Http(e.to_string()))?
        .bytes()
        .await
        .map_err(|e| InstallError::Http(e.to_string()))?;
    let dest = target_dir.join(&safe_slug);
    // ClawHub tarballs are .tar.gz; reuse the tar extractor.
    let n = extract_tar_gz(&bytes, "", &dest)?;
    Ok(InstallResult {
        source: "clawhub".into(),
        name: safe_slug,
        version: meta.version,
        installed_at: dest,
        files_written: n,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub downloads: i64,
    #[serde(rename = "tarballUrl", default)]
    pub tarball_url: String,
}

/// Install a skill from a GitHub repo (e.g. `owner/repo`). If
/// `skill_name` is empty, the whole repo is treated as the skill;
/// otherwise we look for a top-level `<repo>/<skill_name>/` directory
/// in the tarball.
pub async fn install_from_github(
    client: Arc<reqwest::Client>,
    repo: &str,
    skill_name: &str,
    target_dir: &Path,
) -> std::result::Result<InstallResult, InstallError> {
    let parts: Vec<&str> = repo.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(InstallError::Invalid(format!(
            "repo must be owner/repo, got {repo}"
        )));
    }
    let (owner, name) = (parts[0], parts[1]);
    let mut last_err: Option<InstallError> = None;
    for branch in ["main", "master"] {
        let tar_url =
            format!("https://codeload.github.com/{owner}/{name}/tar.gz/refs/heads/{branch}");
        let resp = client
            .get(&tar_url)
            .send()
            .await
            .map_err(|e| InstallError::Http(e.to_string()));
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = Some(InstallError::Http(format!("HTTP {}", resp.status())));
            continue;
        }
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                last_err = Some(InstallError::Http(e.to_string()));
                continue;
            }
        };
        let installed_name = if skill_name.is_empty() {
            sanitize_path_component(name)
        } else {
            sanitize_path_component(skill_name)
        };
        let subpath = if skill_name.is_empty() {
            String::new()
        } else {
            format!("{name}/{skill_name}")
        };
        let dest = target_dir.join(&installed_name);
        let n = extract_tar_gz(&bytes, &subpath, &dest)?;
        return Ok(InstallResult {
            source: "github".into(),
            name: installed_name,
            version: branch.to_string(),
            installed_at: dest,
            files_written: n,
        });
    }
    Err(last_err.unwrap_or_else(|| InstallError::NotFound(repo.to_string())))
}

/// Install a skill from a local .tar.gz file.
pub async fn install_from_tarball(
    path: &Path,
    skill_name: &str,
    target_dir: &Path,
) -> std::result::Result<InstallResult, InstallError> {
    let data = tokio::fs::read(path).await?;
    let installed_name = if skill_name.is_empty() {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("skill")
            .to_string()
    } else {
        sanitize_path_component(skill_name)
    };
    let dest = target_dir.join(&installed_name);
    let n = extract_tar_gz(&data, "", &dest)?;
    Ok(InstallResult {
        source: "tarball".into(),
        name: installed_name,
        version: String::new(),
        installed_at: dest,
        files_written: n,
    })
}

/// Install from a local directory (already on disk). No download.
pub async fn install_from_path(
    src: &Path,
    skill_name: &str,
    target_dir: &Path,
) -> std::result::Result<InstallResult, InstallError> {
    if !src.is_dir() {
        return Err(InstallError::NotFound(src.display().to_string()));
    }
    let installed_name = if skill_name.is_empty() {
        src.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("skill")
            .to_string()
    } else {
        sanitize_path_component(skill_name)
    };
    let dest = target_dir.join(&installed_name);
    copy_dir_recursive(src, &dest).await?;
    let n = count_files(&dest).await?;
    Ok(InstallResult {
        source: "path".into(),
        name: installed_name,
        version: String::new(),
        installed_at: dest,
        files_written: n,
    })
}

fn extract_tar_gz(data: &[u8], subpath: &str, dest: &Path) -> Result<usize, InstallError> {
    let gz = GzDecoder::new(data);
    let mut archive = Archive::new(gz);
    std::fs::create_dir_all(dest)?;
    let mut count = 0;
    let subpath = subpath.trim_end_matches('/');
    for entry in archive.entries().map_err(|e| InstallError::Io(e.into()))? {
        let mut entry = entry.map_err(|e| InstallError::Io(e.into()))?;
        let path = entry
            .path()
            .map_err(|e| InstallError::Io(e.into()))?
            .into_owned();
        // Top-level dir is the repo name (e.g. "repo-SHA/..."); strip
        // it. Then optionally strip the requested subpath.
        let stripped = {
            // Build the prefix to strip: the first path component.
            let first = match path.components().next() {
                Some(c) => c.as_os_str().to_owned(),
                None => continue,
            };
            let prefix: std::path::PathBuf = [&first].iter().collect();
            match path.as_path().strip_prefix(&prefix) {
                Ok(rest) => rest.to_string_lossy().to_string(),
                Err(_) => continue,
            }
        };
        if !subpath.is_empty() {
            let candidate = format!("{subpath}/");
            if !stripped.starts_with(&candidate) {
                continue;
            }
            let final_path = &stripped[candidate.len()..];
            if final_path.is_empty() {
                continue;
            }
            let out = match safe_join(dest, final_path) {
                Some(p) => p,
                None => continue,
            };
            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&out)?;
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::io::copy(&mut entry, &mut std::fs::File::create(&out)?)?;
                count += 1;
            }
        } else {
            if stripped.is_empty() {
                continue;
            }
            let out = match safe_join(dest, &stripped) {
                Some(p) => p,
                None => continue,
            };
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

fn strip_first_component(p: &std::path::Path) -> std::path::PathBuf {
    p.components().skip(1).collect()
}

fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let rel = std::path::Path::new(rel);
    if rel.is_absolute() {
        return None;
    }
    let mut out = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            std::path::Component::Normal(_) => out.push(comp),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None, // traversal
            _ => return None,
        }
    }
    Some(out)
}

async fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), InstallError> {
    tokio::fs::create_dir_all(dest).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let ft = entry.file_type().await?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if ft.is_dir() {
            Box::pin(copy_dir_recursive(&from, &to)).await?;
        } else {
            tokio::fs::copy(&from, &to).await?;
        }
    }
    Ok(())
}

async fn count_files(dir: &Path) -> Result<usize, InstallError> {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&d).await?;
        while let Some(e) = entries.next_entry().await? {
            if e.file_type().await?.is_dir() {
                stack.push(e.path());
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_path_separators() {
        assert_eq!(sanitize_path_component("foo/bar"), "foo_bar");
        assert_eq!(sanitize_path_component("foo bar"), "foo_bar");
        assert_eq!(sanitize_path_component("a.b-c_d"), "a.b-c_d");
    }

    #[test]
    fn safe_join_blocks_traversal() {
        assert!(safe_join(Path::new("/tmp"), "..").is_none());
        assert!(safe_join(Path::new("/tmp"), "/etc/passwd").is_none());
        assert!(safe_join(Path::new("/tmp"), "a/../b").is_none());
        assert!(safe_join(Path::new("/tmp"), "a/b").is_some());
    }

    #[test]
    fn safe_join_blocks_absolute() {
        let out = safe_join(Path::new("/tmp"), "/abs");
        assert!(out.is_none());
    }

    #[test]
    fn strip_first_component_works() {
        let p = std::path::Path::new("repo-sha/sub/file.txt");
        let stripped = strip_first_component(p);
        assert_eq!(stripped, std::path::Path::new("sub/file.txt"));
    }

    #[tokio::test]
    async fn install_from_path_copies_directory() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        tokio::fs::create_dir_all(&src).await.unwrap();
        tokio::fs::write(src.join("a.txt"), b"hello").await.unwrap();
        tokio::fs::create_dir_all(src.join("sub")).await.unwrap();
        tokio::fs::write(src.join("sub/b.txt"), b"world")
            .await
            .unwrap();

        let target = dir.path().join("target");
        let r = install_from_path(&src, "", &target).await.unwrap();
        assert_eq!(r.source, "path");
        assert!(r.installed_at.exists());
        assert!(r.files_written >= 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn install_from_path_rejects_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = install_from_path(&dir.path().join("nope"), "", &dir.path().join("t"))
            .await
            .unwrap_err();
        assert!(matches!(err, InstallError::NotFound(_)));
    }

    #[tokio::test]
    async fn install_from_tarball_extracts_files() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use tar::Builder;
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("skill.tar.gz");
        let f = std::fs::File::create(&tar_path).unwrap();
        let gz = GzEncoder::new(f, Compression::default());
        let mut b = Builder::new(gz);
        let data = b"hello world";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        b.append_data(&mut header, "skill-SHA/file.txt", &data[..])
            .unwrap();
        let gz = b.into_inner().unwrap();
        let _ = gz.finish().unwrap();
        let target = dir.path().join("out");
        let r = install_from_tarball(&tar_path, "", &target).await.unwrap();
        assert_eq!(r.source, "tarball");
        // Either a top-level dir is preserved or files are flat-extracted.
        // The Go behavior flattens by stripping the top component.
        let found = std::fs::read_dir(&target)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().join("file.txt").exists() || e.path().ends_with("file.txt"));
        assert!(found, "no file.txt found in {target:?}");
    }

    #[tokio::test]
    async fn install_from_github_validates_repo_format() {
        let client = Arc::new(default_http_client());
        let dir = tempfile::tempdir().unwrap();
        let err = install_from_github(client, "no-slash", "", &dir.path().join("t"))
            .await
            .unwrap_err();
        assert!(matches!(err, InstallError::Invalid(_)));
    }

    #[tokio::test]
    async fn install_from_clawhub_requires_slug() {
        let client = Arc::new(default_http_client());
        let dir = tempfile::tempdir().unwrap();
        let err = install_from_clawhub(client, "", &dir.path().join("t"))
            .await
            .unwrap_err();
        assert!(matches!(err, InstallError::Invalid(_)));
    }

    #[test]
    fn result_serializes() {
        let r = InstallResult {
            source: "github".into(),
            name: "demo".into(),
            version: "main".into(),
            installed_at: PathBuf::from("/tmp/demo"),
            files_written: 3,
        };
        let blob = serde_json::to_string(&r).unwrap();
        assert!(blob.contains("\"source\":\"github\""));
        assert!(blob.contains("\"files_written\":3"));
    }
}
