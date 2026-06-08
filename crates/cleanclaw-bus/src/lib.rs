//! In-process message bus.
//!
//! Inbound / outbound channels are bounded; consumers run as
//! tokio tasks that pull work and route it to a handler.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub const SOURCE_USER: &str = "";
pub const SOURCE_CRON: &str = "cron";
pub const SOURCE_HEARTBEAT: &str = "heartbeat";
pub const SOURCE_SUBAGENT: &str = "subagent";
pub const SOURCE_GOAL_CONTEXT: &str = "goal_context";

/// Inline-keyboard button. Exactly one of `callback_data` / `url` is
/// normally populated by the channel; `text` is always required.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboundButton {
    pub text: String,
    #[serde(default)]
    pub callback_data: String,
    #[serde(default)]
    pub url: String,
}

impl OutboundButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    pub fn with_callback(mut self, data: impl Into<String>) -> Self {
        self.callback_data = data.into();
        self
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    #[serde(default)]
    pub project_id: String,
    pub user_id: String,
    pub owner_user_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub message_id: String,
    pub text: String,
    #[serde(default)]
    pub peer_kind: String,
    #[serde(default)]
    pub sender_name: String,
    #[serde(default)]
    pub sender_avatar_url: String,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub is_bot_message: bool,
    #[serde(default)]
    pub photo_url: String,
    #[serde(default)]
    pub photo_urls: Vec<String>,
    #[serde(default)]
    pub reply_to_msg_id: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaItem {
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
    pub data_base64: String,
}

impl MediaItem {
    pub fn from_bytes(filename: impl Into<String>, bytes: &[u8]) -> Self {
        use base64::Engine;
        Self {
            filename: filename.into(),
            content_type: String::new(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&self.data_base64)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub account_id: String,
    pub agent_id: String,
    pub chat_id: String,
    pub text: String,
    #[serde(default)]
    pub reply_to_msg_id: String,
    #[serde(default)]
    pub parse_mode: String,
    #[serde(default)]
    pub buttons: Vec<Vec<OutboundButton>>,
    #[serde(default)]
    pub edit_msg_id: String,
    #[serde(default)]
    pub media_paths: Vec<String>,
    #[serde(default)]
    pub media_items: Vec<MediaItem>,
    #[serde(default)]
    pub allow_split: bool,
}

#[derive(Clone)]
pub struct MessageBus {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    pub inbound_rx: Arc<Mutex<mpsc::Receiver<InboundMessage>>>,
    pub outbound_tx: mpsc::Sender<OutboundMessage>,
    pub outbound_rx: Arc<Mutex<mpsc::Receiver<OutboundMessage>>>,
}

impl MessageBus {
    pub fn new(capacity: usize) -> Self {
        let (itx, irx) = mpsc::channel(capacity);
        let (otx, orx) = mpsc::channel(capacity);
        Self {
            inbound_tx: itx,
            inbound_rx: Arc::new(Mutex::new(irx)),
            outbound_tx: otx,
            outbound_rx: Arc::new(Mutex::new(orx)),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(100)
    }

    pub async fn send_inbound(&self, m: InboundMessage) {
        let _ = self.inbound_tx.send(m).await;
    }

    pub async fn send_outbound(&self, m: OutboundMessage) {
        let _ = self.outbound_tx.send(m).await;
    }

    pub async fn try_send_inbound(&self, m: InboundMessage) -> Result<(), mpsc::error::TrySendError<InboundMessage>> {
        self.inbound_tx.try_send(m)
    }

    pub async fn try_send_outbound(
        &self,
        m: OutboundMessage,
    ) -> Result<(), mpsc::error::TrySendError<OutboundMessage>> {
        self.outbound_tx.try_send(m)
    }

    pub async fn recv_inbound(&self) -> Option<InboundMessage> {
        self.inbound_rx.lock().await.recv().await
    }

    pub async fn recv_outbound(&self) -> Option<OutboundMessage> {
        self.outbound_rx.lock().await.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_user_is_empty_for_backcompat() {
        // Mirrors bus_test.go::TestSourceUserIsEmpty
        assert_eq!(SOURCE_USER, "");
        let m = InboundMessage::default();
        assert_eq!(m.source, SOURCE_USER);
    }

    #[test]
    fn source_constants_are_distinct() {
        let all = [
            ("SourceUser", SOURCE_USER),
            ("SourceCron", SOURCE_CRON),
            ("SourceHeartbeat", SOURCE_HEARTBEAT),
            ("SourceSubAgent", SOURCE_SUBAGENT),
            ("SourceGoalContext", SOURCE_GOAL_CONTEXT),
        ];
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (name, val) in all {
            if let Some(prev) = seen.get(val) {
                panic!("{} and {} both equal {:?} — Source constants must be distinct", prev, name, val);
            }
            seen.insert(val, name);
        }
    }

    #[test]
    fn outbound_button_builders() {
        let cb = OutboundButton::new("Yes").with_callback("yes");
        assert_eq!(cb.text, "Yes");
        assert_eq!(cb.callback_data, "yes");
        assert_eq!(cb.url, "");

        let url = OutboundButton::new("Docs").with_url("https://example.com");
        assert_eq!(url.url, "https://example.com");
        assert_eq!(url.callback_data, "");
    }

    #[test]
    fn media_item_roundtrips_bytes() {
        let payload = b"hello world";
        let m = MediaItem::from_bytes("note.txt", payload);
        assert_eq!(m.filename, "note.txt");
        let back = m.decode().expect("valid base64");
        assert_eq!(back, payload);
    }

    #[test]
    fn outbound_message_roundtrips_buttons() {
        let mut msg = OutboundMessage::default();
        msg.channel = "telegram".into();
        msg.chat_id = "42".into();
        msg.text = "Pick one".into();
        msg.buttons = vec![vec![
            OutboundButton::new("Yes").with_callback("y"),
            OutboundButton::new("No").with_callback("n"),
        ]];
        msg.edit_msg_id = "99".into();
        msg.media_paths = vec!["/tmp/out.png".into()];

        let json = serde_json::to_string(&msg).expect("serialize");
        let back: OutboundMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.buttons.len(), 1);
        assert_eq!(back.buttons[0].len(), 2);
        assert_eq!(back.buttons[0][0].callback_data, "y");
        assert_eq!(back.edit_msg_id, "99");
        assert_eq!(back.media_paths, vec!["/tmp/out.png".to_string()]);
    }

    #[test]
    fn inbound_message_carries_photo_legacy_and_modern_fields() {
        let mut m = InboundMessage::default();
        m.channel = "telegram".into();
        m.photo_url = "https://x/a.jpg".into();
        m.photo_urls = vec!["https://x/b.jpg".into(), "https://x/c.jpg".into()];
        m.sender_avatar_url = "https://cdn/avatar.png".into();
        m.reply_to_msg_id = "123".into();

        let json = serde_json::to_string(&m).unwrap();
        let back: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.photo_url, "https://x/a.jpg");
        assert_eq!(back.photo_urls.len(), 2);
        assert_eq!(back.sender_avatar_url, "https://cdn/avatar.png");
        assert_eq!(back.reply_to_msg_id, "123");
    }

    #[tokio::test]
    async fn bus_round_trip_inbound_and_outbound() {
        let bus = MessageBus::new(8);
        bus.send_inbound(InboundMessage {
            channel: "web".into(),
            text: "hi".into(),
            ..Default::default()
        })
        .await;
        let got = bus.recv_inbound().await.expect("inbound msg");
        assert_eq!(got.text, "hi");

        bus.send_outbound(OutboundMessage {
            channel: "web".into(),
            text: "hello".into(),
            ..Default::default()
        })
        .await;
        let got = bus.recv_outbound().await.expect("outbound msg");
        assert_eq!(got.text, "hello");
    }
}
