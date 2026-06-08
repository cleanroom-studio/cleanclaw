//! Gateway orchestrator.
//!
//! The Gateway is the runtime orchestrator that:
//!   1. Opens the Store + Workspace + Plugin Manager
//!   2. Hosts the Channel Manager (Telegram/Discord/Slack/Feishu/WeChat/LINE/Web)
//!   3. Hosts the Cron Scheduler (db-backed, ticks on a timer)
//!   4. Hosts the Webhook Server (HTTP-based inbound)
//!   5. Runs the inbound routing loop (resolve owner → user space → agent)
//!   6. Lazy-loads per-user `UserSpace`s with idle eviction
//!   7. Hot-reloads cached spaces on SIGHUP (Unix) or admin request
//!
//! The Rust port uses a builder pattern: every heavy subsystem is
//! `Option<Arc<T>>` and gets wired via `with_*` methods. The
//! orchestrator checks before using each, so a minimal config (bus +
//! dedup only) still runs and passes tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cleanclaw_bus::MessageBus;
use cleanclaw_config::EnvConfig;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub mod dedup;
pub mod orchestrator;
pub mod userspace;
pub mod userspace_loader;

pub use dedup::{spawn_dedup_cleanup, Dedup, CLEANUP_INTERVAL};
pub use orchestrator::{Orchestrator, OrchestratorError};
// The real per-user runtime lives in `userspace_loader`. The legacy
// `userspace::UserSpace` (a generic handle container used by the
// in-process `UserSpaceCache`) is re-exported under a distinct name
// to avoid clobbering the orchestrator's `UserSpace` type.
pub use userspace::{
    CacheStats, SpaceFactory, UserSpaceCache, UserSpace as UserSpaceHandle,
};
pub use userspace_loader::{Binding, UserSpace, UserSpaceError};

/// Per-(channel, account, chat) routing key used by the task queue
/// so messages for one chat run sequentially. Includes accountID
/// because two bots of the same channel type can share a chat_id
/// (e.g. Telegram chat 12345 on bot A is unrelated to chat 12345 on
/// bot B).
pub fn chat_key(channel: &str, account_id: &str, chat_id: &str) -> String {
    format!("{channel}:{account_id}:{chat_id}")
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("store: {0}")]
    Store(#[from] cleanclaw_core::CleanClawError),
    #[error("missing owner for inbound {channel}:{chat_id}")]
    MissingOwner { channel: String, chat_id: String },
    #[error("orchestrator: {0}")]
    Orchestrator(#[from] OrchestratorError),
    #[error("bind: {0}")]
    Bind(#[from] std::net::AddrParseError),
    #[error("http serve: {0}")]
    HttpServe(String),
}

/// The thin `Gateway` facade kept for backward compatibility with
/// the existing CLI callsites (`Gateway::boot(env, port)` and
/// `gw.run()`). The heavy lifting now lives in `Orchestrator`.
pub struct Gateway {
    pub orch: Arc<Orchestrator>,
    port: u16,
    /// Opened store (sqlite by default) once `boot` wires the full
    /// stack. Held so `start_http` can build the API + setup routers
    /// without re-opening it.
    store: Option<Arc<dyn cleanclaw_store::Store>>,
    /// Chat service that the `/api/chat/stream` endpoint drives.
    chat: Option<Arc<cleanclaw_api::chat::ChatService>>,
}

impl Gateway {
    /// Build a `Gateway` from an `EnvConfig` + port, opening the
    /// store, creating the chat service, and wiring the orchestrator
    /// with all heavy subsystems (`Accounts`, store, scheduler,
    /// plugin manager, channel manager, webhook bridge). The HTTP
    /// server is bound lazily by `start_http` so the orchestrator can
    /// still be inspected via tests without binding a port.
    pub async fn boot(
        env: EnvConfig,
        port: u16,
    ) -> Result<Arc<Self>, GatewayError> {
        let bus = Arc::new(MessageBus::new(100));
        let home = cleanclaw_config::home_dir();
        // Clone the env so we can both pass it to the orchestrator
        // AND read storage config below.
        let env_for_orch = env.clone();
        let orch = Arc::new(Orchestrator::new(bus, env_for_orch, home.clone()));

        // Open the configured store. Default is sqlite at
        // `$CLEANCLAW_HOME/cleanclaw.db`; the env can override via
        // CLEANCLAW_STORAGE_TYPE / CLEANCLAW_STORAGE_DSN.
        let store_cfg = cleanclaw_store::StorageConfig {
            r#type: cleanclaw_store::StorageType::parse(&env.storage.r#type),
            dsn: env.storage.dsn.clone(),
            auto_migrate: env.storage.auto_migrate,
        };
        let store_box = cleanclaw_store::factory::open(&store_cfg, &home).await?;
        let store: Arc<dyn cleanclaw_store::Store> = Arc::from(store_box);

        // Auth: cookie sessions + apikey bearer. The `Resolver` is
        // shared by both the API router and the setup router; the
        // `Accounts` is what the setup router uses for admin flows.
        let accounts = Arc::new(
            cleanclaw_auth::Accounts::new(store.clone())
                .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("accounts: {e}")))?,
        );
        let resolver = Arc::new(cleanclaw_auth::Resolver::new(store.clone()));

        // Chat service: the runtime that `/api/chat/stream` drives.
        // The default model falls back to `MiniMax-M3` (matches the
        // shipped `.env` / e2e tests) so a fresh install can stream
        // a turn without manually adding a provider first.
        let default_model = std::env::var("CLEANCLAW_DEFAULT_MODEL")
            .unwrap_or_else(|_| "MiniMax-M3".to_string());
        let chat = Arc::new(cleanclaw_api::chat::ChatService::new(
            store.clone(),
            default_model,
        ));

        // Register a default provider if the operator exported
        // OPENAI_API_KEY or ANTHROPIC_API_KEY (mirrors the
        // CleanClaw `bootDefaultProvider` behavior). Without one of
        // these the chat service will reject turns with a clear
        // "no provider registered" error.
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            if !api_key.is_empty() {
                let api_base = std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                let cfg = cleanclaw_config::ProviderConfig {
                    api_key,
                    api_base,
                    api_type: "openai".into(),
                    auth_type: "bearer-token".into(),
                    models: vec![],
                };
                if let Ok(p) = cleanclaw_provider::build_provider("openai", &cfg) {
                    chat.register_provider("openai", p);
                    tracing::info!("registered default OpenAI provider");
                }
            }
        }
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            if !api_key.is_empty() {
                let api_base = std::env::var("ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
                let cfg = cleanclaw_config::ProviderConfig {
                    api_key,
                    api_base,
                    api_type: "anthropic".into(),
                    auth_type: "api-key".into(),
                    models: vec![],
                };
                if let Ok(p) = cleanclaw_provider::build_provider("anthropic", &cfg) {
                    chat.register_provider("anthropic", p);
                    tracing::info!("registered default Anthropic provider");
                }
            }
        }

        // Wire the orchestrator with everything we have. Subsystems
        // that aren't auto-built (channels, webhook server, plugin
        // manager, scheduler) stay None and the orchestrator's
        // `if let Some(...)` guards skip them. We hold the only
        // `Arc<Orchestrator>` at this point so `Arc::get_mut`
        // succeeds.
        let mut orch_mut = orch.clone();
        if let Some(o) = Arc::get_mut(&mut orch_mut) {
            o.store = Some(store.clone());
            o.accounts = Some(accounts.clone());
        }
        drop(orch_mut);

        Ok(Arc::new(Self {
            orch,
            port,
            store: Some(store),
            chat: Some(chat),
        }))
    }

    /// Build a `Gateway` that wraps a pre-configured `Orchestrator`.
    /// Use this when the caller has wired heavy subsystems (store,
    /// channels, scheduler, etc.) and just wants the boot/run
    /// facade.
    pub fn wrap(orch: Arc<Orchestrator>, port: u16) -> Arc<Self> {
        Arc::new(Self {
            orch,
            port,
            store: None,
            chat: None,
        })
    }

    /// Long-running main loop. Starts the HTTP server (so the
    /// SvelteKit build + API endpoints are reachable), then parks
    /// on the shutdown signal. Mirrors `gateway.gateway.Run()` in
    /// the CleanClaw reference.
    pub async fn run(&self) -> Result<(), GatewayError> {
        let cancel = CancellationToken::new();

        // 1. HTTP server (SvelteKit build + API + setup routes).
        //    Only bind if a store + chat service were wired in
        //    `boot()`. Tests using `wrap()` skip this.
        if let (Some(store), Some(chat)) = (self.store.as_ref(), self.chat.as_ref()) {
            let http_cancel = cancel.clone();
            let port = self.port;
            let store = store.clone();
            let chat = chat.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_http(port, store, chat, http_cancel).await {
                    tracing::error!(error = %e, "http server exited");
                }
            });
        }

        // 2. Orchestrator (dedup, eviction, inbound loop, cron).
        let _ = self.orch.clone().run(cancel.clone()).await;
        cleanclaw_daemon::shutdown_signal().await;
        cancel.cancel();
        Ok(())
    }

    /// Bind the HTTP server to `0.0.0.0:{port}` and serve until
    /// `cancel` fires. Exposed publicly so tests / alternative CLI
    /// entry points can drive the lifecycle directly.
    pub async fn start_http(&self, cancel: CancellationToken) -> Result<(), GatewayError> {
        if let (Some(store), Some(chat)) = (self.store.as_ref(), self.chat.as_ref()) {
            serve_http(self.port, store.clone(), chat.clone(), cancel).await?;
        } else {
            return Err(GatewayError::HttpServe(
                "no store/chat wired — did you call Gateway::boot?".into(),
            ));
        }
        Ok(())
    }

    pub fn orchestrator(&self) -> &Arc<Orchestrator> {
        &self.orch
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Bind `0.0.0.0:{port}` and serve the merged API + setup +
/// SvelteKit-static-fallback router until `cancel` fires.
async fn serve_http(
    port: u16,
    store: Arc<dyn cleanclaw_store::Store>,
    chat: Arc<cleanclaw_api::chat::ChatService>,
    cancel: CancellationToken,
) -> Result<(), GatewayError> {
    use std::net::SocketAddr;
    let addr: SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .map_err(GatewayError::Bind)?;

    // Auth resolver shared by both API and setup routers.
    let resolver = Arc::new(cleanclaw_auth::Resolver::new(store.clone()));

    // ---- /api/* (W1: core chat / agent / ws) ----
    let api_state = cleanclaw_api::ApiState::new(store.clone(), resolver.clone(), chat.clone());
    let api_router = cleanclaw_api::router(api_state);

    // ---- /api/* + /v1/* (W2: setup — admin, skills, tools, …) ----
    // `Server::new(store)` already builds its own internal `Accounts`
    // from the same store; we just call `router()` to get the
    // axum Router. (No `with_accounts` builder exists on the
    // public API — the constructor does the wiring.)
    let setup = cleanclaw_setup::Server::new(store.clone());
    let setup_router = setup.router();

    // Merge: setup's router takes precedence on /api/* collisions
    // (it carries the most endpoints). `mount` then adds the
    // SvelteKit static-asset fallback for non-/api routes.
    let merged = setup_router.merge(api_router);
    let app = cleanclaw_setup::mount(merged);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("cleanclaw-gateway HTTP listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
        })
        .await
        .map_err(|e| GatewayError::HttpServe(e.to_string()))?;
    Ok(())
}

/// Unix-specific SIGHUP handler. Hot-reloads every cached user
/// space so the next access picks up store mutations made by the
/// CLI or another peer. No-op on Windows (the orchestrator still
/// supports `reload_agents` programmatically; the signal isn't
/// available).
#[cfg(unix)]
pub async fn install_sighup_reload(orch: Arc<Orchestrator>) {
    use tokio::signal::unix::{signal, SignalKind};
    let mut hup = match signal(SignalKind::hangup()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "install_sighup_reload: cannot bind SIGHUP");
            return;
        }
    };
    loop {
        if hup.recv().await.is_none() {
            break;
        }
        tracing::info!("SIGHUP: hot-reloading user spaces");
        orch.reload_agents().await;
    }
}

#[cfg(not(unix))]
pub async fn install_sighup_reload(_orch: Arc<Orchestrator>) {
    // No-op on non-Unix.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_key_combines_components() {
        assert_eq!(chat_key("telegram", "bot1", "c1"), "telegram:bot1:c1");
    }

    #[test]
    fn chat_key_handles_empty_account() {
        assert_eq!(chat_key("telegram", "", "c1"), "telegram::c1");
    }

    #[tokio::test]
    async fn gateway_boot_returns_arc() {
        let g = Gateway::boot(EnvConfig::default(), 18953).await.unwrap();
        assert_eq!(g.port(), 18953);
        assert_eq!(g.orchestrator().user_space_count().await, 0);
    }
}
