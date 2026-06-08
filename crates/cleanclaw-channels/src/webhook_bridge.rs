//! Webhook → bus bridge. Translates platform-specific webhook
//! payloads into `InboundMessage`s and pushes them onto
//! `MessageBus`. Mirrors the dispatcher logic in
//!
//! + `DispatchLINEWebhook` — the Go gateway directly calls the
//!   channel adapter's webhook handler, while the Rust port routes
//!   the HTTP handlers (`/api/line/webhook`, `/api/feishu/webhook/:id`,
//!   `/api/telegram/webhook/:id`) through this bridge so a single
//!   `Arc<MessageBus>` instance receives every inbound.
//!
//! Why a bridge instead of per-channel handlers: the platform
//! payload formats are radically different (LINE events vs.
//! Feishu protobuf vs. Telegram Update), but every one of them
//! ultimately produces an `InboundMessage` that the
//! orchestrator's `process_inbound_loop` already knows how to
//! route. Funneling through a single bridge keeps the bus the
//! only source of truth for in-process inbound flow.

use std::sync::Arc;

use cleanclaw_bus::{InboundMessage, MessageBus};
use serde_json::Value;
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
    #[error("not configured: {0}")]
    NotConfigured(String),
    #[error("send: {0}")]
    Send(String),
}

/// Webhook → bus bridge. Construct with `WebhookBridge::new(bus)`,
/// then call the per-platform `handle_*` methods from your HTTP
/// handler.
#[derive(Clone)]
pub struct WebhookBridge {
    bus: Arc<MessageBus>,
}

impl WebhookBridge {
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self { bus }
    }

    /// Push a fully-parsed `InboundMessage` onto the bus. The
    /// helper is `pub` so platform-specific parsers (e.g. a future
    /// custom JSON shape) can bypass the per-platform wrappers.
    pub async fn dispatch(&self, msg: InboundMessage) -> Result<(), WebhookError> {
        // `send_inbound` is infallible on the Rust side (it
        // silently drops on a full channel). We surface the
        // dispatch as a Result for callers that want to log a
        // warning; the bus itself doesn't fail the call.
        self.bus.send_inbound(msg).await;
        Ok(())
    }

    /// LINE Messaging API webhook. The HTTP handler has already
    /// verified the `X-Line-Signature` HMAC via
    /// `LineChannel::verify_signature`. We just parse the events
    /// array and push each one onto the bus.
    pub async fn handle_line(&self, body: &Value, account_id: &str) -> Result<usize, WebhookError> {
        let events = body
            .get("events")
            .and_then(|e| e.as_array())
            .ok_or_else(|| WebhookError::InvalidPayload("line: missing events".into()))?;
        let mut n = 0;
        for ev in events {
            let Some(rmsg) = parse_line_event(ev, account_id) else {
                continue;
            };
            if let Err(e) = self.dispatch(rmsg).await {
                warn!(error = %e, "line webhook: dispatch failed");
            } else {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Telegram webhook (alternative to long-poll). The HTTP
    /// handler has already verified the `X-Telegram-Bot-Api-Secret-Token`
    /// header. We unwrap the `message` field and push onto the
    /// bus. Multi-update payloads are looped.
    pub async fn handle_telegram(
        &self,
        body: &Value,
        account_id: &str,
    ) -> Result<usize, WebhookError> {
        let mut n = 0;
        // Telegram delivers a single Update or a list of updates.
        if let Some(arr) = body.as_array() {
            for upd in arr {
                if let Some(m) = parse_telegram_update(upd, account_id) {
                    if self.dispatch(m).await.is_ok() {
                        n += 1;
                    }
                }
            }
        } else if let Some(m) = parse_telegram_update(body, account_id) {
            if self.dispatch(m).await.is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Feishu webhook. The HTTP handler has already verified the
    /// `Encrypt` + `Verification Token`. We unwrap the `event`
    /// sub-object for `im.message.receive_v1` and push.
    pub async fn handle_feishu(
        &self,
        body: &Value,
        account_id: &str,
    ) -> Result<usize, WebhookError> {
        let header = body.get("header");
        let event_type = header
            .and_then(|h| h.get("event_type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if event_type != "im.message.receive_v1" {
            return Ok(0);
        }
        let Some(event) = body.get("event") else {
            return Ok(0);
        };
        let Some(sender) = event
            .get("sender")
            .and_then(|s| s.get("sender_id"))
            .and_then(|s| s.get("open_id"))
            .and_then(|o| o.as_str())
        else {
            return Ok(0);
        };
        let Some(chat_id) = event.get("chat_id").and_then(|c| c.as_str()) else {
            return Ok(0);
        };
        let text = event
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let message_id = event
            .get("message")
            .and_then(|m| m.get("message_id"))
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let msg = InboundMessage {
            channel: "feishu".into(),
            account_id: account_id.into(),
            chat_id: chat_id.into(),
            user_id: sender.into(),
            message_id,
            text,
            peer_kind: if event
                .get("chat_type")
                .and_then(|t| t.as_str())
                .map(|s| s == "group" || s == "supergroup" || s == "channel")
                .unwrap_or(false)
            {
                "group".into()
            } else {
                "dm".into()
            },
            ..Default::default()
        };
        self.dispatch(msg).await?;
        Ok(1)
    }

    /// WeChat corp callback. The HTTP handler has already
    /// decrypted + verified the signature. We unwrap the
    /// XML/JSON `Message` and push.
    pub async fn handle_wechat(
        &self,
        body: &Value,
        account_id: &str,
    ) -> Result<usize, WebhookError> {
        // WeChat corp messages arrive as XML; the dashboard's
        // HTTP layer can convert to JSON. We accept either
        // form. Fields: FromUserName, ToUserName, Content,
        // MsgId, MsgType.
        let from = body
            .get("FromUserName")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = body.get("Content").and_then(|v| v.as_str()).unwrap_or("");
        let msg_id = body.get("MsgId").and_then(|v| v.as_str()).unwrap_or("");
        let msg_type = body
            .get("MsgType")
            .and_then(|v| v.as_str())
            .unwrap_or("text");
        if from.is_empty() || msg_type != "text" {
            return Ok(0);
        }
        let msg = InboundMessage {
            channel: "wechat".into(),
            account_id: account_id.into(),
            chat_id: from.into(),
            user_id: from.into(),
            message_id: msg_id.into(),
            text: content.into(),
            peer_kind: "dm".into(),
            ..Default::default()
        };
        self.dispatch(msg).await?;
        Ok(1)
    }

    /// Slack Events API webhook. The HTTP handler has already
    /// verified the `X-Slack-Signature` HMAC + `X-Slack-Request-Timestamp`
    /// (the slack-verify helper does that in the channels
    /// crate); we just parse the Events API envelope and push
    /// message events onto the bus.
    pub async fn handle_slack(
        &self,
        body: &Value,
        account_id: &str,
    ) -> Result<usize, WebhookError> {
        // URL verification handshake.
        if body.get("type").and_then(|t| t.as_str()) == Some("url_verification") {
            // The challenge body comes through this same path;
            // the HTTP handler echoes it back before consulting
            // the bridge. We just return 0 dispatched here.
            return Ok(0);
        }
        // Event callback: `event.type == "message"` for
        // user-typed messages, `event.type == "app_mention"` for
        // @-bot mentions. Both share the same `event.channel`
        // + `event.text` + `event.user` shape.
        let ev = body.get("event");
        let Some(ev) = ev else {
            return Ok(0);
        };
        let ev_type = ev.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ev_type != "message" && ev_type != "app_mention" {
            return Ok(0);
        }
        // Slack message subtypes we don't handle (edits,
        // deletes, joins, etc.) carry `subtype`. The plain
        // "message" event has no `subtype` field.
        if ev.get("subtype").is_some() {
            return Ok(0);
        }
        let channel_id = ev.get("channel").and_then(|c| c.as_str()).unwrap_or("");
        let user_id = ev.get("user").and_then(|u| u.as_str()).unwrap_or("");
        let text = ev
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let ts = ev.get("ts").and_then(|t| t.as_str()).unwrap_or("");
        if channel_id.is_empty() || user_id.is_empty() {
            return Ok(0);
        }
        // Slack DMs have a `channel_type` of "im"; channels /
        // groups are "channel" / "group" / "mpim".
        let channel_type = ev
            .get("channel_type")
            .and_then(|c| c.as_str())
            .unwrap_or("channel");
        let peer_kind = if channel_type == "im" { "dm" } else { "group" }.to_string();
        let msg = InboundMessage {
            channel: "slack".into(),
            account_id: account_id.into(),
            chat_id: channel_id.into(),
            user_id: user_id.into(),
            message_id: ts.into(),
            text,
            peer_kind,
            ..Default::default()
        };
        self.dispatch(msg).await?;
        Ok(1)
    }

    /// Discord Interaction webhook. The HTTP handler has
    /// already verified the `X-Signature-Ed25519` header (the
    /// discord-verify helper does that in the channels crate).
    /// We accept both Interaction (slash command + message
    /// component) and Gateway message payloads; the latter is
    /// what a real Discord bot sees if the operator forwards
    /// gateway events to a webhook.
    pub async fn handle_discord(
        &self,
        body: &Value,
        _account_id: &str,
    ) -> Result<usize, WebhookError> {
        // Discord webhook body shape: `{type, data, channel_id,
        // user, message, ...}`. For a Message Create event
        // forwarded through a webhook proxy, the body has
        // `t=MESSAGE_CREATE` and `d={...}`. For raw messages,
        // the body has the message directly.
        let (msg, guild_id) = if body.get("t").is_some() && body.get("d").is_some() {
            // Gateway event envelope.
            let d = body.get("d").unwrap();
            let guild_id = d
                .get("guild_id")
                .and_then(|g| g.as_str())
                .map(|s| s.to_string());
            let m = extract_discord_message(d);
            (m, guild_id)
        } else {
            let m = extract_discord_message(body);
            (m, None)
        };
        let Some(msg) = msg else {
            return Ok(0);
        };
        let _ = guild_id; // reserved for future per-guild routing
        self.dispatch(msg).await?;
        Ok(1)
    }
}

fn extract_discord_message(d: &Value) -> Option<InboundMessage> {
    let channel_id = d.get("channel_id").and_then(|c| c.as_str()).unwrap_or("");
    let author = d
        .get("author")
        .or_else(|| d.get("member").and_then(|m| m.get("user")))?;
    let user_id = author.get("id").and_then(|i| i.as_str()).unwrap_or("");
    let username = author
        .get("username")
        .and_then(|u| u.as_str())
        .unwrap_or("");
    let text = d
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let message_id = d
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    if channel_id.is_empty() {
        return None;
    }
    // Discord has no built-in DM/group distinction in the
    // message payload — guild_id presence tells us it's a
    // guild channel; absence implies a DM. We default to
    // 'group' since most bot chats are guild-side.
    let peer_kind = if d.get("guild_id").is_some() {
        "group"
    } else {
        "dm"
    }
    .to_string();
    Some(InboundMessage {
        channel: "discord".into(),
        account_id: String::new(), // overridden by HTTP layer via path param
        chat_id: channel_id.into(),
        user_id: user_id.into(),
        message_id,
        text,
        sender_name: username.into(),
        peer_kind,
        ..Default::default()
    })
}

fn parse_line_event(ev: &Value, account_id: &str) -> Option<InboundMessage> {
    let ev_type = ev.get("type").and_then(|t| t.as_str())?;
    if ev_type != "message" {
        return None;
    }
    let source = ev.get("source")?;
    let chat_id = source
        .get("groupId")
        .or_else(|| source.get("roomId"))
        .or_else(|| source.get("userId"))
        .and_then(|s| s.as_str())?
        .to_string();
    let user_id = source
        .get("userId")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let text = ev
        .get("message")
        .and_then(|m| m.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let message_id = ev
        .get("message")
        .and_then(|m| m.get("id"))
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    let peer_kind = if source.get("groupId").is_some() || source.get("roomId").is_some() {
        "group"
    } else {
        "dm"
    }
    .to_string();
    Some(InboundMessage {
        channel: "line".into(),
        account_id: account_id.into(),
        chat_id,
        user_id,
        message_id,
        text,
        peer_kind,
        ..Default::default()
    })
}

fn parse_telegram_update(upd: &Value, account_id: &str) -> Option<InboundMessage> {
    let msg = upd.get("message").or_else(|| upd.get("edited_message"))?;
    let chat = msg.get("chat")?;
    let chat_id = chat.get("id")?.to_string();
    let text = msg
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let message_id = msg
        .get("message_id")
        .map(|m| m.to_string())
        .unwrap_or_default();
    let sender_name = msg
        .get("from")
        .and_then(|f| f.get("username"))
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let peer_kind = match chat.get("type").and_then(|t| t.as_str()) {
        Some("private") => "dm",
        _ => "group",
    }
    .to_string();
    Some(InboundMessage {
        channel: "telegram".into(),
        account_id: account_id.into(),
        chat_id,
        user_id: sender_name.clone(),
        message_id,
        text,
        sender_name,
        peer_kind,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_bus() -> Arc<MessageBus> {
        Arc::new(MessageBus::new(16))
    }

    #[tokio::test]
    async fn dispatch_pushes_to_bus() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let r = bridge
            .dispatch(InboundMessage {
                channel: "test".into(),
                account_id: "a".into(),
                chat_id: "c".into(),
                text: "hi".into(),
                ..Default::default()
            })
            .await;
        assert!(r.is_ok());
        let got = bus.recv_inbound().await;
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.text, "hi");
    }

    #[tokio::test]
    async fn line_webhook_parses_message() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "events": [{
                "type": "message",
                "source": {"userId": "U123", "groupId": "G456"},
                "message": {"id": "m1", "text": "hello"}
            }]
        });
        let n = bridge.handle_line(&body, "bot1").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.channel, "line");
        assert_eq!(got.text, "hello");
        assert_eq!(got.peer_kind, "group");
    }

    #[tokio::test]
    async fn line_webhook_ignores_non_message_events() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "events": [
                {"type": "follow", "source": {"userId": "U123"}},
                {"type": "message", "source": {"userId": "U456"}, "message": {"id": "m1", "text": "hi"}}
            ]
        });
        let n = bridge.handle_line(&body, "bot1").await.unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn telegram_webhook_parses_message() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "update_id": 1,
            "message": {
                "message_id": 42,
                "from": {"username": "alice"},
                "chat": {"id": 12345, "type": "private"},
                "text": "hi telegram"
            }
        });
        let n = bridge.handle_telegram(&body, "bot1").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.channel, "telegram");
        assert_eq!(got.peer_kind, "dm");
        assert_eq!(got.text, "hi telegram");
    }

    #[tokio::test]
    async fn telegram_webhook_group_message() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "message": {
                "message_id": 1,
                "chat": {"id": -1001, "type": "supergroup"},
                "from": {"username": "bob"},
                "text": "@bot ping"
            }
        });
        let n = bridge.handle_telegram(&body, "bot1").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.peer_kind, "group");
    }

    #[tokio::test]
    async fn feishu_webhook_message_receive_v1() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1",
                "app_id": "cli_xxx",
                "tenant_key": "yyy"
            },
            "event": {
                "sender": {"sender_id": {"open_id": "ou_abc"}},
                "chat_id": "oc_xyz",
                "chat_type": "dm",
                "message": {
                    "message_id": "om_123",
                    "content": {"text": "hello feishu"}
                }
            }
        });
        let n = bridge.handle_feishu(&body, "cli_xxx").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.channel, "feishu");
        assert_eq!(got.peer_kind, "dm");
        assert_eq!(got.text, "hello feishu");
    }

    #[tokio::test]
    async fn feishu_webhook_ignores_other_event_types() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "header": {"event_type": "im.message.message_read_v1"},
            "event": {}
        });
        let n = bridge.handle_feishu(&body, "cli_xxx").await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn wechat_webhook_parses_text() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "FromUserName": "user_openid",
            "ToUserName": "corp_id",
            "MsgType": "text",
            "Content": "hello wechat",
            "MsgId": "1234567890"
        });
        let n = bridge.handle_wechat(&body, "corp_id").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.channel, "wechat");
        assert_eq!(got.text, "hello wechat");
        assert_eq!(got.message_id, "1234567890");
    }

    #[tokio::test]
    async fn wechat_webhook_ignores_non_text() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "FromUserName": "user_openid",
            "MsgType": "image",
            "MediaId": "m1"
        });
        let n = bridge.handle_wechat(&body, "corp_id").await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn slack_webhook_parses_message_event() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C123",
                "channel_type": "im",
                "user": "U456",
                "text": "hi slack",
                "ts": "1234567890.123"
            }
        });
        let n = bridge.handle_slack(&body, "T_BOT").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.channel, "slack");
        assert_eq!(got.peer_kind, "dm");
        assert_eq!(got.text, "hi slack");
        assert_eq!(got.message_id, "1234567890.123");
    }

    #[tokio::test]
    async fn slack_webhook_group_channel() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C789",
                "channel_type": "channel",
                "user": "U001",
                "text": "@bot ping",
                "ts": "1.0"
            }
        });
        let n = bridge.handle_slack(&body, "T_BOT").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.peer_kind, "group");
    }

    #[tokio::test]
    async fn slack_webhook_ignores_subtype() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "subtype": "message_changed",
                "channel": "C123",
                "user": "U456",
                "text": "edit",
                "ts": "1.0"
            }
        });
        let n = bridge.handle_slack(&body, "T_BOT").await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn slack_webhook_url_verification_returns_zero() {
        // The HTTP handler echoes the challenge; the bridge
        // reports 0 dispatched.
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "type": "url_verification",
            "challenge": "abc",
            "token": "..."
        });
        let n = bridge.handle_slack(&body, "T_BOT").await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn discord_webhook_parses_message() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "channel_id": "12345",
            "author": {"id": "999", "username": "alice"},
            "content": "hi discord",
            "id": "msg_1",
            "guild_id": "G1"
        });
        let n = bridge.handle_discord(&body, "BOT").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.channel, "discord");
        assert_eq!(got.peer_kind, "group");
        assert_eq!(got.text, "hi discord");
    }

    #[tokio::test]
    async fn discord_webhook_parses_dm() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "channel_id": "D1",
            "author": {"id": "999", "username": "bob"},
            "content": "dm message",
            "id": "msg_2"
        });
        let n = bridge.handle_discord(&body, "BOT").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.peer_kind, "dm");
    }

    #[tokio::test]
    async fn discord_webhook_gateway_envelope() {
        // Discord sometimes forwards gateway events through a
        // webhook proxy — the body has `t` + `d` envelope.
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "t": "MESSAGE_CREATE",
            "d": {
                "channel_id": "C1",
                "author": {"id": "U1", "username": "carol"},
                "content": "via gateway",
                "id": "msg_3",
                "guild_id": "G1"
            }
        });
        let n = bridge.handle_discord(&body, "BOT").await.unwrap();
        assert_eq!(n, 1);
        let got = bus.recv_inbound().await.unwrap();
        assert_eq!(got.text, "via gateway");
    }

    #[tokio::test]
    async fn discord_webhook_skips_empty_channel() {
        let bus = fresh_bus();
        let bridge = WebhookBridge::new(bus.clone());
        let body = json!({
            "channel_id": "",
            "author": {"id": "U1", "username": "x"},
            "content": "no channel"
        });
        let n = bridge.handle_discord(&body, "BOT").await.unwrap();
        assert_eq!(n, 0);
    }
}
