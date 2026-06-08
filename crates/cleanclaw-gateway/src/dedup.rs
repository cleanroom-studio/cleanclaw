//! `Dedup` — suppress duplicate inbound messages within a 60s window.
//!
//! Two keying strategies because the failure modes differ:
//!   - **Group**: Telegram supergroups deliver the same logical message
//!     to each bot with a *different* `message_id`, so `message_id` can't
//!     dedup across bot copies. Key on
//!     `(channel, chat_id, user_id, text-hash)` instead.
//!   - **DM**: every supported IM channel emits a stable per-conversation
//!     `message_id`. Key on `(channel, account_id, message_id)` so the
//!     same inbound being pushed twice drops here.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cleanclaw_bus::InboundMessage;
use tokio::sync::Mutex;

const DEDUP_TTL: Duration = Duration::from_secs(60);
pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
struct DedupEntry {
    seen_at: Instant,
}

pub struct Dedup {
    inner: Mutex<HashMap<String, DedupEntry>>,
}

impl Dedup {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub async fn is_duplicate(&self, msg: &InboundMessage) -> bool {
        let key = dedup_key(msg);
        if key.is_empty() {
            return false;
        }
        let mut g = self.inner.lock().await;
        if g.contains_key(&key) {
            return true;
        }
        g.insert(key, DedupEntry { seen_at: Instant::now() });
        false
    }

    pub async fn cleanup_once(&self) -> usize {
        let now = Instant::now();
        let mut g = self.inner.lock().await;
        let before = g.len();
        g.retain(|_, v| now.duration_since(v.seen_at) <= DEDUP_TTL);
        before - g.len()
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }
}

impl Default for Dedup {
    fn default() -> Self {
        Self::new()
    }
}

fn dedup_key(msg: &InboundMessage) -> String {
    if msg.peer_kind == "group" {
        let h = fnv1a(&msg.text);
        format!("group:{}:{}:{}:{:08x}", msg.channel, msg.chat_id, msg.user_id, h)
    } else if !msg.message_id.is_empty() {
        format!("dm:{}:{}:{}", msg.channel, msg.account_id, msg.message_id)
    } else {
        String::new()
    }
}

fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0;
    for b in s.bytes() {
        h = h.wrapping_mul(0x01000193).wrapping_add(b as u32);
    }
    h
}

/// Spawn a background task that runs `Dedup::cleanup_once` every
/// `CLEANUP_INTERVAL` seconds. Returns the `JoinHandle` so the caller
/// can abort on shutdown.
pub fn spawn_dedup_cleanup(dedup: Arc<Dedup>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let _ = dedup.cleanup_once().await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm(channel: &str, account: &str, chat: &str, msg_id: &str, text: &str) -> InboundMessage {
        let mut m = InboundMessage::default();
        m.channel = channel.into();
        m.account_id = account.into();
        m.chat_id = chat.into();
        m.user_id = "u1".into();
        m.message_id = msg_id.into();
        m.text = text.into();
        m.peer_kind = "dm".into();
        m
    }

    fn group(channel: &str, chat: &str, user: &str, text: &str) -> InboundMessage {
        let mut m = InboundMessage::default();
        m.channel = channel.into();
        m.account_id = "bot".into();
        m.chat_id = chat.into();
        m.user_id = user.into();
        m.text = text.into();
        m.peer_kind = "group".into();
        m
    }

    #[tokio::test]
    async fn dedup_first_message_passes() {
        let d = Dedup::new();
        assert!(!d.is_duplicate(&dm("telegram", "bot1", "c1", "m1", "hi")).await);
    }

    #[tokio::test]
    async fn dedup_dm_repeats() {
        let d = Dedup::new();
        let m = dm("telegram", "bot1", "c1", "m1", "hi");
        assert!(!d.is_duplicate(&m).await);
        assert!(d.is_duplicate(&m).await);
    }

    #[tokio::test]
    async fn dedup_dm_message_id_distinguishes() {
        let d = Dedup::new();
        assert!(!d.is_duplicate(&dm("telegram", "bot1", "c1", "m1", "hi")).await);
        assert!(!d.is_duplicate(&dm("telegram", "bot1", "c1", "m2", "hi")).await);
        assert!(d.is_duplicate(&dm("telegram", "bot1", "c1", "m1", "hi")).await);
    }

    #[tokio::test]
    async fn dedup_group_keys_on_text_hash() {
        let d = Dedup::new();
        let m1 = group("telegram", "c1", "u1", "hello world");
        let m2 = group("telegram", "c1", "u1", "hello world");
        assert!(!d.is_duplicate(&m1).await);
        assert!(d.is_duplicate(&m2).await);
    }

    #[tokio::test]
    async fn dedup_group_different_text_not_dup() {
        let d = Dedup::new();
        assert!(!d.is_duplicate(&group("telegram", "c1", "u1", "a")).await);
        assert!(!d.is_duplicate(&group("telegram", "c1", "u1", "b")).await);
    }

    #[tokio::test]
    async fn dedup_dm_without_id_passes() {
        let d = Dedup::new();
        let mut m = dm("web", "", "c1", "ignored", "hi");
        m.message_id = "".into();
        assert!(!d.is_duplicate(&m).await);
        assert!(!d.is_duplicate(&m).await);
    }

    #[tokio::test]
    async fn dedup_cleanup_removes_old_entries() {
        let d = Dedup::new();
        d.is_duplicate(&dm("telegram", "bot1", "c1", "m1", "hi")).await;
        assert_eq!(d.len().await, 1);
        {
            let mut g = d.inner.lock().await;
            for v in g.values_mut() {
                v.seen_at = Instant::now()
                    .checked_sub(Duration::from_secs(120))
                    .unwrap();
            }
        }
        let removed = d.cleanup_once().await;
        assert_eq!(removed, 1);
        assert_eq!(d.len().await, 0);
    }

    #[test]
    fn fnv1a_is_deterministic_and_collides_only_on_equal() {
        assert_eq!(fnv1a("hello"), fnv1a("hello"));
        assert_ne!(fnv1a("hello"), fnv1a("world"));
    }
}
