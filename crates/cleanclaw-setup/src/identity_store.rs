//! Identity / agent-config helpers. Mirrors
//! .
//!
//! The Go file has three small helpers that the various setup
//! handlers lean on:
//!   - `loadAgentFileConfig(ctx, agentID)` — read per-agent overrides
//!     from `agents.config`
//!   - `saveAgentFileConfig(ctx, agentID, cfg)` — write overrides
//!     back as JSON
//!   - `isStoreNotFound(err)` — recognise "not found" across SQLite
//!     (`sql.ErrNoRows`) and Postgres (`store.ErrNotFound`) backends

use serde_json::Value;

use cleanclaw_store::models::AgentRecord;
use cleanclaw_store::Store;

const NOT_FOUND_FRAGMENT: &str = "no rows in result set";

/// Load an agent's per-row `config` JSON blob. Returns an empty
/// `Value::Null` when the agent doesn't exist (callers wanting a
/// concrete struct can deserialize this into their own type).
pub async fn load_agent_file_config(
    store: &dyn Store,
    agent_id: &str,
) -> Result<Value, String> {
    match store.get_agent(agent_id).await {
        Ok(rec) => {
            // rec.config is already serde_json::Value
            Ok(rec.config)
        }
        Err(e) if is_store_not_found(&e.to_string()) => Ok(Value::Null),
        Err(e) => Err(e.to_string()),
    }
}

/// Persist per-agent overrides into the agent's `config` column.
/// When the agent row is missing, a fresh one is created. The
/// `config` is stored as a `Value` (any JSON shape) so this
/// helper doesn't have to know about the wire schema.
pub async fn save_agent_file_config(
    store: &dyn Store,
    agent_id: &str,
    config: &Value,
    owner_user_id: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut rec = match store.get_agent(agent_id).await {
        Ok(r) => r,
        Err(e) if is_store_not_found(&e.to_string()) => AgentRecord {
            id: agent_id.to_string(),
            user_id: owner_user_id.to_string(),
            name: agent_id.to_string(),
            config: Value::Null,
            is_public: false,
            created_at: now,
            updated_at: now,
        },
        Err(e) => return Err(e.to_string()),
    };
    rec.config = config.clone();
    rec.updated_at = now;
    if rec.created_at.timestamp() == 0 {
        rec.created_at = now;
    }
    store.save_agent(&rec).await.map_err(|e| e.to_string())
}

/// True when the error string contains either `sql.ErrNoRows`
/// or `store.ErrNotFound`. Both backends (sqlite / postgres)
/// surface a "not found" signal but in different shapes — the
/// Go version checks both via `errors.Is`; here we lean on
/// the stringified form since `Store` doesn't expose a typed
/// not-found error yet.
pub fn is_store_not_found(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("not found") || lower.contains(NOT_FOUND_FRAGMENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_core::now_utc;
    use cleanclaw_store::models::UserRecord;

    async fn test_store() -> std::sync::Arc<dyn Store> {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cleanclaw_store::StorageConfig {
            r#type: cleanclaw_store::StorageType::Sqlite,
            dsn: format!("sqlite://{}/test.db", dir.path().display()),
            auto_migrate: true,
        };
        let st = cleanclaw_store::open(&cfg, dir.path()).await.unwrap();
        let u = UserRecord {
            id: "u1".into(),
            username: "alice".into(),
            email: "a@x.com".into(),
            password_hash: String::new(),
            display_name: "alice".into(),
            role: "user".into(),
            status: "active".into(),
            apikey_id: String::new(),
            external_id: String::new(),
            avatar_url: String::new(),
            agent_quota: -1,
            created_at: now_utc(),
            updated_at: now_utc(),
        };
        st.create_user(&u).await.unwrap();
        let st: std::sync::Arc<dyn Store> = st.into();
        st
    }

    #[tokio::test]
    async fn load_missing_agent_returns_null() {
        let st = test_store().await;
        let cfg = load_agent_file_config(&*st, "nope").await.unwrap();
        assert!(cfg.is_null());
    }

    #[tokio::test]
    async fn save_then_load_round_trips_config() {
        let st = test_store().await;
        let payload = serde_json::json!({"color": "red", "max_tokens": 100});
        save_agent_file_config(&*st, "a1", &payload, "u1")
            .await
            .unwrap();
        let back = load_agent_file_config(&*st, "a1").await.unwrap();
        assert_eq!(back["color"], "red");
        assert_eq!(back["max_tokens"], 100);
    }

    #[test]
    fn is_store_not_found_recognises_both_shapes() {
        assert!(is_store_not_found("sql: no rows in result set"));
        assert!(is_store_not_found("store: not found"));
        assert!(is_store_not_found("agent not found"));
        assert!(!is_store_not_found("connection refused"));
    }
}
