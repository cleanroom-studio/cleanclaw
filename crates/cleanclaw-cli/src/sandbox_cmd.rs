//! `cleanclaw sandbox …` — manage sandboxed execution environments.
//!
//! The CLI is
//! a thin wrapper that shells out to `docker` for the runtime side
//! and updates the local config for the metadata.

use clap::Subcommand;
use cleanclaw_core::Result;
use std::process::Command;

#[derive(Subcommand)]
pub enum SandboxCmd {
    /// Create a standalone sandbox container (docker exec backend).
    Create {
        #[arg(long, default_value = "thinkany/CleanClaw-sandbox:latest")]
        image: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// List running sandbox containers.
    List,
    /// Attach an interactive shell to a running sandbox.
    Connect {
        #[arg(long)]
        name: String,
    },
    /// Stop and remove a sandbox.
    Destroy {
        #[arg(long)]
        name: String,
    },
}

pub async fn run(cmd: SandboxCmd) -> Result<()> {
    match cmd {
        SandboxCmd::Create { image, name } => create(image, name),
        SandboxCmd::List => list(),
        SandboxCmd::Connect { name } => connect(&name),
        SandboxCmd::Destroy { name } => destroy(&name),
    }
}

fn create(image: String, name: Option<String>) -> Result<()> {
    let resolved = resolve_image(&image);
    let container_name = name.unwrap_or_else(|| {
        format!(
            "cleanclaw-sandbox-{}",
            cleanclaw_core::IdGen::new().next("sb")
        )
    });
    let status = Command::new("docker")
        .args([
            "run",
            "-d",
            "--rm",
            "--name",
            &container_name,
            "-v",
            "cleanclaw-sandbox:/workspace",
            &resolved,
            "sleep",
            "infinity",
        ])
        .status()?;
    if !status.success() {
        return Err(cleanclaw_core::CleanClawError::Internal(format!(
            "docker run failed: {}",
            status
        )));
    }
    println!("created sandbox {container_name} (image={resolved})");
    Ok(())
}

fn list() -> Result<()> {
    let out = Command::new("docker")
        .args([
            "ps",
            "--filter",
            "name=cleanclaw-sandbox",
            "--format",
            "{{.Names}}\t{{.Image}}\t{{.Status}}",
        ])
        .output()?;
    if !out.status.success() {
        return Err(cleanclaw_core::CleanClawError::Internal(format!(
            "docker ps failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let body = String::from_utf8_lossy(&out.stdout);
    if body.trim().is_empty() {
        println!("(no sandboxes running)");
    } else {
        println!("{}", body.trim_end());
    }
    Ok(())
}

fn connect(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["exec", "-it", name, "/bin/sh"])
        .status()?;
    if !status.success() {
        return Err(cleanclaw_core::CleanClawError::Internal(format!(
            "docker exec failed: {}",
            status
        )));
    }
    Ok(())
}

fn destroy(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["stop", name])
        .status()?;
    if !status.success() {
        return Err(cleanclaw_core::CleanClawError::Internal(format!(
            "docker stop failed: {}",
            status
        )));
    }
    println!("destroyed sandbox {name}");
    Ok(())
}

/// Resolve the image to the configured `sandbox.image` when the
/// caller passes the default placeholder.
fn resolve_image(image: &str) -> String {
    if image == "thinkany/cleanclaw-sandbox:latest" {
        let env = cleanclaw_config::load_env();
        if !env.sandbox.image.is_empty() {
            return env.sandbox.image;
        }
    }
    image.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_image_passthrough() {
        let r = resolve_image("custom:tag");
        assert_eq!(r, "custom:tag");
    }
}
