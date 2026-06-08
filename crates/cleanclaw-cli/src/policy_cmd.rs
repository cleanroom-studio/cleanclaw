//! `cleanclaw policy …` — list / show policy presets.
//!
//! Policies
//! describe the FS / net / tool rules the agent runtime enforces
//! during a turn.

use clap::Subcommand;
use cleanclaw_core::Result;
use cleanclaw_policy::{default_policy, load_preset, restricted_policy, standard_policy, Engine, Policy};

#[derive(Subcommand)]
pub enum PolicyCmd {
    /// List the built-in policy presets.
    List,
    /// Show a single policy preset in full (rules + presets).
    Show { name: String },
}

const BUILTIN_PRESETS: &[(&str, &str)] = &[
    ("default", "Default — host passthrough with audit logging"),
    ("restricted", "Restricted — read-only FS, no net, no exec"),
    ("standard", "Standard — workspace FS only, HTTPS net, all tools"),
];

pub async fn run(cmd: PolicyCmd) -> Result<()> {
    match cmd {
        PolicyCmd::List => list(),
        PolicyCmd::Show { name } => show(&name),
    }
}

fn list() -> Result<()> {
    println!("{:<16} {}", "NAME", "DESCRIPTION");
    for (id, desc) in BUILTIN_PRESETS {
        println!("{id:<16} {desc}");
    }
    Ok(())
}

fn show(name: &str) -> Result<()> {
    let policy: Policy = match name {
        "default" => default_policy(),
        "restricted" => restricted_policy(),
        "standard" => standard_policy(),
        _ => load_preset(name),
    };
    let engine = Engine::new(policy.clone());
    println!("name:        {}", policy.name);
    println!("description: {}", policy.description);
    println!();
    println!("# filesystem rules");
    if policy.filesystem.allow_read.is_empty() && policy.filesystem.allow_write.is_empty() {
        println!("  (none)");
    } else {
        for r in &policy.filesystem.allow_read {
            println!("  read:  {r}");
        }
        for r in &policy.filesystem.allow_write {
            println!("  write: {r}");
        }
    }
    println!("# network rules");
    if policy.network.outbound.is_empty() {
        println!("  (none)");
    } else {
        for r in &policy.network.outbound {
            println!("  host: {}  ports: {:?}", r.host, r.ports);
        }
    }
    println!("# resource limits");
    println!("  max_cpu:      {}", policy.resources.max_cpu);
    println!("  max_memory:   {}", policy.resources.max_memory);
    println!("  max_disk_mb:  {}", policy.resources.max_disk_mb);
    println!("  exec_timeout: {}s", policy.resources.exec_timeout_sec);
    let _ = engine; // exercise Engine ctor
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_returns_three_presets() {
        assert_eq!(BUILTIN_PRESETS.len(), 3);
    }

    #[test]
    fn show_known_presets() {
        for p in ["default", "restricted", "standard"] {
            assert!(show(p).is_ok());
        }
    }
}
