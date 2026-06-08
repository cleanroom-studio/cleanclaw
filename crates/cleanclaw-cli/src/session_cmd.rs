//! `cleanclaw session …` — list / show / rm chat sessions.

use clap::Subcommand;
use cleanclaw_core::Result;
use cleanclaw_store::store::Store;

use crate::agents_cmd::{open_store, resolve_agent};

#[derive(Subcommand)]
pub enum SessionCmd {
    /// List sessions for a given agent.
    Ls { name: String },
    /// Show a session's message history.
    Show { name: String, key: String },
    /// Delete a session.
    Rm { name: String, key: String },
}

pub async fn run(cmd: SessionCmd) -> Result<()> {
    let store = open_store().await?;
    match cmd {
        SessionCmd::Ls { name } => ls(&*store, &name).await,
        SessionCmd::Show { name, key } => show(&*store, &name, &key).await,
        SessionCmd::Rm { name, key } => rm(&*store, &name, &key).await,
    }
}

async fn ls(store: &dyn Store, name: &str) -> Result<()> {
    let agent = resolve_agent(store, name).await?;
    let sessions = store.list_sessions(&agent.user_id, &agent.id).await?;
    for s in sessions {
        println!("{:<28} {:<8} {}", s.key, s.message_count, s.title);
    }
    Ok(())
}

async fn show(store: &dyn Store, name: &str, key: &str) -> Result<()> {
    let agent = resolve_agent(store, name).await?;
    let msgs = store
        .list_session_messages(&agent.user_id, &agent.id, key)
        .await?;
    for m in msgs {
        println!("[{:>5}] {}", m.role, m.content);
    }
    Ok(())
}

async fn rm(store: &dyn Store, name: &str, key: &str) -> Result<()> {
    let agent = resolve_agent(store, name).await?;
    store.delete_session(&agent.user_id, &agent.id, key).await?;
    println!("session {key} deleted");
    Ok(())
}
