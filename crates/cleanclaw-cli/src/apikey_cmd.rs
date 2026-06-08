//! `cleanclaw apikey …` — list / create / rotate / delete API keys.

use clap::Subcommand;
use cleanclaw_auth::apikey;
use cleanclaw_core::{ApiKeyId, Result, UserId};
use cleanclaw_store::models::ApiKeyRecord;
use cleanclaw_store::Store;

use crate::agents_cmd::open_store;

#[derive(Subcommand)]
pub enum ApikeyCmd {
    /// List API keys (optionally filtered by user).
    Ls {
        #[arg(long)]
        user: Option<String>,
    },
    /// Mint a new API key for a user.
    Create {
        #[arg(long)]
        user: String,
        #[arg(long, default_value = "user")]
        r#type: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Rotate an existing API key (issues a new token, keeps the id).
    Rotate { id: String },
    /// Delete an API key.
    Rm { id: String },
}

pub async fn run(cmd: ApikeyCmd) -> Result<()> {
    let store = open_store().await?;
    match cmd {
        ApikeyCmd::Ls { user } => ls(&*store, user).await,
        ApikeyCmd::Create { user, r#type, name } => create(&*store, &user, &r#type, name).await,
        ApikeyCmd::Rotate { id } => rotate(&*store, &id).await,
        ApikeyCmd::Rm { id } => rm(&*store, &id).await,
    }
}

async fn ls(store: &dyn Store, user: Option<String>) -> Result<()> {
    let rows = match user {
        Some(u) => store.list_api_keys(&resolve_user(store, &u).await?).await?,
        None => {
            let mut all = Vec::new();
            for u in store.list_users().await? {
                all.extend(store.list_api_keys(&u.id).await?);
            }
            all
        }
    };
    if rows.is_empty() {
        println!("(no api keys)");
        return Ok(());
    }
    for r in rows {
        println!("{:<20} {:<8} {:<20} {}", r.id, r.r#type, r.key_prefix, r.name);
    }
    Ok(())
}

async fn create(store: &dyn Store, user: &str, r#type: &str, name: Option<String>) -> Result<()> {
    let user_id = resolve_user(store, user).await?;
    let (token, hash, prefix) = apikey::generate();
    let rec = ApiKeyRecord {
        id: ApiKeyId::generate().to_string(),
        user_id,
        name: name.unwrap_or_else(|| "default".into()),
        key_hash: hash,
        key_prefix: prefix,
        r#type: r#type.into(),
        created_at: cleanclaw_core::now_utc(),
        prev_hash: None,
        prev_hash_set_at: None,
    };
    store.create_api_key(&rec).await?;
    println!("id        : {}", rec.id);
    println!("type      : {}", r#type);
    println!("prefix    : {}", rec.key_prefix);
    println!();
    println!("API KEY (save it now — shown only once):");
    println!("{token}");
    Ok(())
}

async fn rotate(store: &dyn Store, id: &str) -> Result<()> {
    let (token, hash, prefix) = apikey::generate();
    store.rotate_api_key(id, &hash, &prefix).await?;
    println!();
    println!("new API KEY (save it now): {token}");
    Ok(())
}

async fn rm(store: &dyn Store, id: &str) -> Result<()> {
    store.delete_api_key(id).await?;
    println!("api key {id} deleted");
    Ok(())
}

pub async fn resolve_user(store: &dyn Store, login: &str) -> Result<String> {
    Ok(store.get_user_by_login(login).await?.id)
}

// Suppress unused import warning for UserId (used in provider_cmd).
#[allow(dead_code)]
fn _unused_userid(_: UserId) {}
