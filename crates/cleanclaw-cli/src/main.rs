//! CleanClaw CLI.
//!
//! Subcommands are split into modules; the top-level `Cli` enum wires
//! them together.

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod admin_cmd;
mod agents_cmd;
mod apikey_cmd;
mod daemon_cmd;
mod plugin_cmd;
mod policy_cmd;
mod provider_cmd;
mod sandbox_cmd;
mod session_cmd;
mod skill_cmd;

#[derive(Parser)]
#[command(
    name = "cleanclaw",
    version,
    about = "CleanClaw — multi-tenant AI agent runtime"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the gateway (default when no subcommand given).
    Gateway {
        #[arg(long, default_value_t = 18953)]
        port: u16,
    },
    /// Operator-only DB operations (create user, reset password, …).
    #[command(subcommand)]
    Admin(admin_cmd::AdminCmd),
    /// Manage agents.
    #[command(subcommand)]
    Agents(agents_cmd::AgentsCmd),
    /// Manage LLM providers.
    #[command(subcommand)]
    Provider(provider_cmd::ProviderCmd),
    /// Manage API keys.
    #[command(subcommand)]
    Apikey(apikey_cmd::ApikeyCmd),
    /// Manage chat sessions.
    #[command(subcommand)]
    Session(session_cmd::SessionCmd),
    /// Manage skills.
    #[command(subcommand)]
    Skill(skill_cmd::SkillCmd),
    /// Manage the gateway daemon.
    #[command(subcommand)]
    Daemon(daemon_cmd::DaemonCmd),
    /// Manage plugins.
    #[command(subcommand)]
    Plugin(plugin_cmd::PluginCmd),
    /// Manage policy presets.
    #[command(subcommand)]
    Policy(policy_cmd::PolicyCmd),
    /// Manage sandboxed execution environments.
    #[command(subcommand)]
    Sandbox(sandbox_cmd::SandboxCmd),
    /// Print the version.
    Version,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("CLEANCLAW_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let res: std::result::Result<(), Box<dyn std::error::Error>> = match cli.cmd {
        Cmd::Gateway { port } => run_gateway(port).await,
        Cmd::Version => {
            println!("cleanclaw {}", cleanclaw_core::BUILD_VERSION);
            Ok(())
        }
        Cmd::Admin(c) => admin_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Agents(c) => agents_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Provider(c) => provider_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Apikey(c) => apikey_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Session(c) => session_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Skill(c) => skill_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Daemon(c) => daemon_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Plugin(c) => plugin_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Policy(c) => policy_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Cmd::Sandbox(c) => sandbox_cmd::run(c)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
    };
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

async fn run_gateway(port: u16) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let env = cleanclaw_config::load_env();
    let gw = cleanclaw_gateway::Gateway::boot(env, port)
        .await
        .map_err(|e| format!("boot: {e}"))?;
    cleanclaw_config::scrub_boot_secrets();
    gw.run().await.map_err(|e| format!("run: {e}"))?;
    Ok(())
}
