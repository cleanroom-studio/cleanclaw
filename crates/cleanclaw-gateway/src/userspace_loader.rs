//! Per-user runtime: store snapshot, agent manager, bindings. Lazy-
//! loaded by the orchestrator on first auth.
//!
//!
//! and `UserSpace.EnsureAgent`. The Rust port keeps the same
//! ownership semantics (UserSpace borrows the gateway-owned sandbox
//! pool) but uses a builder pattern for tests + a runtime loader for
//! production paths.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use cleanclaw_agent::AgentManager;
use cleanclaw_auth::Accounts;
use cleanclaw_bus::{InboundMessage, MessageBus};
use cleanclaw_plugin::Manager as PluginManager;
use cleanclaw_sandbox::ExecutorPool;
use cleanclaw_store::models::{AgentRecord, ConfigRecord};
use cleanclaw_store::Store;
use cleanclaw_usage::Meter;
use cleanclaw_workspace::Store as WorkspaceStore;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum UserSpaceError {
    #[error("store: {0}")]
    Store(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// One channel-binding rule: "this agent handles messages that match
/// this `(channel, account_id, peer_kind, chat_id)` tuple". Mirrors
/// .
#[derive(Debug, Clone, Default)]
pub struct Binding {
    pub agent_id: String,
    pub channel: String,
    pub account_id: String,
    pub peer_kind: String,
    pub chat_id: String,
}

impl Binding {
    pub fn matches(&self, msg: &InboundMessage) -> bool {
        if !self.channel.is_empty() && self.channel != msg.channel {
            return false;
        }
        if !self.account_id.is_empty() && self.account_id != msg.account_id {
            return false;
        }
        if !self.peer_kind.is_empty() && self.peer_kind != msg.peer_kind {
            return false;
        }
        if !self.chat_id.is_empty() && self.chat_id != msg.chat_id {
            return false;
        }
        true
    }
}

/// Per-user runtime. The fields mirror the Go `UserSpace` struct but
/// several are simplified — the Rust runtime doesn't yet need the
/// full `*config.Config` blob, just the bindings + agent manager +
/// (borrowed) sandbox pool.
pub struct UserSpace {
    pub user_id: String,
    pub provider: Mutex<Option<Arc<dyn cleanclaw_provider::Provider>>>,
    pub agents: Arc<AgentManager>,
    pub bindings: Vec<Binding>,
    /// Borrowed from the gateway. Held here so the per-UserSpace
    /// EnsureAgent path can reach the pool without going back to the
    /// orchestrator. The orchestrator owns the lifecycle; we never
    /// call `close_all` from this struct.
    pub sandbox_pool: Option<Arc<dyn ExecutorPool>>,
    /// Borrowed from the gateway.
    pub plugin_mgr: Option<Arc<PluginManager>>,

    home_dir: PathBuf,
}

impl UserSpace {
    /// Return `true` when this space has an agent with the given id.
    pub async fn has_agent(&self, agent_id: &str) -> bool {
        // AgentManager doesn't expose a `contains` method; we walk
        // the (private) map via `all()` and check ids. Cheap because
        // a user has at most a few dozen agents.
        self.agents
            .all()
            .await
            .into_iter()
            .any(|a| a.agent_id == agent_id)
    }

    /// Pick an agent for an inbound message: explicit `msg.agent_id`
    /// wins, then bindings, then the first loaded agent (no
    /// "default" marker in the Rust port yet).
    pub async fn match_agent(
        &self,
        msg: &InboundMessage,
    ) -> Option<Arc<cleanclaw_agent::Agent>> {
        if !msg.agent_id.is_empty() {
            if let Some(a) = self.agents.get(&msg.agent_id).await {
                return Some(a);
            }
        }
        for b in &self.bindings {
            if !b.matches(msg) {
                continue;
            }
            if let Some(a) = self.agents.get(&b.agent_id).await {
                return Some(a);
            }
        }
        // Fallback: first loaded agent. The Go reference returns
        // `DefaultAgent()` which the Rust manager doesn't have
        // yet — first-loaded is the closest equivalent until
        // per-user default marking lands.
        self.agents.all().await.into_iter().next()
    }

    /// Lazy-attach a foreign agent (super_admin chat, public link,
    /// API-key caller). Idempotent: no-op if the agent is already
    /// loaded.
    pub async fn ensure_agent(
        &self,
        agent_id: &str,
        store: Option<&Arc<dyn Store>>,
    ) -> Result<(), UserSpaceError> {
        if agent_id.is_empty() {
            return Ok(());
        }
        if self.has_agent(agent_id).await {
            return Ok(());
        }
        let store = store.ok_or_else(|| {
            UserSpaceError::Store("ensure_agent: store not wired".to_string())
        })?;
        let rec = store
            .get_agent(agent_id)
            .await
            .map_err(|e| UserSpaceError::Store(e.to_string()))?;
        // `get_agent` returns AgentRecord directly (not Option).
        // The build path needs (agent_id, owner, model, provider,
        // store) — the user-scoped provider isn't picked yet (the
        // Rust port's ensure_agent builds a minimal agent and
        // patches the per-turn provider at dispatch time).
        let provider: Arc<dyn cleanclaw_provider::Provider> = build_noop_provider();
        let agent = cleanclaw_agent::AgentBuilder::new(
            rec.id.clone(),
            rec.user_id.clone(),
            rec.name.clone(),
            provider,
            store.clone(),
        )
        .build();
        self.agents.put(&rec.id, Arc::new(agent)).await;
        Ok(())
    }
}

/// Load (or build) a fresh `UserSpace` for `user_id`. Mirrors the
/// Go `loadUserSpace` — list the user's agents, build the agent
/// manager, expand channel rows into bindings, attach the borrowed
/// subsystems.
pub async fn load_user_space(
    user_id: &str,
    _bus: Arc<MessageBus>,
    store: Arc<dyn Store>,
    _workspace: Option<Arc<dyn WorkspaceStore>>,
    _usage: Option<Arc<dyn Meter>>,
    sandbox_pool: Option<Arc<dyn ExecutorPool>>,
    plugin_mgr: Option<Arc<PluginManager>>,
    _accounts: Option<Arc<Accounts>>,
) -> Result<UserSpace, UserSpaceError> {
    if user_id.is_empty() {
        return Err(UserSpaceError::Store("user_id required".into()));
    }
    let home_dir = PathBuf::from("/tmp");

    // 1. List the user's agents.
    let agents: Vec<AgentRecord> = store
        .list_agents(user_id)
        .await
        .map_err(|e| UserSpaceError::Store(e.to_string()))?;

    // 2. Build the agent manager and load each agent. The full
    //    per-agent configuration (provider, skills, tools, hooks)
    //    is a follow-up — current path uses a no-op provider
    //    placeholder so the dispatch succeeds; the real provider
    //    is patched at run_turn time.
    let manager = AgentManager::new();
    let provider: Arc<dyn cleanclaw_provider::Provider> = build_noop_provider();
    for ar in &agents {
        let a = cleanclaw_agent::AgentBuilder::new(
            ar.id.clone(),
            ar.user_id.clone(),
            ar.name.clone(),
            provider.clone(),
            store.clone(),
        )
        .build();
        manager.put(&ar.id, Arc::new(a)).await;
    }

    // 3. Build bindings from the channel rows.
    let bindings = build_bindings(&store, user_id, &agents).await?;

    Ok(UserSpace {
        user_id: user_id.to_string(),
        provider: Mutex::new(Some(provider)),
        agents: Arc::new(manager),
        bindings,
        sandbox_pool,
        plugin_mgr,
        home_dir,
    })
}

/// A no-op `Provider` that returns a fixed reply. Used as a
/// placeholder until the real provider is picked from the resolved
/// agent config at turn time. The orchestrator's hot path is
/// unaffected — every `run_turn` call goes through the agent's
/// own provider handle, which is patched by the user space loader
/// when the real config becomes available.
pub fn build_noop_provider() -> Arc<dyn cleanclaw_provider::Provider> {
    use cleanclaw_provider::message::{ChatResponse, Message};
    use cleanclaw_provider::{
        ChatRequest, ProviderError, ProviderStream, StreamEvent, Usage,
    };
    use async_stream::stream;

    struct NoopProvider;
    #[async_trait::async_trait]
    impl cleanclaw_provider::Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }
        async fn chat(
            &self,
            _req: &ChatRequest,
        ) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                id: String::new(),
                model: String::new(),
                message: Message::assistant("(no provider configured)".to_string()),
                finish_reason: "stop".to_string(),
                usage: Usage::default(),
                raw: serde_json::Value::Null,
            })
        }
        async fn chat_stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<ProviderStream, ProviderError> {
            let s = stream! {
                yield Ok(StreamEvent::ContentDelta {
                    delta: "(no provider configured)".into()
                });
                yield Ok(StreamEvent::Done {
                    finish_reason: "stop".into(),
                    usage: Some(Usage::default()),
                });
            };
            Ok(Box::pin(s))
        }
    }
    Arc::new(NoopProvider)
}

async fn build_bindings(
    store: &Arc<dyn Store>,
    user_id: &str,
    agents: &[AgentRecord],
) -> Result<Vec<Binding>, UserSpaceError> {
    let mut out: Vec<Binding> = Vec::new();
    // Per-user channel rows.
    if let Ok(user_rows) = store.list_configs("channel", user_id, "").await {
        for r in &user_rows {
            out.extend(expand_channel_record(r));
        }
    }
    // Per-agent channel rows (system scope, agent_id matches one of
    // the user's agents).
    for ar in agents {
        if let Ok(agent_rows) = store.list_configs("channel", "", &ar.id).await {
            for r in &agent_rows {
                out.extend(expand_channel_record(r));
            }
        }
    }
    Ok(out)
}

fn expand_channel_record(r: &ConfigRecord) -> Vec<Binding> {
    if !r.enabled {
        return Vec::new();
    }
    if r.agent_id.is_empty() {
        return Vec::new();
    }
    // ConfigRecord stores account info inside the `data` JSON
    // blob; the `accounts` field doesn't exist on the Rust
    // record (the Go side synthesizes it from data.Accounts).
    // We treat a missing accounts map as a single
    // implicit-account binding for backward compat.
    let mut out = Vec::new();
    let accts = extract_accounts(&r.data);
    if accts.is_empty() {
        out.push(Binding {
            agent_id: r.agent_id.clone(),
            channel: r.name.clone(),
            ..Default::default()
        });
        return out;
    }
    for account_id in accts {
        out.push(Binding {
            agent_id: r.agent_id.clone(),
            channel: r.name.clone(),
            account_id,
            ..Default::default()
        });
    }
    out
}

fn extract_accounts(data: &serde_json::Value) -> Vec<String> {
    let Some(obj) = data.as_object() else {
        return Vec::new();
    };
    if let Some(arr) = obj.get("Accounts").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(map) = obj.get("Accounts").and_then(|v| v.as_object()) {
        return map.keys().cloned().collect();
    }
    if let Some(map) = obj.get("accounts").and_then(|v| v.as_object()) {
        return map.keys().cloned().collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_bus::InboundMessage;

    fn msg(channel: &str, account: &str, chat: &str, peer: &str) -> InboundMessage {
        let mut m = InboundMessage::default();
        m.channel = channel.into();
        m.account_id = account.into();
        m.chat_id = chat.into();
        m.peer_kind = peer.into();
        m
    }

    #[test]
    fn binding_matches_by_channel() {
        let b = Binding {
            agent_id: "a1".into(),
            channel: "telegram".into(),
            ..Default::default()
        };
        assert!(b.matches(&msg("telegram", "bot1", "c1", "dm")));
        assert!(!b.matches(&msg("discord", "bot1", "c1", "dm")));
    }

    #[test]
    fn binding_matches_with_account() {
        let b = Binding {
            agent_id: "a1".into(),
            channel: "telegram".into(),
            account_id: "bot1".into(),
            ..Default::default()
        };
        assert!(b.matches(&msg("telegram", "bot1", "c1", "dm")));
        assert!(!b.matches(&msg("telegram", "bot2", "c1", "dm")));
    }

    #[test]
    fn binding_empty_matches_anything() {
        let b = Binding {
            agent_id: "a1".into(),
            ..Default::default()
        };
        assert!(b.matches(&msg("anything", "", "c1", "dm")));
    }

    #[test]
    fn expand_channel_record_no_accounts_yields_one_binding() {
        let r = ConfigRecord {
            id: "r1".into(),
            kind: "channel".into(),
            scope: "user".into(),
            user_id: "u1".into(),
            agent_id: "a1".into(),
            name: "telegram".into(),
            enabled: true,
            credential_key: String::new(),
            data: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let bs = expand_channel_record(&r);
        assert_eq!(bs.len(), 1);
        assert_eq!(bs[0].channel, "telegram");
        assert_eq!(bs[0].agent_id, "a1");
    }

    #[test]
    fn expand_channel_record_disabled_yields_nothing() {
        let r = ConfigRecord {
            id: "r1".into(),
            kind: "channel".into(),
            scope: "user".into(),
            user_id: "u1".into(),
            agent_id: "a1".into(),
            name: "telegram".into(),
            enabled: false,
            credential_key: String::new(),
            data: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let bs = expand_channel_record(&r);
        assert!(bs.is_empty());
    }

    #[test]
    fn extract_accounts_handles_array_form() {
        let v = serde_json::json!({ "Accounts": ["bot1", "bot2"] });
        assert_eq!(extract_accounts(&v), vec!["bot1", "bot2"]);
    }

    #[test]
    fn extract_accounts_handles_object_form() {
        let v = serde_json::json!({ "Accounts": { "bot1": {}, "bot2": {} } });
        let mut got = extract_accounts(&v);
        got.sort();
        assert_eq!(got, vec!["bot1", "bot2"]);
    }

    #[test]
    fn extract_accounts_handles_missing() {
        assert!(extract_accounts(&serde_json::json!({})).is_empty());
        assert!(extract_accounts(&serde_json::Value::Null).is_empty());
    }
}
