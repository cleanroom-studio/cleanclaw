//! Configuration scope resolution. Mirrors
//! .
//!
//! Resolves (user, agent)-keyed rows out of the configs table and
//! merges them into the flat shapes the runtime expects. Walks
//! ownership outer→inner: system → user → agent → per-(user, agent).
//! Inner rows shadow outer ones by `name`.

use std::collections::HashMap;

use cleanclaw_config::{ChannelConfig, ProviderConfig};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::models::ConfigRecord;
use cleanclaw_store::Store;
use serde_json::Value;

/// HTTP-side scope identifiers. Translate to storage (user_id, agent_id)
/// via `ownership_from_scope`.
pub const SYSTEM: &str = "system";
pub const USER: &str = "user";
pub const AGENT: &str = "agent";
pub const USER_AGENT: &str = "user-agent";

/// Convert (scope, scopeID) to (user_id, agent_id) storage tuple.
pub fn ownership_from_scope(scope: &str, scope_id: &str) -> (String, String) {
    match scope {
        USER => (scope_id.to_string(), String::new()),
        AGENT => (String::new(), scope_id.to_string()),
        _ => (String::new(), String::new()),
    }
}

/// Inverse — emit (scope, scopeID) for the dashboard JSON. A row with
/// both `user_id` and `agent_id` set is rendered as `user-agent` so
/// the UI can tell it apart from plain user/agent rows.
pub fn scope_from_ownership(user_id: &str, agent_id: &str) -> (&'static str, String) {
    match (user_id.is_empty(), agent_id.is_empty()) {
        (false, false) => (USER_AGENT, format!("{user_id}/{agent_id}")),
        (false, true) => (USER, user_id.to_string()),
        (true, false) => (AGENT, agent_id.to_string()),
        (true, true) => (SYSTEM, String::new()),
    }
}

fn provider_to_config(rec: &ConfigRecord) -> ProviderConfig {
    serde_json::from_value(rec.data.clone()).unwrap_or_default()
}

fn channel_to_config(rec: &ConfigRecord) -> ChannelConfig {
    let mut c: ChannelConfig = serde_json::from_value(rec.data.clone()).unwrap_or_default();
    c.enabled = rec.enabled;
    c
}

/// Walk the four layers (system → user → agent → per-(user, agent))
/// and merge provider rows. Inner rows replace outer entries entirely.
pub async fn providers(
    st: &dyn Store,
    user_id: &str,
    agent_id: &str,
) -> Result<HashMap<String, ProviderConfig>, CleanClawError> {
    let mut out: HashMap<String, ProviderConfig> = HashMap::new();
    let mut apply = |rows: Vec<ConfigRecord>| {
        for r in rows {
            let name = r.name.clone();
            out.insert(name, provider_to_config(&r));
        }
    };
    let sys = st.list_configs("provider", "", "").await?;
    apply(sys);
    if !user_id.is_empty() {
        let rows = st.list_configs("provider", user_id, "").await?;
        apply(rows);
    }
    if !agent_id.is_empty() {
        let rows = st.list_configs("provider", "", agent_id).await?;
        apply(rows);
    }
    if !user_id.is_empty() && !agent_id.is_empty() {
        let rows = st.list_configs("provider", user_id, agent_id).await?;
        apply(rows);
    }
    Ok(out)
}

/// Return agent-only provider rows (no system or user layers merged).
pub async fn agent_scope_providers(
    st: &dyn Store,
    agent_id: &str,
) -> Result<HashMap<String, ProviderConfig>, CleanClawError> {
    if agent_id.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = st.list_configs("provider", "", agent_id).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let n = r.name.clone();
            (n, provider_to_config(&r))
        })
        .collect())
}

/// Return user-only provider rows (no system layer).
pub async fn user_scope_providers(
    st: &dyn Store,
    user_id: &str,
) -> Result<HashMap<String, ProviderConfig>, CleanClawError> {
    if user_id.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = st.list_configs("provider", user_id, "").await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let n = r.name.clone();
            (n, provider_to_config(&r))
        })
        .collect())
}

/// Merged channel map. A disabled row in an inner scope erases the
/// outer entry — lets a user opt out of a system-wide bot.
pub async fn channels(
    st: &dyn Store,
    user_id: &str,
    agent_id: &str,
) -> Result<HashMap<String, ChannelConfig>, CleanClawError> {
    let mut out: HashMap<String, ChannelConfig> = HashMap::new();
    let mut apply = |rows: Vec<ConfigRecord>| {
        for r in rows {
            let name = r.name.clone();
            if !r.enabled {
                out.remove(&name);
                continue;
            }
            out.insert(name, channel_to_config(&r));
        }
    };
    let sys = st.list_configs("channel", "", "").await?;
    apply(sys);
    if !user_id.is_empty() {
        let rows = st.list_configs("channel", user_id, "").await?;
        apply(rows);
    }
    if !agent_id.is_empty() {
        let rows = st.list_configs("channel", "", agent_id).await?;
        apply(rows);
    }
    if !user_id.is_empty() && !agent_id.is_empty() {
        let rows = st.list_configs("channel", user_id, agent_id).await?;
        apply(rows);
    }
    Ok(out)
}

/// Field-level merge of a single namespace across the four layers.
/// Inner keys win over outer ones. Unset namespaces yield an empty map.
pub async fn setting(
    st: &dyn Store,
    namespace: &str,
    user_id: &str,
    agent_id: &str,
) -> Result<HashMap<String, Value>, CleanClawError> {
    let mut out: HashMap<String, Value> = HashMap::new();

    async fn try_layer(
        st: &dyn Store,
        uid: &str,
        aid: &str,
        namespace: &str,
        out: &mut HashMap<String, Value>,
    ) -> Result<(), CleanClawError> {
        let rec = st.get_config("setting", uid, aid, namespace).await;
        match rec {
            Ok(r) => {
                if let Value::Object(map) = r.data {
                    for (k, v) in map {
                        out.insert(k, v);
                    }
                }
            }
            Err(CleanClawError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }

    try_layer(st, "", "", namespace, &mut out).await?;
    if !user_id.is_empty() {
        try_layer(st, user_id, "", namespace, &mut out).await?;
    }
    if !agent_id.is_empty() {
        try_layer(st, "", agent_id, namespace, &mut out).await?;
    }
    if !user_id.is_empty() && !agent_id.is_empty() {
        try_layer(st, user_id, agent_id, namespace, &mut out).await?;
    }
    Ok(out)
}

/// Resolve a setting and unmarshal into the given typed destination.
pub async fn setting_into<T: serde::de::DeserializeOwned + Default>(
    st: &dyn Store,
    namespace: &str,
    user_id: &str,
    agent_id: &str,
) -> Result<T, CleanClawError> {
    let merged = setting(st, namespace, user_id, agent_id).await?;
    if merged.is_empty() {
        return Ok(T::default());
    }
    let blob = serde_json::to_value(merged)?;
    Ok(serde_json::from_value(blob)?)
}

/// Upsert a single namespace at the given (user, agent) ownership.
/// Pass an empty map to delete the row instead of writing `{}`.
pub async fn save_setting(
    st: &dyn Store,
    user_id: &str,
    agent_id: &str,
    namespace: &str,
    data: HashMap<String, Value>,
) -> Result<(), CleanClawError> {
    if data.is_empty() {
        // Try to drop the row if it exists; missing-row is a no-op.
        match st.get_config("setting", user_id, agent_id, namespace).await {
            Ok(_r) => {
                st.delete_config("setting", user_id, agent_id, namespace)
                    .await?;
            }
            Err(CleanClawError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        return Ok(());
    }
    let now = chrono::Utc::now();
    let rec = ConfigRecord {
        id: format!("cfg_{}", uuid::Uuid::new_v4().simple()),
        kind: "setting".to_string(),
        scope: scope_from_ownership(user_id, agent_id).0.to_string(),
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        name: namespace.to_string(),
        enabled: true,
        credential_key: String::new(),
        data: Value::Object(data.into_iter().collect()),
        created_at: now,
        updated_at: now,
    };
    st.save_config(&rec).await
}

/// Upsert a provider row.
pub async fn save_provider(
    st: &dyn Store,
    user_id: &str,
    agent_id: &str,
    name: &str,
    p: &ProviderConfig,
) -> Result<(), CleanClawError> {
    let now = chrono::Utc::now();
    let rec = ConfigRecord {
        id: format!("cfg_{}", uuid::Uuid::new_v4().simple()),
        kind: "provider".to_string(),
        scope: scope_from_ownership(user_id, agent_id).0.to_string(),
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        name: name.to_string(),
        enabled: true,
        credential_key: String::new(),
        data: serde_json::to_value(p)?,
        created_at: now,
        updated_at: now,
    };
    st.save_config(&rec).await
}

/// Upsert a channel row.
pub async fn save_channel(
    st: &dyn Store,
    user_id: &str,
    agent_id: &str,
    channel_type: &str,
    credential_key: &str,
    enabled: bool,
    c: &ChannelConfig,
) -> Result<(), CleanClawError> {
    let now = chrono::Utc::now();
    let mut data = serde_json::to_value(c)?;
    // `enabled` lives on the row column, not in data.
    if let Value::Object(ref mut map) = data {
        map.remove("enabled");
    }
    let rec = ConfigRecord {
        id: format!("cfg_{}", uuid::Uuid::new_v4().simple()),
        kind: "channel".to_string(),
        scope: scope_from_ownership(user_id, agent_id).0.to_string(),
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        name: channel_type.to_string(),
        enabled,
        credential_key: credential_key.to_string(),
        data,
        created_at: now,
        updated_at: now,
    };
    st.save_config(&rec).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_from_scope_round_trip() {
        assert_eq!(
            ownership_from_scope("user", "u1"),
            ("u1".to_string(), "".to_string())
        );
        assert_eq!(
            ownership_from_scope("agent", "a1"),
            ("".to_string(), "a1".to_string())
        );
        assert_eq!(
            ownership_from_scope("system", ""),
            ("".to_string(), "".to_string())
        );
        assert_eq!(
            ownership_from_scope("garbage", "x"),
            ("".to_string(), "".to_string())
        );
    }

    #[test]
    fn scope_from_ownership_user_agent_compound() {
        assert_eq!(
            scope_from_ownership("u", "a"),
            ("user-agent", "u/a".to_string())
        );
        assert_eq!(scope_from_ownership("u", ""), ("user", "u".to_string()));
        assert_eq!(scope_from_ownership("", "a"), ("agent", "a".to_string()));
        assert_eq!(scope_from_ownership("", ""), ("system", "".to_string()));
    }

    #[test]
    fn channel_config_round_trip() {
        let c = ChannelConfig {
            enabled: true,
            bot_token: "abc".into(),
            app_token: "def".into(),
            ..Default::default()
        };
        let v = serde_json::to_value(&c).unwrap();
        let rec = ConfigRecord {
            id: "x".into(),
            kind: "channel".into(),
            scope: "user".into(),
            user_id: "u".into(),
            agent_id: "".into(),
            name: "telegram".into(),
            enabled: false,
            credential_key: "abc".into(),
            data: v.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let back = channel_to_config(&rec);
        assert_eq!(back.bot_token, "abc");
        assert_eq!(back.app_token, "def");
        // The row's `enabled` column overrides whatever was in `data`.
        assert!(!back.enabled, "row enabled column must win over data field");
    }

    #[test]
    fn provider_config_round_trip() {
        let p = ProviderConfig {
            api_key: "k".into(),
            api_base: "https://x".into(),
            api_type: "openai".into(),
            auth_type: "bearer".into(),
            models: vec![],
        };
        let v = serde_json::to_value(&p).unwrap();
        let rec = ConfigRecord {
            id: "x".into(),
            kind: "provider".into(),
            scope: "user".into(),
            user_id: "u".into(),
            agent_id: "".into(),
            name: "openai".into(),
            enabled: true,
            credential_key: "".into(),
            data: v,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let back = provider_to_config(&rec);
        assert_eq!(back.api_key, "k");
        assert_eq!(back.api_base, "https://x");
    }

    #[test]
    fn scope_round_trip_user() {
        let (u, a) = ownership_from_scope(USER, "u1");
        assert_eq!(u, "u1");
        assert_eq!(a, "");
        let (sc, id) = scope_from_ownership(&u, &a);
        assert_eq!(sc, USER);
        assert_eq!(id, "u1");
    }

    #[test]
    fn scope_round_trip_agent() {
        let (u, a) = ownership_from_scope(AGENT, "a1");
        assert_eq!(u, "");
        assert_eq!(a, "a1");
        let (sc, id) = scope_from_ownership(&u, &a);
        assert_eq!(sc, AGENT);
        assert_eq!(id, "a1");
    }

    #[test]
    fn scope_round_trip_system() {
        // System rows have empty (user, agent).
        let (u, a) = ownership_from_scope(SYSTEM, "");
        assert!(u.is_empty());
        assert!(a.is_empty());
        let (sc, _) = scope_from_ownership(&u, &a);
        assert_eq!(sc, SYSTEM);
    }

    #[test]
    fn scope_user_agent_renders_user_agent() {
        // A row with both user_id and agent_id set renders as
        // "user-agent" with "{user}/{agent}" as the scope id.
        let (sc, id) = scope_from_ownership("u1", "a1");
        assert_eq!(sc, USER_AGENT);
        assert_eq!(id, "u1/a1");
    }

    #[test]
    fn unknown_scope_falls_back_to_system() {
        let (u, a) = ownership_from_scope("garbage", "x");
        assert!(u.is_empty());
        assert!(a.is_empty());
    }
}
