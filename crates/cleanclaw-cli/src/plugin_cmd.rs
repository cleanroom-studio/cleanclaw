//! `cleanclaw plugin …` — list / install / remove plugins.
//!
//! Plugins are
//! external processes (python3, node, …) that speak JSON-RPC over
//! stdio. The CLI reads a `plugin.json` manifest and spawns the
//! command exactly as the Go daemon would.

use clap::Subcommand;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_plugin::Manifest;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum PluginCmd {
    /// List installed plugins under $CLEANCLAW_HOME/plugins.
    Ls,
    /// Install a plugin from a local directory, or — if no path is
    /// given — fetch the named plugin from the configured hub repo on
    /// GitHub (default `CleanClaw-ai/CleanClaw`).
    Install {
        name: String,
        /// Path to a directory containing `plugin.json` (and the entrypoint script).
        /// Omit to fetch from the hub repo's `plugins/<name>/` subtree.
        #[arg(default_value = "")]
        path: String,
    },
    /// Remove an installed plugin.
    Rm { name: String },
}

const HUB_REPO: &str = "cleanroom-studio/cleanclaw";

pub async fn run(cmd: PluginCmd) -> Result<()> {
    match cmd {
        PluginCmd::Ls => ls(),
        PluginCmd::Install { name, path } => {
            if path.is_empty() {
                install_from_hub(&name).await
            } else {
                install(&name, Path::new(&path))
            }
        }
        PluginCmd::Rm { name } => rm(&name),
    }
}

fn plugins_root() -> PathBuf {
    cleanclaw_config::env::home_dir().join("plugins")
}

fn ls() -> Result<()> {
    let root = plugins_root();
    if !root.exists() {
        println!("(no plugins — install one with `cleanclaw plugin install <name> <path>`)");
        return Ok(());
    }
    let mut found = false;
    for entry in std::fs::read_dir(&root)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("plugin.json");
        if !manifest_path.exists() {
            continue;
        }
        found = true;
        match load_manifest(&manifest_path) {
            Ok(m) => println!("{:<28} {:<10} {}", m.id, plugin_kind_str(&m), m.name),
            Err(e) => eprintln!("  ! {} (manifest error: {e})", path.display()),
        }
    }
    if !found {
        println!("(no plugins installed)");
    }
    Ok(())
}

fn install(name: &str, src: &Path) -> Result<()> {
    let manifest = src.join("plugin.json");
    if !manifest.exists() {
        return Err(CleanClawError::InvalidArgument(format!(
            "no plugin.json at {}",
            src.display()
        )));
    }
    let dest = plugins_root().join(name);
    if dest.exists() {
        return Err(CleanClawError::Conflict(format!(
            "plugin {name} already installed"
        )));
    }
    copy_dir_recursive(src, &dest)?;
    println!("installed plugin {name}");
    Ok(())
}

fn rm(name: &str) -> Result<()> {
    let dest = plugins_root().join(name);
    if !dest.exists() {
        return Err(CleanClawError::NotFound(format!("plugin {name}")));
    }
    std::fs::remove_dir_all(&dest)?;
    println!("removed plugin {name}");
    Ok(())
}

/// Fetch `<repo>/plugins/<name>/plugin.json` from GitHub's raw
/// content CDN. Mirrors the Go daemon's `cmd_plugin.go` install
/// path: when no local path is given, download the manifest and
/// entrypoint script from the upstream repo, then materialize a
/// installable directory under `$CLEANCLAW_HOME/plugins/<name>`.
//
/// Offline-only: when `CARGO_NET_OFFLINE=true` (or no `reqwest`
/// feature is compiled in), this returns a clear `InvalidArgument`
/// error so the caller can fall back to a local install.
async fn install_from_hub(name: &str) -> Result<()> {
    if !is_valid_plugin_name(name) {
        return Err(CleanClawError::InvalidArgument(format!(
            "invalid plugin name: {name}"
        )));
    }
    let url = format!(
        "https://raw.githubusercontent.com/{HUB_REPO}/main/plugins/{name}/plugin.json"
    );
    println!("fetching {url}");
    let manifest_bytes = match fetch_url(&url).await {
        Ok(b) => b,
        Err(e) => {
            println!(
                "(hub install unavailable — install a local copy with `cleanclaw plugin install {name} <path>`)"
            );
            return Err(e);
        }
    };
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| CleanClawError::InvalidArgument(format!("bad manifest: {e}")))?;
    let dest = plugins_root().join(name);
    if dest.exists() {
        return Err(CleanClawError::Conflict(format!(
            "plugin {name} already installed"
        )));
    }
    std::fs::create_dir_all(&dest)?;
    std::fs::write(dest.join("plugin.json"), &manifest_bytes)?;
    // Try to fetch the entrypoint script referenced in the manifest's
    // `command` field. The Go version does this; we do too as a
    // best-effort.
    if !manifest.command.is_empty() {
        let cmd_name = manifest.command.split_whitespace().next().unwrap_or("");
        if !cmd_name.is_empty() && !cmd_name.contains('/') && !cmd_name.contains("..") {
            let script_url = format!(
                "https://raw.githubusercontent.com/{HUB_REPO}/main/plugins/{name}/{cmd_name}"
            );
            if let Ok(script_bytes) = fetch_url(&script_url).await {
                let script_dest = dest.join(cmd_name);
                if let Err(e) = std::fs::write(&script_dest, &script_bytes) {
                    eprintln!("warn: failed to write {}: {e}", script_dest.display());
                } else {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perm = std::fs::metadata(&script_dest)
                            .map_err(|e| CleanClawError::Internal(format!("perm: {e}")))?
                            .permissions();
                        perm.set_mode(0o755);
                        std::fs::set_permissions(&script_dest, perm)
                            .map_err(|e| CleanClawError::Internal(format!("chmod: {e}")))?;
                    }
                }
            }
        }
    }
    println!("installed plugin {name} (from {HUB_REPO})");
    Ok(())
}

fn is_valid_plugin_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && s.len() <= 64
}

async fn fetch_url(url: &str) -> Result<Vec<u8>> {
    // Minimal HTTP GET. Avoids pulling in reqwest to keep the offline
    // build lean — falls through to a clear "not supported in this
    // build" message when the `reqwest` feature isn't on.
    #[cfg(feature = "http")]
    {
        let resp = reqwest::get(url)
            .await
            .map_err(|e| CleanClawError::Internal(format!("fetch {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(CleanClawError::Internal(format!(
                "fetch {url} returned {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| CleanClawError::Internal(format!("read {url}: {e}")))?;
        Ok(bytes.to_vec())
    }
    #[cfg(not(feature = "http"))]
    {
        let _ = url;
        Err(CleanClawError::NotImplemented(
            "hub install requires the `http` feature on cleanclaw-cli".into(),
        ))
    }
}

fn load_manifest(path: &Path) -> Result<Manifest> {
    let raw = std::fs::read_to_string(path)?;
    let m: Manifest = serde_json::from_str(&raw)?;
    Ok(m)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
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

/// `HubRepo` is the GitHub repo `cleanclaw plugin install <name>`
/// defaults to when the path is omitted.
pub fn hub_repo() -> &'static str {
    HUB_REPO
}

fn plugin_kind_str(m: &Manifest) -> &'static str {
    use cleanclaw_plugin::PluginType::*;
    match m.plugin_type {
        Channel => "channel",
        Tool => "tool",
        Provider => "provider",
        Hook => "hook",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_repo_default() {
        assert_eq!(hub_repo(), "cleanroom-studio/cleanclaw");
    }

    #[test]
    fn valid_plugin_names() {
        assert!(is_valid_plugin_name("telegram"));
        assert!(is_valid_plugin_name("my-plugin_v2"));
        assert!(is_valid_plugin_name("a"));
    }

    #[test]
    fn invalid_plugin_names() {
        assert!(!is_valid_plugin_name(""));
        assert!(!is_valid_plugin_name("../etc/passwd"));
        assert!(!is_valid_plugin_name("foo/bar"));
        assert!(!is_valid_plugin_name("foo bar"));
        assert!(!is_valid_plugin_name("foo.bar"));
        let s = "a".repeat(65);
        assert!(!is_valid_plugin_name(&s));
    }

    #[test]
    fn hub_url_format() {
        let name = "demo";
        let url = format!(
            "https://raw.githubusercontent.com/{HUB_REPO}/main/plugins/{name}/plugin.json"
        );
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/cleanroom-studio/cleanclaw/main/plugins/demo/plugin.json"
        );
    }

    #[test]
    fn hub_url_rejects_path_traversal() {
        // is_valid_plugin_name is the gate; traversal can't get past it.
        assert!(!is_valid_plugin_name("../demo"));
        assert!(!is_valid_plugin_name(".."));
        assert!(!is_valid_plugin_name("/etc/passwd"));
    }
}
