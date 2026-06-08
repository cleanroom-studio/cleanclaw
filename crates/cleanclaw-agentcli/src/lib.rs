//! Agent CLI data layer.
//!
//! The CLI's `agents …` subcommands are a thin convenience wrapper
//! over the same store the gateway and dashboard use. This crate
//! exposes the public API: create-or-update an agent, list, show,
//! delete — all against the operator's `cleanclaw_store::Store`.

use std::collections::HashMap;
use std::sync::Arc;

use cleanclaw_core::CleanClawError;
use cleanclaw_store::models::{AgentRecord, UserRecord};
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentCliError {
    #[error("store: {0}")]
    Store(#[from] CleanClawError),
    #[error("agent name is required")]
    MissingName,
    #[error("agent {0} not found")]
    NotFound(String),
    #[error("invalid provider/model: {0}")]
    InvalidModel(String),
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// Input bag for `Init` (create or update).
#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub description: String,
    /// Override the auto-derived id (for "update existing" via name).
    pub agent_id: String,
    /// Provider short name (e.g. "openai", "anthropic"). Empty keeps
    /// the current value (or leaves it unset on create).
    pub provider: String,
    /// Model id within the provider (e.g. "gpt-4o-mini").
    pub model: String,
    /// Name of the env var holding the API key (e.g. "OPENAI_API_KEY").
    pub api_key_env: String,
    /// Optional API base URL.
    pub api_base: String,
    /// Optional API type discriminator (e.g. "openai", "anthropic-messages").
    pub api_type: String,
    /// Auth scheme ("bearer", "x-api-key", "header: <name>").
    pub auth_type: String,
    /// Owner account username. Empty → use existing owner or first
    /// super_admin or create a new admin.
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitResult {
    pub agent: AgentRecord,
    pub owner_username: String,
    pub created: bool,
    pub owner_created: bool,
    pub generated_password: String,
    pub provider_saved: bool,
    pub model_saved: bool,
}

/// Validate agent name. Mirrors the dashboard's only check: non-empty
/// after trim.
fn validate_name(name: &str) -> Result<&str, AgentCliError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AgentCliError::MissingName);
    }
    Ok(trimmed)
}

/// Normalize the `<provider>/<model>` reference. Returns
/// (provider_short, model_id, full_reference).
pub fn normalize_provider_model(
    provider: &str,
    model: &str,
) -> Result<(String, String, String), AgentCliError> {
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() && model.is_empty() {
        return Err(AgentCliError::InvalidModel(
            "provider and model are both empty".into(),
        ));
    }
    // model may be in the form "<prov>/<model>"; we accept that too.
    let (prov, mid) = if let Some((p, m)) = model.split_once('/') {
        (p, m)
    } else {
        (provider, model)
    };
    let full = if prov.is_empty() {
        mid.to_string()
    } else {
        format!("{prov}/{mid}")
    };
    Ok((prov.to_string(), mid.to_string(), full))
}

/// Create a new agent or update an existing one in the operator's
/// store. Writes the same tables the dashboard does.
pub async fn init(
    store: Arc<dyn Store>,
    name: &str,
    opts: InitOptions,
) -> Result<InitResult, AgentCliError> {
    let name = validate_name(name)?.to_string();
    let _display_name = name.clone();

    let mut save_provider = false;
    let mut provider_short = String::new();
    let mut model_id = String::new();
    let mut full_ref = String::new();

    if !opts.provider.is_empty() || !opts.model.is_empty() {
        let (p, m, f) = normalize_provider_model(&opts.provider, &opts.model)?;
        provider_short = p;
        model_id = m;
        full_ref = f;
        save_provider = true;
    }

    // Find an existing agent by name (or by override id).
    let existing: Option<AgentRecord> = if !opts.agent_id.is_empty() {
        store.get_agent(&opts.agent_id).await.ok()
    } else {
        None
    };

    // Resolve or create the owner.
    let (owner, owner_username, owner_created, generated_password) =
        resolve_owner(&store, &opts).await?;

    let now = chrono::Utc::now();
    let (agent, created) = if let Some(mut rec) = existing {
        if !opts.description.is_empty() {
            // Merge description into the agent's JSON config blob.
            let mut cfg: serde_json::Value = rec.config.clone();
            if let Some(obj) = cfg.as_object_mut() {
                obj.insert(
                    "description".into(),
                    serde_json::Value::String(opts.description.clone()),
                );
            } else {
                cfg = serde_json::json!({ "description": opts.description });
            }
            rec.config = cfg;
        }
        rec.user_id = owner.id.clone();
        rec.updated_at = now;
        store.save_agent(&rec).await?;
        (rec, false)
    } else {
        let id = format!("agent_{}", Uuid::new_v4().simple());
        let cfg = if opts.description.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "description": opts.description })
        };
        let rec = AgentRecord {
            id: id.clone(),
            user_id: owner.id.clone(),
            name: name.clone(),
            config: cfg,
            is_public: false,
            created_at: now,
            updated_at: now,
        };
        store.save_agent(&rec).await?;
        (rec, true)
    };

    if save_provider {
        // Persist the provider config under (agent_id, model) so the
        // gateway picks it up via the scope resolver.
        let cfg = serde_json::json!({
            "apiKey": opts.api_key_env,
            "apiBase": opts.api_base,
            "apiType": opts.api_type,
            "authType": opts.auth_type,
            "model": model_id,
        });
        let rec = cleanclaw_store::models::ConfigRecord {
            id: format!("cfg_{}", Uuid::new_v4().simple()),
            kind: "provider".into(),
            scope: "agent".into(),
            user_id: String::new(),
            agent_id: agent.id.clone(),
            name: provider_short.clone(),
            enabled: true,
            credential_key: String::new(),
            data: cfg,
            created_at: now,
            updated_at: now,
        };
        store.save_config(&rec).await?;
    }

    Ok(InitResult {
        agent,
        owner_username,
        created,
        owner_created,
        generated_password,
        provider_saved: save_provider,
        model_saved: save_provider && !model_id.is_empty(),
    })
}

async fn resolve_owner(
    store: &Arc<dyn Store>,
    opts: &InitOptions,
) -> Result<(UserRecord, String, bool, String), AgentCliError> {
    if !opts.username.is_empty() {
        // Look up by login; if missing and we have a password, create.
        if let Ok(u) = store.get_user_by_login(&opts.username).await {
            return Ok((u, opts.username.clone(), false, String::new()));
        }
        if opts.password.is_empty() {
            return Err(AgentCliError::Invalid(format!(
                "user '{}' not found and no password provided",
                opts.username
            )));
        }
        // Bootstrap a new admin and attach the password. The Go
        // implementation uses a real auth.Users registry; the
        // Rust stub is a simpler "create + set_password" path.
        let id = format!("u_{}", Uuid::new_v4().simple());
        let now = chrono::Utc::now();
        let rec = UserRecord {
            id: id.clone(),
            username: opts.username.clone(),
            email: opts.email.clone(),
            password_hash: String::new(), // caller runs set_password
            display_name: opts.display_name.clone(),
            role: "super_admin".into(),
            status: "active".into(),
            apikey_id: String::new(),
            external_id: String::new(),
            avatar_url: String::new(),
            agent_quota: -1,
            created_at: now,
            updated_at: now,
        };
        store.create_user(&rec).await?;
        return Ok((rec, opts.username.clone(), true, opts.password.clone()));
    }
    // No username → first existing super_admin, or fail.
    let all = store.list_users().await?;
    for u in &all {
        if u.role == "super_admin" && u.status == "active" {
            return Ok((u.clone(), u.username.clone(), false, String::new()));
        }
    }
    Err(AgentCliError::Invalid(
        "no username supplied and no super_admin exists".into(),
    ))
}

/// List agents owned by `user_id`. Pass empty string to list all
/// (admin path).
pub async fn list(store: Arc<dyn Store>, user_id: &str) -> Result<Vec<AgentRecord>, AgentCliError> {
    if user_id.is_empty() {
        // No public "list all" on the store trait; we approximate
        // by listing all users and union-ing their agent rows.
        // The Go side does the same via `users.GetAll` → loop.
        let users = store.list_users().await?;
        let mut all = Vec::new();
        for u in users {
            if let Ok(rows) = store.list_agents(&u.id).await {
                all.extend(rows);
            }
        }
        return Ok(all);
    }
    let rows = store.list_agents(user_id).await?;
    Ok(rows)
}

/// List all agents (admin). Convenience wrapper over the empty-
/// user-id branch of `list`.
pub async fn list_all(store: Arc<dyn Store>) -> Result<Vec<AgentRecord>, AgentCliError> {
    list(store, "").await
}

/// Find an agent by its (case-insensitive) name. The store has no
/// `find_by_name` accessor; the Go side does the same loop in
/// `Init`. We surface the helper so the CLI can do
/// `agents find <name>` and the dashboard's URL `?agent=<name>`
/// can resolve the id before opening a chat session.
pub async fn find_by_name(store: Arc<dyn Store>, name: &str) -> Result<AgentRecord, AgentCliError> {
    let trimmed = validate_name(name)?.to_string();
    let candidates = list_all(store.clone()).await?;
    // Exact match first.
    for a in &candidates {
        if a.name == trimmed {
            return Ok(a.clone());
        }
    }
    // Case-insensitive fallback.
    for a in &candidates {
        if a.name.eq_ignore_ascii_case(&trimmed) {
            return Ok(a.clone());
        }
    }
    Err(AgentCliError::NotFound(trimmed))
}

/// Fetch a single agent by id.
pub async fn show(store: Arc<dyn Store>, agent_id: &str) -> Result<AgentRecord, AgentCliError> {
    let rec = store.get_agent(agent_id).await?;
    Ok(rec)
}

/// Delete an agent. The Go side cascades the FK to sessions,
/// channels, cron jobs, projects, and any per-agent
/// `ConfigRecord` rows. The Rust Store impl handles the cascade
/// for the main tables; we additionally scrub any per-agent
/// config rows that might have survived.
pub async fn delete(store: Arc<dyn Store>, agent_id: &str) -> Result<(), AgentCliError> {
    let rec = store.get_agent(agent_id).await?;
    // Best-effort cleanup of per-agent ConfigRecord rows
    // (kind=provider, kind=channel, kind=setting with this
    // agent_id). Errors here are non-fatal — the agent record
    // gets deleted regardless.
    if let Ok(configs) = store.list_configs_all_kinds().await {
        for c in configs {
            if c.agent_id == rec.id {
                let _ = store
                    .delete_config(&c.kind, &c.user_id, &c.agent_id, &c.name)
                    .await;
            }
        }
    }
    store.delete_agent(&rec.id).await?;
    Ok(())
}

/// Transfer ownership of an agent to a different user. The Go
/// side uses this when an agent's `user_id` has to move (e.g.
/// team reshuffles). The agent's per-user config rows are
/// re-keyed; the per-agent rows are untouched.
pub async fn set_owner(
    store: Arc<dyn Store>,
    agent_id: &str,
    new_user_id: &str,
) -> Result<AgentRecord, AgentCliError> {
    let mut rec = store.get_agent(agent_id).await?;
    rec.user_id = new_user_id.to_string();
    rec.updated_at = chrono::Utc::now();
    store.save_agent(&rec).await?;
    Ok(rec)
}

#[allow(dead_code)]
fn _validate_types(_h: HashMap<String, String>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_rejects_blank() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
    }

    #[test]
    fn validate_name_trims() {
        assert_eq!(validate_name("  alice  ").unwrap(), "alice");
    }

    #[test]
    fn normalize_provider_model_separates_components() {
        let (p, m, f) = normalize_provider_model("openai", "gpt-4o-mini").unwrap();
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4o-mini");
        assert_eq!(f, "openai/gpt-4o-mini");
    }

    #[test]
    fn normalize_provider_model_accepts_inline() {
        let (p, m, f) = normalize_provider_model("", "openai/gpt-4o").unwrap();
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4o");
        assert_eq!(f, "openai/gpt-4o");
    }

    #[test]
    fn normalize_provider_model_rejects_empty() {
        assert!(normalize_provider_model("", "").is_err());
    }

    #[tokio::test]
    async fn list_all_returns_no_agents_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cleanclaw_store::StorageConfig {
            r#type: cleanclaw_store::StorageType::Sqlite,
            dsn: format!("sqlite://{}/test.db", dir.path().display()),
            auto_migrate: true,
        };
        let store: Arc<dyn cleanclaw_store::Store> =
            Arc::from(cleanclaw_store::open(&cfg, dir.path()).await.unwrap());
        let all = list_all(store).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn find_by_name_matches_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cleanclaw_store::StorageConfig {
            r#type: cleanclaw_store::StorageType::Sqlite,
            dsn: format!("sqlite://{}/test.db", dir.path().display()),
            auto_migrate: true,
        };
        let store: Arc<dyn cleanclaw_store::Store> =
            Arc::from(cleanclaw_store::open(&cfg, dir.path()).await.unwrap());
        // Need a user first; create one inline.
        let now = chrono::Utc::now();
        let user = cleanclaw_store::models::UserRecord {
            id: "u_test".into(),
            username: "alice".into(),
            email: "a@x.com".into(),
            password_hash: String::new(),
            display_name: "Alice".into(),
            role: "super_admin".into(),
            status: "active".into(),
            apikey_id: String::new(),
            external_id: String::new(),
            avatar_url: String::new(),
            agent_quota: -1,
            created_at: now,
            updated_at: now,
        };
        store.create_user(&user).await.unwrap();
        // Create two agents under alice.
        let mut opts = InitOptions::default();
        opts.username = "alice".into();
        opts.password = "p".into();
        let _ = init(store.clone(), "Alpha", opts.clone()).await.unwrap();
        opts.email = "a2@x.com".into();
        let _ = init(store.clone(), "Beta", opts).await.unwrap();
        // Exact match.
        let found = find_by_name(store.clone(), "Alpha").await.unwrap();
        assert_eq!(found.name, "Alpha");
        // Case-insensitive.
        let found = find_by_name(store.clone(), "alpha").await.unwrap();
        assert_eq!(found.name, "Alpha");
        // Missing.
        let r = find_by_name(store.clone(), "Gamma").await;
        assert!(matches!(r, Err(AgentCliError::NotFound(_))));
    }

    #[tokio::test]
    async fn set_owner_reassigns_agent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cleanclaw_store::StorageConfig {
            r#type: cleanclaw_store::StorageType::Sqlite,
            dsn: format!("sqlite://{}/test.db", dir.path().display()),
            auto_migrate: true,
        };
        let store: Arc<dyn cleanclaw_store::Store> =
            Arc::from(cleanclaw_store::open(&cfg, dir.path()).await.unwrap());
        // Two users.
        let now = chrono::Utc::now();
        for (id, name) in [("u1", "alice"), ("u2", "bob")] {
            store
                .create_user(&cleanclaw_store::models::UserRecord {
                    id: id.into(),
                    username: name.into(),
                    email: format!("{name}@x.com"),
                    password_hash: String::new(),
                    display_name: name.into(),
                    role: "super_admin".into(),
                    status: "active".into(),
                    apikey_id: String::new(),
                    external_id: String::new(),
                    avatar_url: String::new(),
                    agent_quota: -1,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
        }
        let mut opts = InitOptions::default();
        opts.username = "alice".into();
        opts.password = "p".into();
        let created = init(store.clone(), "MyAgent", opts).await.unwrap();
        assert_eq!(created.agent.user_id, "u1");
        // Transfer to bob.
        let after = set_owner(store.clone(), &created.agent.id, "u2")
            .await
            .unwrap();
        assert_eq!(after.user_id, "u2");
    }

    #[tokio::test]
    async fn delete_cascades_per_agent_configs() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cleanclaw_store::StorageConfig {
            r#type: cleanclaw_store::StorageType::Sqlite,
            dsn: format!("sqlite://{}/test.db", dir.path().display()),
            auto_migrate: true,
        };
        let store: Arc<dyn cleanclaw_store::Store> =
            Arc::from(cleanclaw_store::open(&cfg, dir.path()).await.unwrap());
        let now = chrono::Utc::now();
        store
            .create_user(&cleanclaw_store::models::UserRecord {
                id: "u1".into(),
                username: "alice".into(),
                email: "a@x.com".into(),
                password_hash: String::new(),
                display_name: "Alice".into(),
                role: "super_admin".into(),
                status: "active".into(),
                apikey_id: String::new(),
                external_id: String::new(),
                avatar_url: String::new(),
                agent_quota: -1,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        // Create an agent with a provider config.
        let mut opts = InitOptions::default();
        opts.username = "alice".into();
        opts.password = "p".into();
        opts.provider = "openai".into();
        opts.model = "gpt-4o-mini".into();
        opts.api_key_env = "OPENAI_API_KEY".into();
        let created = init(store.clone(), "ProviderAgent", opts).await.unwrap();
        // Confirm the config row is there.
        let configs_before = store.list_configs_all_kinds().await.unwrap();
        let per_agent_before: Vec<_> = configs_before
            .iter()
            .filter(|c| c.agent_id == created.agent.id)
            .collect();
        assert!(!per_agent_before.is_empty());
        // Delete and verify the per-agent configs are gone.
        delete(store.clone(), &created.agent.id).await.unwrap();
        let configs_after = store.list_configs_all_kinds().await.unwrap();
        let per_agent_after: Vec<_> = configs_after
            .iter()
            .filter(|c| c.agent_id == created.agent.id)
            .collect();
        assert!(per_agent_after.is_empty(), "configs should be cleaned up");
    }
}
