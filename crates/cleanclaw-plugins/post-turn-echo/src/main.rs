//! Post-turn echo hook plugin. Mirrors
//! .
//!
//! On every `post_turn` hook fire, sends a fixed follow-up message
//! back to the same chat via the `chat.send` notification. Skeleton
//! you can copy for richer plugins (post-reply audio, translation,
//! summarization, ...).

use async_trait::async_trait;
use cleanclaw_plugin_runtime::{
    send_notification, HookRegistration, Plugin, PluginError,
};
use serde_json::Value;
use std::sync::Arc;

struct PostTurnEchoPlugin;

#[async_trait]
impl Plugin for PostTurnEchoPlugin {
    fn id(&self) -> &str {
        "post-turn-echo-demo"
    }

    async fn hook_register(&self) -> Result<HookRegistration, PluginError> {
        Ok(HookRegistration {
            points: vec!["post_turn".into()],
        })
    }

    async fn hook_fire(&self, params: Value) -> Result<(), PluginError> {
        let point = params.get("point").and_then(|v| v.as_str()).unwrap_or("");
        if point != "post_turn" {
            return Ok(());
        }
        let channel = params
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let chat_id = params
            .get("chatId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let account_id = params
            .get("accountId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        tracing::info!(
            "post-turn-echo: {channel}/{chat_id} (account={account_id})"
        );
        // Fire-and-forget: tell the host to send a follow-up.
        let _ = send_notification(
            "chat.send",
            serde_json::json!({
                "channel": channel,
                "chatId": chat_id,
                "accountId": account_id,
                "message": "👋 from post-turn-echo-demo",
            }),
        )
        .await;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    cleanclaw_plugin_runtime::run_plugin(Arc::new(PostTurnEchoPlugin)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_plugin_runtime::InProcPluginClient;

    #[tokio::test]
    async fn register_returns_post_turn() {
        let c = InProcPluginClient::spawn(PostTurnEchoPlugin);
        let reg: HookRegistration = serde_json::from_value(
            c.call("hook.register", Value::Null).await.unwrap(),
        )
        .unwrap();
        assert_eq!(reg.points, vec!["post_turn".to_string()]);
    }

    #[tokio::test]
    async fn unknown_point_is_noop() {
        let c = InProcPluginClient::spawn(PostTurnEchoPlugin);
        c.notify("hook.fire", serde_json::json!({"point": "pre_turn"})).await;
        // Notification - no reply. The hook returns Ok(()) for any
        // point != "post_turn".
    }
}
