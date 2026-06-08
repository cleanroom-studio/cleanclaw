//! `cleanclaw provider …` — list / add / remove LLM providers.

use clap::Subcommand;
use cleanclaw_config::ProviderConfig;
use cleanclaw_core::{Result, UserId};
use cleanclaw_store::models::{ConfigRecord, UserRecord};
use cleanclaw_store::store::Store;
use std::collections::HashMap;

use crate::agents_cmd::open_store;

#[derive(Subcommand)]
pub enum ProviderCmd {
    /// List configured providers.
    Ls,
    /// Add or update a provider entry.
    Add {
        name: String,
        #[arg(long)]
        api_key: String,
        #[arg(long, default_value = "openai")]
        api_type: String,
        #[arg(long)]
        api_base: Option<String>,
        #[arg(long)]
        model: Vec<String>,
    },
    /// Remove a provider entry.
    Rm { name: String },
}

pub async fn run(cmd: ProviderCmd) -> Result<()> {
    let store = open_store().await?;
    match cmd {
        ProviderCmd::Ls => ls(&*store).await,
        ProviderCmd::Add {
            name,
            api_key,
            api_type,
            api_base,
            model,
        } => add(&*store, name, api_key, api_type, api_base, model).await,
        ProviderCmd::Rm { name } => rm(&*store, &name).await,
    }
}

async fn ls(store: &dyn Store) -> Result<()> {
    let rows = store.list_configs("provider", "", "").await?;
    if rows.is_empty() {
        println!("(no providers — add one with `cleanclaw provider add <name> --api-key …`)");
        return Ok(());
    }
    for r in rows {
        let api_type = r
            .data
            .get("apiType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let api_base = r
            .data
            .get("apiBase")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let models: Vec<String> = r
            .data
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "{:<20} {:<12} {:<40} {}",
            r.name,
            api_type,
            api_base,
            models.join(", ")
        );
    }
    Ok(())
}

async fn add(
    store: &dyn Store,
    name: String,
    api_key: String,
    api_type: String,
    api_base: Option<String>,
    model: Vec<String>,
) -> Result<()> {
    let now = cleanclaw_core::now_utc();
    let data = serde_json::json!({
        "apiKey": api_key,
        "apiType": api_type,
        "apiBase": api_base.unwrap_or_default(),
        "models": model.iter().map(|id| serde_json::json!({"id": id, "name": id})).collect::<Vec<_>>(),
    });
    let rec = ConfigRecord {
        id: format!("cfg_{}", UserId::generate()),
        kind: "provider".into(),
        scope: "system".into(),
        user_id: String::new(),
        agent_id: String::new(),
        name: name.clone(),
        enabled: true,
        credential_key: String::new(),
        data,
        created_at: now,
        updated_at: now,
    };
    store.save_config(&rec).await?;
    println!("provider {name} added");
    Ok(())
}

async fn rm(store: &dyn Store, name: &str) -> Result<()> {
    store.delete_config("provider", "", "", name).await?;
    println!("provider {name} removed");
    Ok(())
}

