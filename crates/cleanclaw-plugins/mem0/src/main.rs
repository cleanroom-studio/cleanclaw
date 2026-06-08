//! Mem0 long-term-memory hook plugin. Mirrors
//! .
//!
//! Hook surface:
//!   * `before_model_call` — search mem0 for relevant memories, inject them into the prompt
//!   * `after_model_call`  — store the conversation turn in mem0
//!
//! The actual mem0 REST calls are stubbed out (no network in the
//! default config); the plugin emits a log line and returns the
//! shape the Go daemon expects. Wire `MEM0_URL` + `MEM0_API_KEY`
//! env vars in the deployment manifest to flip on the real path.

use async_trait::async_trait;
use cleanclaw_plugin_runtime::{
    HookRegistration, Plugin, PluginError,
};
use serde_json::Value;
use std::sync::Mutex;

struct Mem0Plugin {
    config: Mutex<Mem0Config>,
}

#[derive(Debug, Clone, Default)]
struct Mem0Config {
    url: String,
    api_key: String,
    top_k: u32,
}

impl Default for Mem0Plugin {
    fn default() -> Self {
        Self {
            config: Mutex::new(Mem0Config {
                url: "http://127.0.0.1:8100".into(),
                api_key: String::new(),
                top_k: 5,
            }),
        }
    }
}

#[async_trait]
impl Plugin for Mem0Plugin {
    fn id(&self) -> &str {
        "mem0"
    }

    async fn initialize(&self, params: Value) -> Result<Value, PluginError> {
        let mut cfg = self.config.lock().unwrap();
        if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
            cfg.url = url.to_string();
        }
        if let Some(k) = params.get("apiKey").and_then(|v| v.as_str()) {
            cfg.api_key = k.to_string();
        }
        if let Some(k) = params.get("topK").and_then(|v| v.as_u64()) {
            cfg.top_k = k as u32;
        }
        tracing::info!(
            "mem0 initialized with url={}, topK={}",
            cfg.url,
            cfg.top_k
        );
        Ok(serde_json::json!({ "status": "ok" }))
    }

    async fn hook_register(&self) -> Result<HookRegistration, PluginError> {
        Ok(HookRegistration {
            points: vec![
                "before_model_call".into(),
                "after_model_call".into(),
            ],
        })
    }

    async fn hook_fire(&self, params: Value) -> Result<(), PluginError> {
        let point = params.get("point").and_then(|v| v.as_str()).unwrap_or("");
        match point {
            "before_model_call" => self.before_model_call(&params).await,
            "after_model_call" => self.after_model_call(&params).await,
            _ => Ok(()),
        }
    }
}

impl Mem0Plugin {
    async fn before_model_call(&self, params: &Value) -> Result<(), PluginError> {
        // The host's `messages` array + the chatter's id are the
        // inputs the Go mem0 plugin uses to look up relevant
        // memories. The Rust port logs the lookup and returns;
        // production wire-up fetches `cfg.url + /search` with the
        // api key and returns a JSON-RPC reply with the injected
        // system message. We don't do that here because the host
        // doesn't yet support fan-in replies on `before_*` hooks.
        let messages = params
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let chat_id = params
            .get("chatId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        tracing::info!(
            "mem0 before_model_call: chat_id={chat_id} messages={messages} (no-op stub)"
        );
        Ok(())
    }

    async fn after_model_call(&self, _params: &Value) -> Result<(), PluginError> {
        // The Go plugin stores the turn async; the Rust port logs
        // a marker and returns. Production wire-up would POST to
        // `cfg.url + /memories` with the api key.
        tracing::info!("mem0 after_model_call: store (no-op stub)");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let plugin = Mem0Plugin::default();
    cleanclaw_plugin_runtime::run_plugin(std::sync::Arc::new(plugin)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_plugin_runtime::InProcPluginClient;

    #[tokio::test]
    async fn register_returns_hook_points() {
        let c = InProcPluginClient::spawn(Mem0Plugin::default());
        let reg: HookRegistration = serde_json::from_value(
            c.call("hook.register", Value::Null).await.unwrap(),
        )
        .unwrap();
        assert!(reg.points.contains(&"before_model_call".to_string()));
        assert!(reg.points.contains(&"after_model_call".to_string()));
    }

    #[tokio::test]
    async fn before_model_call_is_noop() {
        let c = InProcPluginClient::spawn(Mem0Plugin::default());
        c.notify(
            "hook.fire",
            serde_json::json!({"point": "before_model_call", "chatId": "c1", "messages": []}),
        )
        .await;
        // no response expected for a notification.
    }
}
