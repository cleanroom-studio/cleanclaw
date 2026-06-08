//! `cleanclaw agents …` — list / init / config / files / rm.
//!
//!

use clap::Subcommand;
use cleanclaw_core::{AgentId, Result, UserId};
use cleanclaw_store::models::{AgentRecord, UserRecord};
use cleanclaw_store::{open, StorageConfig, StorageType, Store};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum AgentsCmd {
    /// List all agents in the local store.
    Ls,
    /// Create + initialize a new agent (id, name, model).
    Init {
        name: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        api_key_env: Option<String>,
        #[arg(long)]
        username: Option<String>,
        #[arg(long)]
        soul: Option<PathBuf>,
        #[arg(long)]
        identity: Option<PathBuf>,
    },
    /// List or update the agent's identity files (SOUL.md, IDENTITY.md, …).
    Files {
        name: String,
        #[command(subcommand)]
        cmd: FilesCmd,
    },
    /// Delete an agent.
    Rm { name: String },
}

#[derive(Subcommand)]
pub enum FilesCmd {
    Ls { name: String },
    Get { name: String, filename: String },
    Put { name: String, filename: String, path: PathBuf },
}

pub async fn run(cmd: AgentsCmd) -> Result<()> {
    let store = open_store().await?;
    match cmd {
        AgentsCmd::Ls => ls(&*store).await,
        AgentsCmd::Init {
            name,
            model,
            api_key_env,
            username,
            soul,
            identity,
        } => {
            init(
                &*store,
                name,
                model,
                api_key_env,
                username,
                soul,
                identity,
            )
            .await
        }
        AgentsCmd::Files { name, cmd } => files(&*store, &name, cmd).await,
        AgentsCmd::Rm { name } => rm(&*store, &name).await,
    }
}

async fn ls(store: &dyn Store) -> Result<()> {
    let agents = store.list_all_agents().await?;
    if agents.is_empty() {
        println!("(no agents — run `cleanclaw agents init <name> --model openai/gpt-4o-mini`)");
        return Ok(());
    }
    for a in agents {
        let model = a
            .config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        println!("{:<24} {:<32} {}", a.id, a.name, model);
    }
    Ok(())
}

async fn init(
    store: &dyn Store,
    name: String,
    model: String,
    api_key_env: Option<String>,
    username: Option<String>,
    soul: Option<PathBuf>,
    identity: Option<PathBuf>,
) -> Result<()> {
    // Resolve owner user
    let owner_user_id = match username {
        Some(u) => ensure_user(store, &u).await?,
        None => ensure_admin(store).await?,
    };

    let agent = AgentRecord {
        id: AgentId::generate().to_string(),
        user_id: owner_user_id.clone(),
        name: name.clone(),
        config: serde_json::json!({"model": model}),
        is_public: false,
        created_at: cleanclaw_core::now_utc(),
        updated_at: cleanclaw_core::now_utc(),
    };
    store.save_agent(&agent).await?;
    println!("created agent {} ({})", agent.id, name);

    if let Some(api_key_env_name) = api_key_env {
        println!(
            "hint: export {api_key_env_name}=... then add the provider via `cleanclaw provider add <name>`"
        );
    }
    if let Some(path) = soul {
        let bytes = std::fs::read(&path)
            .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read {path:?}: {e}")))?;
        store
            .save_workspace_file(&agent.id, "", "SOUL.md", &bytes)
            .await?;
        println!("uploaded SOUL.md ({} bytes)", bytes.len());
    }
    if let Some(path) = identity {
        let bytes = std::fs::read(&path)
            .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read {path:?}: {e}")))?;
        store
            .save_workspace_file(&agent.id, "", "IDENTITY.md", &bytes)
            .await?;
        println!("uploaded IDENTITY.md ({} bytes)", bytes.len());
    }
    Ok(())
}

async fn files(store: &dyn Store, name: &str, cmd: FilesCmd) -> Result<()> {
    let agent = resolve_agent(store, name).await?;
    match cmd {
        FilesCmd::Ls { .. } => {
            let files = store.list_workspace_files(&agent.id).await?;
            for f in files {
                println!("{f}");
            }
            Ok(())
        }
        FilesCmd::Get { filename, .. } => {
            let (_, bytes) = store.get_workspace_file(&agent.id, "", &filename).await?;
            std::io::Write::write_all(&mut std::io::stdout(), &bytes).ok();
            Ok(())
        }
        FilesCmd::Put { filename, path, .. } => {
            let bytes = std::fs::read(&path)
                .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("read {path:?}: {e}")))?;
            store.save_workspace_file(&agent.id, "", &filename, &bytes).await?;
            println!("uploaded {filename} ({} bytes)", bytes.len());
            Ok(())
        }
    }
}

async fn rm(store: &dyn Store, name: &str) -> Result<()> {
    let agent = resolve_agent(store, name).await?;
    store.delete_agent(&agent.id).await?;
    println!("deleted agent {} ({name})", agent.id);
    Ok(())
}

pub async fn resolve_agent(store: &dyn Store, name: &str) -> Result<AgentRecord> {
    // Try id first
    if let Ok(a) = store.get_agent(name).await {
        return Ok(a);
    }
    // Then by name
    for a in store.list_all_agents().await? {
        if a.name == name {
            return Ok(a);
        }
    }
    Err(cleanclaw_core::CleanClawError::NotFound(format!("agent {name}")))
}

async fn ensure_admin(store: &dyn Store) -> Result<String> {
    if let Some(u) = first_user(store).await? {
        return Ok(u.id);
    }
    // Bootstrap the first user as super_admin with a random password
    let password = random_password();
    let hash = cleanclaw_auth::password::hash_password(&password)?;
    let u = UserRecord {
        id: UserId::generate().to_string(),
        username: "admin".into(),
        email: "admin@localhost".into(),
        password_hash: hash,
        display_name: "Admin".into(),
        role: "super_admin".into(),
        status: "active".into(),
        apikey_id: String::new(),
        external_id: String::new(),
        avatar_url: String::new(),
        agent_quota: -1,
        created_at: cleanclaw_core::now_utc(),
        updated_at: cleanclaw_core::now_utc(),
    };
    store.create_user(&u).await?;
    eprintln!("bootstrap: created admin user with password: {password}");
    Ok(u.id)
}

async fn ensure_user(store: &dyn Store, username: &str) -> Result<String> {
    if let Ok(u) = store.get_user_by_login(username).await {
        return Ok(u.id);
    }
    let password = random_password();
    let hash = cleanclaw_auth::password::hash_password(&password)?;
    let u = UserRecord {
        id: UserId::generate().to_string(),
        username: username.to_string(),
        email: format!("{username}@localhost"),
        password_hash: hash,
        display_name: username.to_string(),
        role: "user".into(),
        status: "active".into(),
        apikey_id: String::new(),
        external_id: String::new(),
        avatar_url: String::new(),
        agent_quota: -1,
        created_at: cleanclaw_core::now_utc(),
        updated_at: cleanclaw_core::now_utc(),
    };
    store.create_user(&u).await?;
    eprintln!("created user {username} with password: {password}");
    Ok(u.id)
}

async fn first_user(store: &dyn Store) -> Result<Option<UserRecord>> {
    match store.count_users().await? {
        0 => Ok(None),
        _ => Ok(Some(store.list_users().await?.into_iter().next().unwrap())),
    }
}

fn random_password() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| {
            let c: u8 = rng.gen_range(b'a'..=b'z');
            c as char
        })
        .collect()
}

pub async fn open_store() -> Result<Arc<dyn Store>> {
    let env = cleanclaw_config::load_env();
    let home = cleanclaw_config::env::home_dir();
    std::fs::create_dir_all(&home).ok();
    let cfg = StorageConfig {
        r#type: StorageType::parse(&env.storage.r#type),
        dsn: env.storage.dsn.clone(),
        auto_migrate: env.storage.auto_migrate,
    };
    let s = open(&cfg, &home).await?;
    Ok(Arc::from(s))
}
