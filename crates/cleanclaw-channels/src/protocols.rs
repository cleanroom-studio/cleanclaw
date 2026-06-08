//! Real-time channel protocol shapes. The `start()` path in each
//! channel adapter still relies on long-poll / liveness probes for
//! offline builds; the modules here provide the wire-format
//! primitives a follow-up gateway WS impl will need.
//!
//!
//! for the protocol envelope layer.

use serde::{Deserialize, Serialize};

// =====================================================================
// Discord Gateway v10
// =====================================================================
//
// The Discord gateway sends two flavors of payload:
//   1. `HELLO` (op=10) — client must reply with `IDENTIFY` (op=2)
//   2. `MESSAGE_CREATE` (op=0, t="MESSAGE_CREATE") — new message
//
// The CleanClaw `internal/channels/discord.go` keeps a single
// persistent WS connection. For our offline-friendly Rust port
// we expose:
//   - the URL builder (Discord's gateway URL is normally fetched
//     from `/api/v10/gateway/bot`)
//   - the HELLO/IDENTIFY/MESSAGE_CREATE envelope structs
//   - a heartbeat helper

pub const DISCORD_GATEWAY_VERSION: u8 = 10;
pub const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// `GET /api/v10/gateway/bot` → `{ "url": "wss://gateway.discord.gg" }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayBotResponse {
    pub url: String,
    #[serde(default)]
    pub shards: u8,
    #[serde(default)]
    pub session_start_limit: Option<SessionStartLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartLimit {
    pub total: u32,
    pub remaining: u32,
    pub reset_after: u64,
    pub max_concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloPayload {
    pub op: u8, // always 10
    pub d: HelloData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloData {
    pub heartbeat_interval: u64,
    #[serde(default)]
    pub _trace: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifyPayload {
    pub op: u8, // always 2
    pub d: IdentifyData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifyData {
    pub token: String,
    pub intents: u32,
    pub shard: [u8; 2],
    pub presence: PresenceUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub since: u64,
    pub activities: Vec<serde_json::Value>,
    pub status: String,
    pub afk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub op: u8,         // always 1
    pub d: Option<i64>, // last sequence number, or null
}

/// Dispatch event envelope. `op=0` is the only "data" opcode; the
/// `t` field is the event name (`MESSAGE_CREATE`, `READY`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dispatch {
    pub op: u8,         // 0
    pub s: Option<u64>, // sequence
    pub t: String,      // event name
    pub d: serde_json::Value,
}

/// Subset of MESSAGE_CREATE we care about. The CleanClaw
/// `discord.go` reduces the full payload to this surface; matches
/// the `InboundMessage` the gateway bus expects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageCreate {
    pub id: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub author: Author,
    pub content: String,
    pub timestamp: String,
    #[serde(default)]
    pub attachments: Vec<serde_json::Value>,
    #[serde(default)]
    pub referenced_message: Option<Box<MessageCreate>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub username: String,
    pub discriminator: Option<String>,
    pub bot: bool,
}

impl MessageCreate {
    /// Reduce the full payload to an `InboundMessage` shape the
    /// gateway bus can route. Group vs DM keying is the channel
    /// adapter's responsibility (not this module's).
    pub fn chat_id(&self) -> &str {
        &self.channel_id
    }
    pub fn user_id(&self) -> &str {
        &self.author.id
    }
    pub fn text(&self) -> &str {
        &self.content
    }
    pub fn is_group(&self) -> bool {
        self.guild_id.is_some()
    }
}

// =====================================================================
// Slack Socket Mode
// =====================================================================
//
// Socket Mode is a WSS push channel: Slack opens a WS to your
// app, sends `hello`, the app acks with `hello`, then events
// arrive as JSON envelopes with a `payload` of `events_api`.
// shapes.

pub const SLACK_SOCKET_MODE_URL: &str = "https://slack.com/api/apps.connections.open";

/// Envelope sent over the WSS. `envelope_id` is the dedup key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackEnvelope {
    pub envelope_id: String,
    pub r#type: String, // "events_api", "interactive", "slash_commands", …
    pub payload: serde_json::Value,
    #[serde(default)]
    pub accepts_response_payload: bool,
}

/// `events_api` payload wraps a Slack `Event` (the body delivered
/// to the Events API endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackEvent {
    pub r#type: String, // "message", "app_mention", …
    pub event: serde_json::Value,
    pub team_id: String,
    pub event_time: u64,
    #[serde(default)]
    pub authed_users: Vec<String>,
}

/// The `event.message` shape we use to populate `InboundMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackMessageEvent {
    pub r#type: String, // "message"
    pub subtype: Option<String>,
    pub channel: String,
    pub user: String,
    pub text: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub channel_type: String, // "channel" | "group" | "im" | "mpim"
}

impl SlackMessageEvent {
    pub fn is_group(&self) -> bool {
        matches!(self.channel_type.as_str(), "channel" | "group" | "mpim")
    }
}

// =====================================================================
// Feishu (Lark) long-conn WSS
// =====================================================================
//
// Feishu's open-platform pushes events over a long-connection
// WSS to the app server. The protocol is its own beast: a
// custom binary header (4 bytes), then protobuf OR JSON
// envelopes with a `type` + `msg` body. We use JSON here since
// the Go daemon already accepts JSON mode.

pub const FEISHU_OPEN_HOST: &str = "https://open.feishu.cn";
pub const FEISHU_WSS_URL: &str = "wss://open.feishu.cn/open-apis/gateway/v1/connect";

/// Feishu envelope. The header is binary in production, but for
/// JSON mode every message has this shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuEnvelope {
    /// Message id (uuid).
    pub msg_id: String,
    /// Sequence number within the connection.
    pub sn: u64,
    /// `type` is one of: "event", "ping", "pong", "system", …
    pub r#type: String,
    /// Message body. For "event" messages this is the bot's
    /// typed event payload.
    pub payload: serde_json::Value,
}

/// Subset of `im.message.receive_v1` we care about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuMessageEvent {
    pub sender: FeishuSender,
    pub message: FeishuMessage,
    pub chat_id: String,
    pub chat_type: String, // "p2p" | "group"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuSender {
    pub sender_id: FeishuId,
    pub sender_type: String, // "user" | "app" | …
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuId {
    pub open_id: String,
    pub union_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuMessage {
    pub message_id: String,
    pub root_id: Option<String>,
    pub create_time: String,
    pub chat_id: String,
    pub message_type: String, // "text" | "image" | "post" | …
    pub content: String,      // JSON-encoded body
}

impl FeishuMessageEvent {
    pub fn is_group(&self) -> bool {
        self.chat_type == "group"
    }
    pub fn user_id(&self) -> &str {
        &self.sender.sender_id.open_id
    }
    pub fn text(&self) -> String {
        // The `content` field is a JSON object as a string;
        // for `text` messages it looks like `{"text": "..."}`.
        if self.message.message_type == "text" {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&self.message.content) {
                if let Some(s) = parsed.get("text").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        self.message.content.clone()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discord_gateway_bot_response_parses() {
        let raw = json!({
            "url": "wss://gateway.discord.gg",
            "shards": 1,
            "session_start_limit": {
                "total": 1000,
                "remaining": 999,
                "reset_after": 14400000_i64,
                "max_concurrency": 1
            }
        });
        let parsed: GatewayBotResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.url, "wss://gateway.discord.gg");
        assert_eq!(parsed.shards, 1);
        let s = parsed.session_start_limit.unwrap();
        assert_eq!(s.total, 1000);
    }

    #[test]
    fn discord_hello_envelope_round_trip() {
        let hello = HelloPayload {
            op: 10,
            d: HelloData {
                heartbeat_interval: 41250,
                _trace: vec!["[\"gateway\"]".into()],
            },
        };
        let s = serde_json::to_string(&hello).unwrap();
        let back: HelloPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.op, 10);
        assert_eq!(back.d.heartbeat_interval, 41250);
    }

    #[test]
    fn discord_identify_envelope_round_trip() {
        let id = IdentifyPayload {
            op: 2,
            d: IdentifyData {
                token: "Bot abc".into(),
                intents: 513,
                shard: [0, 1],
                presence: PresenceUpdate {
                    since: 0,
                    activities: vec![],
                    status: "online".into(),
                    afk: false,
                },
            },
        };
        let s = serde_json::to_string(&id).unwrap();
        let back: IdentifyPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.d.token, "Bot abc");
        assert_eq!(back.d.intents, 513);
        assert_eq!(back.d.shard, [0, 1]);
    }

    #[test]
    fn discord_message_create_extracts_fields() {
        let raw = json!({
            "id": "1111",
            "channel_id": "2222",
            "guild_id": "3333",
            "author": {
                "id": "u1",
                "username": "alice",
                "bot": false
            },
            "content": "hello world",
            "timestamp": "2024-01-01T00:00:00Z"
        });
        let m: MessageCreate = serde_json::from_value(raw).unwrap();
        assert_eq!(m.chat_id(), "2222");
        assert_eq!(m.user_id(), "u1");
        assert_eq!(m.text(), "hello world");
        assert!(m.is_group());
    }

    #[test]
    fn discord_message_create_dm_has_no_guild() {
        let raw = json!({
            "id": "1",
            "channel_id": "dm1",
            "author": { "id": "u1", "username": "x", "bot": false },
            "content": "hi",
            "timestamp": "t"
        });
        let m: MessageCreate = serde_json::from_value(raw).unwrap();
        assert!(!m.is_group());
    }

    #[test]
    fn slack_envelope_round_trip() {
        let env = SlackEnvelope {
            envelope_id: "abc-123".into(),
            r#type: "events_api".into(),
            payload: json!({ "event": { "type": "message" } }),
            accepts_response_payload: true,
        };
        let s = serde_json::to_string(&env).unwrap();
        let back: SlackEnvelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.envelope_id, "abc-123");
        assert_eq!(back.r#type, "events_api");
    }

    #[test]
    fn slack_message_event_is_group_for_channel() {
        let raw = json!({
            "type": "message",
            "channel": "C1",
            "user": "U1",
            "text": "hi",
            "ts": "1.0",
            "channel_type": "channel"
        });
        let m: SlackMessageEvent = serde_json::from_value(raw).unwrap();
        assert!(m.is_group());
        assert_eq!(m.channel, "C1");
    }

    #[test]
    fn slack_message_event_dm_is_not_group() {
        let raw = json!({
            "type": "message",
            "channel": "D1",
            "user": "U1",
            "text": "hi",
            "ts": "1.0",
            "channel_type": "im"
        });
        let m: SlackMessageEvent = serde_json::from_value(raw).unwrap();
        assert!(!m.is_group());
    }

    #[test]
    fn feishu_message_event_text_extraction() {
        let raw = json!({
            "sender": {
                "sender_id": { "open_id": "ou_1" },
                "sender_type": "user"
            },
            "message": {
                "message_id": "om_1",
                "create_time": "1700000000",
                "chat_id": "oc_1",
                "message_type": "text",
                "content": "{\"text\":\"hello\"}"
            },
            "chat_id": "oc_1",
            "chat_type": "p2p"
        });
        let m: FeishuMessageEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(m.text(), "hello");
        assert_eq!(m.user_id(), "ou_1");
        assert!(!m.is_group());
    }

    #[test]
    fn feishu_message_event_group_chat() {
        let raw = json!({
            "sender": {
                "sender_id": { "open_id": "ou_1" },
                "sender_type": "user"
            },
            "message": {
                "message_id": "om_1",
                "create_time": "t",
                "chat_id": "oc_2",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            },
            "chat_id": "oc_2",
            "chat_type": "group"
        });
        let m: FeishuMessageEvent = serde_json::from_value(raw).unwrap();
        assert!(m.is_group());
    }

    #[test]
    fn feishu_envelope_round_trip() {
        let env = FeishuEnvelope {
            msg_id: "m_1".into(),
            sn: 42,
            r#type: "event".into(),
            payload: json!({"event": {"type": "im.message.receive_v1"}}),
        };
        let s = serde_json::to_string(&env).unwrap();
        let back: FeishuEnvelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.sn, 42);
        assert_eq!(back.r#type, "event");
    }

    #[test]
    fn discord_heartbeat_payload_with_null_d() {
        let h = HeartbeatPayload { op: 1, d: None };
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(s, r#"{"op":1,"d":null}"#);
    }

    #[test]
    fn discord_dispatch_envelope_round_trip() {
        let d = Dispatch {
            op: 0,
            s: Some(7),
            t: "MESSAGE_CREATE".into(),
            d: json!({"id": "1", "content": "x"}),
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: Dispatch = serde_json::from_str(&s).unwrap();
        assert_eq!(back.s, Some(7));
        assert_eq!(back.t, "MESSAGE_CREATE");
    }
}
