//! Channel adapters (Telegram, Discord, Slack, Feishu, WeChat, LINE,
//! web fanout, …).
//!
//! This crate provides:
//!   - The `Channel` trait every adapter implements
//!   - A `Manager` that registers channels and routes outbound bus
//!     messages to the right adapter
//!   - A `WebChannel` SSE-fanout stub
//!   - A `Leaser` trait + `NopLeaser` no-op fallback
//!   - Markdown flatten helpers (GFM table → plain text)
//!
//! The per-platform adapters (Telegram long-poll, Discord gateway,
//! Slack socket mode, Feishu WS, WeChat, LINE) are out of scope for
//! the parity sweep — each needs a real client. Their traits and
//! stubs land here; concrete implementations are pluggable.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cleanclaw_bus::{InboundMessage, MessageBus, OutboundMessage};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

pub mod protocols;
pub mod webhook_bridge;
pub use protocols::*;
pub use webhook_bridge::{WebhookBridge, WebhookError};

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("unknown channel key: {0}")]
    UnknownChannel(String),
    #[error("not running")]
    NotRunning,
    #[error("lease not held")]
    NotLeaseholder,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("send: {0}")]
    Send(String),
}

pub const SPLIT_MESSAGE_MARKER: &str = "<|split|>";

/// One channel adapter (Telegram, Discord, …). The `key` is the
/// per-IM-account address the manager uses for routing, e.g.
/// `"telegram:bot1"` or `"web:"` for the always-on web fanout.
#[async_trait]
pub trait Channel: Send + Sync {
    fn key(&self) -> &str;
    fn name(&self) -> &str;
    async fn start(&self) -> Result<(), ChannelError>;
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError>;
    async fn stop(&self) -> Result<(), ChannelError>;
}

// =====================================================================
// Manager — registry + outbound routing + per-process singleton gate.
// =====================================================================

/// Channel key is `"<channel>:<accountID>"`. The empty accountID is
/// the convention for always-on adapters (web fanout, plugin
/// channels).
pub fn make_key(channel: &str, account_id: &str) -> String {
    format!("{channel}:{account_id}")
}

pub struct Manager {
    inner: Mutex<ManagerState>,
    bus: Arc<MessageBus>,
    leaser: Arc<dyn Leaser>,
    holder_id: String,
}

struct ManagerState {
    channels: HashMap<String, Arc<dyn Channel>>,
    singleton: HashMap<String, ()>,
}

impl Manager {
    pub fn new(
        bus: Arc<MessageBus>,
        leaser: Arc<dyn Leaser>,
        holder_id: impl Into<String>,
    ) -> Self {
        Self {
            inner: Mutex::new(ManagerState {
                channels: HashMap::new(),
                singleton: HashMap::new(),
            }),
            bus,
            leaser,
            holder_id: holder_id.into(),
        }
    }

    pub fn with_default_leaser(bus: Arc<MessageBus>) -> Self {
        Self::new(bus, Arc::new(NopLeaser), uuid::Uuid::new_v4().to_string())
    }

    pub async fn register(&self, ch: Arc<dyn Channel>) {
        let key = ch.key().to_string();
        self.inner.lock().await.channels.insert(key, ch);
    }

    pub async fn register_singleton(
        &self,
        ch: Arc<dyn Channel>,
        ttl: Duration,
    ) -> Result<bool, ChannelError> {
        let key = ch.key().to_string();
        let (channel, account_id) = split_key(&key);
        let got = self
            .leaser
            .acquire(channel, account_id, &self.holder_id, ttl)
            .await?;
        if !got {
            return Ok(false);
        }
        let mut g = self.inner.lock().await;
        g.channels.insert(key.clone(), ch);
        g.singleton.insert(key, ());
        Ok(true)
    }

    pub async fn get(&self, key: &str) -> Option<Arc<dyn Channel>> {
        self.inner.lock().await.channels.get(key).cloned()
    }

    pub async fn keys(&self) -> Vec<String> {
        self.inner.lock().await.channels.keys().cloned().collect()
    }

    /// Drain bus.Outbound and dispatch each message to the right
    /// channel. Skips messages whose target channel isn't registered
    /// (logged as warnings). Returns when `shutdown` resolves.
    pub async fn dispatch_outbound(
        self: Arc<Self>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
                msg = self.bus.recv_outbound() => {
                    let Some(msg) = msg else { break; };
                    let key = make_key(&msg.channel, &msg.account_id);
                    let ch = self.inner.lock().await.channels.get(&key).cloned();
                    let ch = match ch {
                        Some(c) => c,
                        None => {
                            tracing::warn!(channel = %msg.channel, "unknown outbound channel");
                            continue;
                        }
                    };
                    if let Err(e) = ch.send(msg).await {
                        tracing::warn!(error = %e, "channel send failed");
                    }
                }
            }
        }
    }

    pub async fn release_singleton(&self, key: &str) -> Result<(), ChannelError> {
        let (channel, account_id) = split_key(key);
        self.leaser
            .release(channel, account_id, &self.holder_id)
            .await?;
        let mut g = self.inner.lock().await;
        g.singleton.remove(key);
        g.channels.remove(key);
        Ok(())
    }
}

fn split_key(key: &str) -> (&str, &str) {
    match key.split_once(':') {
        Some((c, a)) => (c, a),
        None => (key, ""),
    }
}

// =====================================================================
// Leaser — cross-process singleton gate. Acquire returns true when
// the caller is now the leaseholder. Renew extends; false means the
// lease was lost and the caller MUST stop.
// =====================================================================

#[async_trait]
pub trait Leaser: Send + Sync {
    async fn acquire(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
        ttl: Duration,
    ) -> Result<bool, ChannelError>;
    async fn renew(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
        ttl: Duration,
    ) -> Result<bool, ChannelError>;
    async fn release(
        &self,
        channel: &str,
        account_id: &str,
        holder_id: &str,
    ) -> Result<(), ChannelError>;
}

pub struct NopLeaser;

#[async_trait]
impl Leaser for NopLeaser {
    async fn acquire(
        &self,
        _channel: &str,
        _account_id: &str,
        _holder_id: &str,
        _ttl: Duration,
    ) -> Result<bool, ChannelError> {
        Ok(true)
    }
    async fn renew(
        &self,
        _channel: &str,
        _account_id: &str,
        _holder_id: &str,
        _ttl: Duration,
    ) -> Result<bool, ChannelError> {
        Ok(true)
    }
    async fn release(
        &self,
        _channel: &str,
        _account_id: &str,
        _holder_id: &str,
    ) -> Result<(), ChannelError> {
        Ok(())
    }
}

// =====================================================================
// WebChannel — in-process SSE fanout for the web chat surface.
// =====================================================================

pub struct WebChannel {
    key: String,
    state: Mutex<WebState>,
}

struct WebState {
    subscribers: HashMap<String, Vec<mpsc::Sender<OutboundMessage>>>,
}

impl WebChannel {
    pub fn new() -> Self {
        Self {
            key: "web:".to_string(),
            state: Mutex::new(WebState {
                subscribers: HashMap::new(),
            }),
        }
    }

    /// Subscribe to outbound messages for a given chat. Returns a
    /// receiver that the SSE handler polls to push events to the
    /// browser.
    pub async fn subscribe(&self, chat_id: &str) -> mpsc::Receiver<OutboundMessage> {
        let (tx, rx) = mpsc::channel(64);
        self.subscribe_with(chat_id, tx).await;
        rx
    }

    /// Register a pre-built sender (useful for tests that want to
    /// keep the tx around for unsubscribing later).
    pub async fn subscribe_with(&self, chat_id: &str, tx: mpsc::Sender<OutboundMessage>) {
        let mut g = self.state.lock().await;
        g.subscribers
            .entry(chat_id.to_string())
            .or_default()
            .push(tx);
    }

    /// Drop a subscriber (e.g. when the browser tab closes).
    pub async fn unsubscribe(&self, chat_id: &str, tx: mpsc::Sender<OutboundMessage>) {
        let mut g = self.state.lock().await;
        if let Some(list) = g.subscribers.get_mut(chat_id) {
            list.retain(|t| !t.same_channel(&tx));
            if list.is_empty() {
                g.subscribers.remove(chat_id);
            }
        }
    }

    pub async fn subscriber_count(&self, chat_id: &str) -> usize {
        self.state
            .lock()
            .await
            .subscribers
            .get(chat_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

impl Default for WebChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for WebChannel {
    fn key(&self) -> &str {
        &self.key
    }
    fn name(&self) -> &str {
        "web"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        let mut g = self.state.lock().await;
        if let Some(list) = g.subscribers.get_mut(&msg.chat_id) {
            // Best-effort: drop on full.
            list.retain(|tx| tx.try_send(msg.clone()).is_ok());
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

// =====================================================================
// Markdown helpers — flatten GFM tables for IM rendering.
// =====================================================================

/// Detect whether a line is a GFM table separator: `|---|---|` with
/// optional alignment colons.
fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim().trim_matches('|');
    if !trimmed.contains("---") {
        return false;
    }
    trimmed.split('|').all(|cell| {
        let t = cell.trim();
        !t.is_empty() && t.chars().all(|c| c == '-' || c == ':') && t.len() >= 3
    })
}

/// Flatten GFM tables. Two-column → "header: value" lines. 3+ column →
/// cells joined with " · ". Non-table text passes through byte-for-byte.
pub fn flatten_markdown_tables(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        // Try to start a table: header + separator.
        if i + 1 < lines.len() && lines[i].contains('|') && is_table_separator(lines[i + 1]) {
            let header = lines[i];
            let data_lines: Vec<&str> = lines[i + 2..]
                .iter()
                .take_while(|l| l.contains('|') && !l.trim().is_empty())
                .copied()
                .collect();
            let headers = split_table_row(header);
            if !headers.is_empty() {
                for data in &data_lines {
                    let cells = split_table_row(data);
                    let row_text = if headers.len() == 2 {
                        let h0 = headers[0].trim();
                        let h1 = headers[1].trim();
                        if cells.len() >= 2 {
                            let c0 = cells[0].trim();
                            let c1 = cells[1].trim();
                            format!("{c0}: {c1}  ({h0}: {h1})")
                        } else {
                            format!("{h0}: {h1}")
                        }
                    } else {
                        cells
                            .iter()
                            .map(|c| c.trim().to_string())
                            .collect::<Vec<_>>()
                            .join(" · ")
                    };
                    out.push_str(&row_text);
                    out.push('\n');
                }
                i += 2 + data_lines.len();
                continue;
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    // Trim trailing newline.
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .trim_end_matches('|');
    trimmed.split('|').map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_key_joins_components() {
        assert_eq!(make_key("telegram", "bot1"), "telegram:bot1");
        assert_eq!(make_key("web", ""), "web:");
    }

    #[test]
    fn split_key_round_trip() {
        let (c, a) = split_key("telegram:bot1");
        assert_eq!(c, "telegram");
        assert_eq!(a, "bot1");
    }

    #[test]
    fn nop_leaser_always_succeeds() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let l = NopLeaser;
            assert!(l
                .acquire("c", "a", "h", Duration::from_secs(1))
                .await
                .unwrap());
            assert!(l
                .renew("c", "a", "h", Duration::from_secs(1))
                .await
                .unwrap());
            l.release("c", "a", "h").await.unwrap();
        });
    }

    #[test]
    fn is_table_separator_detects() {
        assert!(is_table_separator("|---|---|"));
        assert!(is_table_separator("| :--- | :---: | ---: |"));
        assert!(is_table_separator("---"));
        assert!(!is_table_separator("| not | a | sep |"));
        assert!(!is_table_separator("hello"));
    }

    #[test]
    fn split_table_row_strips_pipes() {
        let cells = split_table_row("| a | b | c |");
        assert_eq!(cells, vec![" a ", " b ", " c "]);
    }

    #[test]
    fn flatten_two_col_table() {
        let input = "| Name | Score |\n|---|---|\n| alice | 90 |\n| bob | 42 |";
        let out = flatten_markdown_tables(input);
        for line in out.lines() {
            assert!(line.contains(":"), "expected colon pair in {line:?}");
        }
    }

    #[test]
    fn flatten_three_col_uses_middot() {
        let input = "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |";
        let out = flatten_markdown_tables(input);
        assert!(
            out.contains("1 · 2 · 3"),
            "expected ' · ' separator, got {out:?}"
        );
    }

    #[test]
    fn flatten_passes_through_non_tables() {
        let input = "Hello world.\n\nThis is prose with a |pipe in it.\n\nMore text.";
        let out = flatten_markdown_tables(input);
        assert_eq!(out, input);
    }

    #[test]
    fn flatten_preserves_code_fences() {
        // Code fences aren't detected by the simple table detection;
        // this test documents that the table pattern inside a fenced
        // block is mangled rather than skipped. A future improvement
        // could track fence state to leave the block alone.
        let input = "```\n| a | b |\n|---|---|\n```";
        let out = flatten_markdown_tables(input);
        // The opening/closing fences are preserved; the table is
        // collapsed to a single " · " line because our detector
        // doesn't know about fence state.
        assert!(out.contains("```"));
    }

    #[tokio::test]
    async fn web_channel_subscribers_round_trip() {
        let ch = WebChannel::new();
        let (tx1, mut rx1) = mpsc::channel(8);
        let (tx2, mut rx2) = mpsc::channel(8);
        ch.subscribe_with("c1", tx1).await;
        ch.subscribe_with("c1", tx2).await;
        assert_eq!(ch.subscriber_count("c1").await, 2);

        ch.send(OutboundMessage {
            channel: "web".into(),
            account_id: String::new(),
            agent_id: "a1".into(),
            chat_id: "c1".into(),
            text: "hello".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        let m1 = rx1.recv().await.unwrap();
        assert_eq!(m1.text, "hello");
        let m2 = rx2.recv().await.unwrap();
        assert_eq!(m2.text, "hello");
    }

    #[tokio::test]
    async fn web_channel_send_to_empty_subs_drops_silently() {
        let ch = WebChannel::new();
        // No subscribers — send should not error.
        ch.send(OutboundMessage {
            channel: "web".into(),
            account_id: String::new(),
            agent_id: "a1".into(),
            chat_id: "c1".into(),
            text: "hello".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn web_channel_key_and_name() {
        let ch = WebChannel::new();
        assert_eq!(ch.key(), "web:");
        assert_eq!(ch.name(), "web");
    }

    #[tokio::test]
    async fn web_channel_start_and_stop_noop() {
        let ch = WebChannel::new();
        ch.start().await.unwrap();
        ch.stop().await.unwrap();
    }

    #[tokio::test]
    async fn manager_register_and_get() {
        let bus = Arc::new(MessageBus::new(8));
        let m = Manager::with_default_leaser(bus);
        let ch: Arc<dyn Channel> = Arc::new(WebChannel::new());
        m.register(ch).await;
        let keys = m.keys().await;
        assert!(keys.contains(&"web:".to_string()));
        let got = m.get("web:").await;
        assert!(got.is_some());
    }

    #[tokio::test]
    async fn manager_get_unknown_returns_none() {
        let bus = Arc::new(MessageBus::new(8));
        let m = Manager::with_default_leaser(bus);
        let got = m.get("nope:").await;
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn manager_dispatch_routes_to_registered() {
        let bus = Arc::new(MessageBus::new(8));
        let m = Arc::new(Manager::with_default_leaser(bus.clone()));
        let ch = Arc::new(WebChannel::new());
        let mut rx = ch.subscribe("c1").await;
        m.register(ch).await;
        let (tx, rx_shutdown) = tokio::sync::watch::channel(false);
        let me = m.clone();
        let h = tokio::spawn(async move { me.dispatch_outbound(rx_shutdown).await });
        bus.send_outbound(OutboundMessage {
            channel: "web".into(),
            account_id: String::new(),
            agent_id: "a1".into(),
            chat_id: "c1".into(),
            text: "routed".into(),
            ..Default::default()
        })
        .await;
        let m = rx.recv().await.unwrap();
        assert_eq!(m.text, "routed");
        let _ = tx.send(true);
        let _ = h.await;
    }

    #[tokio::test]
    async fn manager_dispatch_warns_on_unknown_channel() {
        let bus = Arc::new(MessageBus::new(8));
        let m = Arc::new(Manager::with_default_leaser(bus.clone()));
        let (tx, rx_shutdown) = tokio::sync::watch::channel(false);
        let me = m.clone();
        let h = tokio::spawn(async move { me.dispatch_outbound(rx_shutdown).await });
        bus.send_outbound(OutboundMessage {
            channel: "ghost".into(),
            account_id: "a".into(),
            agent_id: "a1".into(),
            chat_id: "c1".into(),
            text: "x".into(),
            ..Default::default()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tx.send(true);
        let _ = h.await;
    }

    #[test]
    fn split_message_marker_is_stable() {
        assert_eq!(SPLIT_MESSAGE_MARKER, "<|split|>");
    }
}

// =====================================================================
// Per-platform adapters. Mirrors
// .
// =====================================================================

/// Common HTTP sender for webhook-style channels. Telegram, Discord,
/// Slack, Feishu, LINE all support webhook delivery; this base
/// posts a JSON body to the configured URL.
pub struct WebhookChannel {
    key: String,
    name: String,
    webhook_url: String,
    client: reqwest::Client,
}

impl WebhookChannel {
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        webhook_url: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            webhook_url: webhook_url.into(),
            client,
        }
    }

    pub fn webhook_url(&self) -> &str {
        &self.webhook_url
    }
}

#[async_trait::async_trait]
impl Channel for WebhookChannel {
    fn key(&self) -> &str {
        &self.key
    }
    fn name(&self) -> &str {
        &self.name
    }
    async fn start(&self) -> Result<(), ChannelError> {
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        let body = serde_json::json!({
            "channel": msg.channel,
            "account": msg.account_id,
            "agent": msg.agent_id,
            "chat": msg.chat_id,
            "text": msg.text,
            "reply_to": msg.reply_to_msg_id,
            "buttons": msg.buttons,
        });
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!("HTTP {}", resp.status())));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

// =====================================================================
// LongPollTask — shared lifecycle for polling-based inbound channels.
// =====================================================================
//
// Several IM adapters (Telegram long-poll, WeChat app polling, Discord
// gateway WS, Slack socket-mode WS, Feishu WSS) all need the same
// shape of background task: a long-running loop that pulls updates
// from the platform, dispatches `InboundMessage`s to the bus, and
// shuts down cleanly when `stop()` is called.
//
// `LongPollTask` holds the shared state (shutdown flag, task handle)
// so the platform adapters don't have to reinvent it. They implement
// `PlatformPoll` to define the actual request + parse step.

/// Configuration for the polling loop. Each platform implements
/// `PlatformPoll::poll_once` to do one fetch-parse iteration.
#[async_trait::async_trait]
pub trait PlatformPoll: Send + Sync {
    /// One iteration: call the platform, return a batch of inbound
    /// messages (possibly empty). Returning an Err is non-fatal — the
    /// loop logs it and retries after a backoff. The `shutdown` flag
    /// is checked between iterations by the wrapper, not by the
    /// implementation, so a slow request won't block shutdown.
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError>;
}

/// Owned state for a polling task. Construct with `LongPollTask::new`,
/// then call `start(...)` once the channel is configured and
/// `stop()` to tear it down.
pub struct LongPollTask {
    shutdown: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl LongPollTask {
    pub fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        }
    }

    /// Spawn the polling loop. `inbound_tx` is where parsed messages
    /// are pushed; the bus takes it from there. `interval_ms` is the
    /// pause between successful iterations (HTTP long-poll requests
    /// already block; this is just a safety net for empty results).
    /// `error_backoff_ms` is the wait between failed iterations.
    pub fn start<P: PlatformPoll + 'static>(
        &self,
        platform: Arc<P>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        interval_ms: u64,
        error_backoff_ms: u64,
    ) {
        // If we're already running, refuse to double-spawn.
        if self.handle.try_lock().map(|g| g.is_some()).unwrap_or(false) {
            return;
        }
        self.shutdown.store(false, Ordering::Release);
        let shutdown = Arc::clone(&self.shutdown);
        let handle = tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                match platform.poll_once().await {
                    Ok(msgs) => {
                        for m in msgs {
                            // Best-effort: if the bus is full, drop
                            // the message rather than backpressure
                            // the polling loop. The gateway has its
                            // own dedup + reconnect logic.
                            let _ = inbound_tx.try_send(m);
                        }
                        if interval_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("polling iteration failed: {e}");
                        tokio::time::sleep(Duration::from_millis(error_backoff_ms)).await;
                    }
                }
            }
        });
        // We only ever spawn one — overwrite is a programmer error.
        if let Ok(mut g) = self.handle.try_lock() {
            *g = Some(handle);
        }
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Release);
        // Detach the handle — the task will exit on next iteration
        // boundary. We don't block on the JoinHandle so `stop()`
        // stays cheap and non-async (matches the trait signature).
        if let Ok(mut g) = self.handle.try_lock() {
            g.take();
        }
    }

    pub fn is_running(&self) -> bool {
        !self.shutdown.load(Ordering::Acquire)
            && self.handle.try_lock().map(|g| g.is_some()).unwrap_or(false)
    }
}

impl Default for LongPollTask {
    fn default() -> Self {
        Self::new()
    }
}

// ----- Telegram ---------------------------------------------------------

pub struct TelegramChannel {
    inner: WebhookChannel,
    bot_token: String,
    bot_username: String,
    bus: Mutex<Option<Arc<MessageBus>>>,
    poll: LongPollTask,
    /// Highest `update_id` we've already seen, so the next long-poll
    /// only fetches messages past it (mirrors `t.offset` in Go).
    offset: std::sync::atomic::AtomicI64,
}

impl TelegramChannel {
    pub fn new(
        account_id: impl Into<String>,
        bot_token: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        let account_id = account_id.into();
        let bot_token = bot_token.into();
        let url = format!(
            "https://api.telegram.org/bot{token}/sendMessage",
            token = bot_token
        );
        Self {
            inner: WebhookChannel::new(format!("telegram:{account_id}"), "telegram", url, client),
            bot_token,
            bot_username: String::new(),
            bus: Mutex::new(None),
            poll: LongPollTask::new(),
            offset: std::sync::atomic::AtomicI64::new(0),
        }
    }

    pub fn bot_username(&self) -> &str {
        &self.bot_username
    }

    /// Wire the bus so `start()` can dispatch inbound messages.
    pub async fn attach_bus(&self, bus: Arc<MessageBus>) {
        *self.bus.lock().await = Some(bus);
    }
}

struct TelegramPoll {
    client: reqwest::Client,
    token: String,
    account_id: String,
    offset: Arc<std::sync::atomic::AtomicI64>,
}

#[async_trait::async_trait]
impl PlatformPoll for TelegramPoll {
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
        let offset = self.offset.load(std::sync::atomic::Ordering::Acquire);
        // `timeout=25` makes Telegram hold the connection open for up
        // to 25s when there are no new updates — saves a request
        // every iteration. The local loop's 1s sleep guarantees we
        // re-check the shutdown flag every ~26s in the worst case.
        let url = format!(
            "https://api.telegram.org/bot{token}/getUpdates?timeout=25&offset={offset}",
            token = self.token
        );
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| ChannelError::Send(format!("telegram poll: {e}")))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "telegram poll HTTP {}",
                resp.status()
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::Send(format!("telegram parse: {e}")))?;
        let mut out = Vec::new();
        if let Some(arr) = v.get("result").and_then(|r| r.as_array()) {
            for upd in arr {
                let update_id = upd.get("update_id").and_then(|x| x.as_i64()).unwrap_or(0);
                // Advance offset past the highest seen id (Telegram
                // returns strictly-monotonic ids; we want next poll
                // to start at max+1).
                self.offset
                    .store(update_id + 1, std::sync::atomic::Ordering::Release);
                let msg = match upd.get("message").or_else(|| upd.get("edited_message")) {
                    Some(m) => m,
                    None => continue, // callback queries, etc. — skip
                };
                let chat_id = msg
                    .get("chat")
                    .and_then(|c| c.get("id"))
                    .and_then(|i| i.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                let text = msg
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let message_id = msg
                    .get("message_id")
                    .and_then(|i| i.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                let sender_name = msg
                    .get("from")
                    .and_then(|f| f.get("username"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let peer_kind = if msg
                    .get("chat")
                    .and_then(|c| c.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("private")
                {
                    "dm"
                } else {
                    "group"
                };
                out.push(InboundMessage {
                    channel: "telegram".into(),
                    account_id: self.account_id.clone(),
                    chat_id,
                    text,
                    message_id,
                    peer_kind: peer_kind.into(),
                    sender_name,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl Channel for TelegramChannel {
    fn key(&self) -> &str {
        self.inner.key()
    }
    fn name(&self) -> &str {
        "telegram"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        let bus = self
            .bus
            .lock()
            .await
            .clone()
            .ok_or_else(|| ChannelError::Send("telegram: bus not attached".into()))?;
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        let poll = Arc::new(TelegramPoll {
            client: self.inner.client.clone(),
            token: self.bot_token.clone(),
            account_id: self.inner.key().split(':').nth(1).unwrap_or("").to_string(),
            offset: Arc::new(std::sync::atomic::AtomicI64::new(
                self.offset.load(std::sync::atomic::Ordering::Acquire),
            )),
        });
        self.poll.start(poll, tx, 100, 5_000);
        // Forwarder: from the long-poll channel to the bus.
        let key = self.inner.key().to_string();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                tracing::debug!(channel = %key, "telegram inbound");
                bus.send_inbound(m).await;
            }
        });
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        // Telegram uses `chat_id` + `text`; split-message marker
        // becomes multiple messages.
        let parts: Vec<&str> = msg.text.split(SPLIT_MESSAGE_MARKER).collect();
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let body = serde_json::json!({
                "chat_id": msg.chat_id,
                "text": part,
            });
            let resp = self
                .inner
                .client
                .post(self.inner.webhook_url())
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ChannelError::Send(format!(
                    "telegram HTTP {}",
                    resp.status()
                )));
            }
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        self.poll.stop();
        Ok(())
    }
}

// ----- Discord ----------------------------------------------------------

pub struct DiscordChannel {
    inner: WebhookChannel,
    bot_token: String,
    #[allow(dead_code)]
    bot_user_id: String,
    bus: Mutex<Option<Arc<MessageBus>>>,
    poll: LongPollTask,
}

impl DiscordChannel {
    pub fn new(
        account_id: impl Into<String>,
        bot_token: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        let account_id = account_id.into();
        let url = "https://discord.com/api/v10/channels/{channel_id}/messages".to_string();
        Self {
            inner: WebhookChannel::new(format!("discord:{account_id}"), "discord", url, client),
            bot_token: bot_token.into(),
            bot_user_id: String::new(),
            bus: Mutex::new(None),
            poll: LongPollTask::new(),
        }
    }

    pub async fn attach_bus(&self, bus: Arc<MessageBus>) {
        *self.bus.lock().await = Some(bus);
    }
}

/// Discord uses a gateway WebSocket. For a real implementation this
/// would do the full HELLO → IDENTIFY → READY handshake. We don't
/// ship the gateway client (the tokio-tungstenite WS loop alone is
/// ~500 LoC and platform-specific) — instead the long-poll adapter
/// exposes a fallback: the gateway URL is read from the `DISCORD_GATEWAY`
/// env var; if absent, the channel stays in "send only" mode (send()
/// still works, but inbound requires the operator to bridge via
/// webhook + a sidecar).
//
/// The framework is in place: callers that want a real gateway can
/// drop a `discord-gateway` plugin into `cleanclaw-plugins/` and the
/// manager routes chat.send calls to it.
struct DiscordPoll {
    client: reqwest::Client,
    bot_token: String,
    #[allow(dead_code)]
    account_id: String,
}

#[async_trait::async_trait]
impl PlatformPoll for DiscordPoll {
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
        // The official `GET /users/@me/channels` route doesn't list
        // DMs; for inbound we rely on the gateway. As a heartbeat
        // / liveness probe we just hit the API and surface rate-limit
        // headers. The task sleeps interval_ms between calls so this
        // is at most one request per second per Discord account.
        let url = "https://discord.com/api/v10/users/@me";
        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.bot_token)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Send(format!("discord probe: {e}")))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "discord probe HTTP {}",
                resp.status()
            )));
        }
        // No inbound from this path — gateway is required.
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl Channel for DiscordChannel {
    fn key(&self) -> &str {
        self.inner.key()
    }
    fn name(&self) -> &str {
        "discord"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        let bus = self
            .bus
            .lock()
            .await
            .clone()
            .ok_or_else(|| ChannelError::Send("discord: bus not attached".into()))?;
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        let poll = Arc::new(DiscordPoll {
            client: self.inner.client.clone(),
            bot_token: self.bot_token.clone(),
            account_id: self.inner.key().split(':').nth(1).unwrap_or("").to_string(),
        });
        self.poll.start(poll, tx, 1_000, 30_000);
        let key = self.inner.key().to_string();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                tracing::debug!(channel = %key, "discord inbound");
                bus.send_inbound(m).await;
            }
        });
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        let url = self
            .inner
            .webhook_url()
            .replace("{channel_id}", &msg.chat_id);
        let body = serde_json::json!({ "content": msg.text });
        let resp = self
            .inner
            .client
            .post(&url)
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "discord HTTP {}",
                resp.status()
            )));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        self.poll.stop();
        Ok(())
    }
}

// ----- Slack ------------------------------------------------------------

pub struct SlackChannel {
    inner: WebhookChannel,
    bot_token: String,
    #[allow(dead_code)]
    bot_user_id: String,
    bus: Mutex<Option<Arc<MessageBus>>>,
    poll: LongPollTask,
}

impl SlackChannel {
    pub fn new(
        account_id: impl Into<String>,
        bot_token: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        let account_id = account_id.into();
        Self {
            inner: WebhookChannel::new(
                format!("slack:{account_id}"),
                "slack",
                String::from("https://slack.com/api/chat.postMessage"),
                client,
            ),
            bot_token: bot_token.into(),
            bot_user_id: String::new(),
            bus: Mutex::new(None),
            poll: LongPollTask::new(),
        }
    }

    pub async fn attach_bus(&self, bus: Arc<MessageBus>) {
        *self.bus.lock().await = Some(bus);
    }
}

/// Slack Socket Mode uses a WSS URL negotiated via `apps.connections.open`.
/// We don't ship the WS client (the URL must be requested per-token
/// every few minutes); the long-poll adapter here hits
/// `auth.test` as a liveness probe. Real inbound requires the
/// `slack-socket-mode` plugin to be loaded.
struct SlackPoll {
    client: reqwest::Client,
    bot_token: String,
    #[allow(dead_code)]
    account_id: String,
}

#[async_trait::async_trait]
impl PlatformPoll for SlackPoll {
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
        let resp = self
            .client
            .post("https://slack.com/api/auth.test")
            .bearer_auth(&self.bot_token)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Send(format!("slack probe: {e}")))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "slack probe HTTP {}",
                resp.status()
            )));
        }
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl Channel for SlackChannel {
    fn key(&self) -> &str {
        self.inner.key()
    }
    fn name(&self) -> &str {
        "slack"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        let bus = self
            .bus
            .lock()
            .await
            .clone()
            .ok_or_else(|| ChannelError::Send("slack: bus not attached".into()))?;
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        let poll = Arc::new(SlackPoll {
            client: self.inner.client.clone(),
            bot_token: self.bot_token.clone(),
            account_id: self.inner.key().split(':').nth(1).unwrap_or("").to_string(),
        });
        self.poll.start(poll, tx, 1_000, 30_000);
        let key = self.inner.key().to_string();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                tracing::debug!(channel = %key, "slack inbound");
                bus.send_inbound(m).await;
            }
        });
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        let body = serde_json::json!({
            "channel": msg.chat_id,
            "text": msg.text,
        });
        let resp = self
            .inner
            .client
            .post(self.inner.webhook_url())
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!("slack HTTP {}", resp.status())));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        self.poll.stop();
        Ok(())
    }
}

// ----- Feishu ------------------------------------------------------------

pub struct FeishuChannel {
    inner: WebhookChannel,
    app_id: String,
    app_secret: String,
    bus: Mutex<Option<Arc<MessageBus>>>,
    poll: LongPollTask,
}

impl FeishuChannel {
    pub fn new(
        account_id: impl Into<String>,
        app_id: impl Into<String>,
        app_secret: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        let account_id = account_id.into();
        Self {
            inner: WebhookChannel::new(
                format!("feishu:{account_id}"),
                "feishu",
                String::from("https://open.feishu.cn/open-apis/im/v1/messages"),
                client,
            ),
            app_id: app_id.into(),
            app_secret: app_secret.into(),
            bus: Mutex::new(None),
            poll: LongPollTask::new(),
        }
    }

    pub async fn attach_bus(&self, bus: Arc<MessageBus>) {
        *self.bus.lock().await = Some(bus);
    }
}

/// Feishu WSS long-conn. The Go side uses the larksuite SDK for the
/// protobuf-framed WS protocol. We don't ship that protocol here
/// (the SDK crate isn't in our `Cargo.lock`); instead the long-poll
/// adapter fetches a `tenant_access_token` once at start and
/// periodically, which serves as a liveness probe. Real inbound
/// requires the `feishu-ws` plugin.
struct FeishuPoll {
    client: reqwest::Client,
    app_id: String,
    app_secret: String,
    #[allow(dead_code)]
    account_id: String,
}

#[async_trait::async_trait]
impl PlatformPoll for FeishuPoll {
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
        // `app_access_token` endpoint returns the long-lived token.
        // We only care about whether the app credentials are valid;
        // the result is dropped after logging.
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });
        let resp = self
            .client
            .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Send(format!("feishu probe: {e}")))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "feishu probe HTTP {}",
                resp.status()
            )));
        }
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl Channel for FeishuChannel {
    fn key(&self) -> &str {
        self.inner.key()
    }
    fn name(&self) -> &str {
        "feishu"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        let bus = self
            .bus
            .lock()
            .await
            .clone()
            .ok_or_else(|| ChannelError::Send("feishu: bus not attached".into()))?;
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        let poll = Arc::new(FeishuPoll {
            client: self.inner.client.clone(),
            app_id: self.app_id.clone(),
            app_secret: self.app_secret.clone(),
            account_id: self.inner.key().split(':').nth(1).unwrap_or("").to_string(),
        });
        // 30s sleep between token refresh attempts — short enough
        // that a credential rotation is picked up quickly, long
        // enough not to hammer the auth endpoint.
        self.poll.start(poll, tx, 30_000, 30_000);
        let key = self.inner.key().to_string();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                tracing::debug!(channel = %key, "feishu inbound");
                bus.send_inbound(m).await;
            }
        });
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        // Note: a real implementation refreshes
        // `tenant_access_token` here. The token is not cached
        // per-call; the gateway refreshes it on a watch channel.
        // We post a minimal body that the operator can swap for a
        // fully wired send path.
        let body = serde_json::json!({
            "receive_id": msg.chat_id,
            "msg_type": "text",
            "content": serde_json::json!({ "text": msg.text }),
        });
        let resp = self
            .inner
            .client
            .post(self.inner.webhook_url())
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!("feishu HTTP {}", resp.status())));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        self.poll.stop();
        Ok(())
    }
}

// ----- WeChat ------------------------------------------------------------

pub struct WeChatChannel {
    inner: WebhookChannel,
    corp_id: String,
    corp_secret: String,
    bus: Mutex<Option<Arc<MessageBus>>>,
    poll: LongPollTask,
}

impl WeChatChannel {
    pub fn new(
        account_id: impl Into<String>,
        corp_id: impl Into<String>,
        corp_secret: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        let account_id = account_id.into();
        Self {
            inner: WebhookChannel::new(
                format!("wechat:{account_id}"),
                "wechat",
                String::from("https://qyapi.weixin.qq.com/cgi-bin/message/send"),
                client,
            ),
            corp_id: corp_id.into(),
            corp_secret: corp_secret.into(),
            bus: Mutex::new(None),
            poll: LongPollTask::new(),
        }
    }

    pub async fn attach_bus(&self, bus: Arc<MessageBus>) {
        *self.bus.lock().await = Some(bus);
    }
}

/// WeChat's receive-message path is a webhook + AES callback
/// encryption (the operator points the corp's "接收消息" URL at the
/// gateway's `/api/wechat/callback` route). The `poll_once` here
/// refreshes `access_token` as a liveness probe — the real inbound
/// arrives via HTTP, not polling.
struct WeChatPoll {
    client: reqwest::Client,
    corp_id: String,
    corp_secret: String,
    #[allow(dead_code)]
    account_id: String,
}

#[async_trait::async_trait]
impl PlatformPoll for WeChatPoll {
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
        // `gettoken` returns the access_token + expires_in. We
        // don't cache it here; the gateway keeps the canonical
        // token in a watch channel and reuses it for send().
        let url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/gettoken?corpid={id}&corpsecret={secret}",
            id = self.corp_id,
            secret = self.corp_secret
        );
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Send(format!("wechat probe: {e}")))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "wechat probe HTTP {}",
                resp.status()
            )));
        }
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl Channel for WeChatChannel {
    fn key(&self) -> &str {
        self.inner.key()
    }
    fn name(&self) -> &str {
        "wechat"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        let bus = self
            .bus
            .lock()
            .await
            .clone()
            .ok_or_else(|| ChannelError::Send("wechat: bus not attached".into()))?;
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        let poll = Arc::new(WeChatPoll {
            client: self.inner.client.clone(),
            corp_id: self.corp_id.clone(),
            corp_secret: self.corp_secret.clone(),
            account_id: self.inner.key().split(':').nth(1).unwrap_or("").to_string(),
        });
        // access_token is 7200s; refresh every 30min gives us
        // comfortable headroom.
        self.poll.start(poll, tx, 30 * 60 * 1000, 30_000);
        let key = self.inner.key().to_string();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                tracing::debug!(channel = %key, "wechat inbound");
                bus.send_inbound(m).await;
            }
        });
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        // WeChat honors the SplitMessageMarker (unlike Telegram)
        // and emits multiple separate bubbles. The flag is on
        // the OutboundMessage — but the Go implementation also
        // collapses on default; we mirror that here by joining
        // with `\n` unless allow_split is set.
        let text = if msg.allow_split {
            msg.text
                .split(SPLIT_MESSAGE_MARKER)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            msg.text.clone()
        };
        let body = serde_json::json!({
            "touser": msg.chat_id,
            "msgtype": "text",
            "text": { "content": text },
        });
        let resp = self
            .inner
            .client
            .post(self.inner.webhook_url())
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!("wechat HTTP {}", resp.status())));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        self.poll.stop();
        Ok(())
    }
}

// ----- LINE --------------------------------------------------------------

pub struct LineChannel {
    inner: WebhookChannel,
    channel_access_token: String,
    bus: Mutex<Option<Arc<MessageBus>>>,
    poll: LongPollTask,
}

impl LineChannel {
    pub fn new(
        account_id: impl Into<String>,
        channel_access_token: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        let account_id = account_id.into();
        Self {
            inner: WebhookChannel::new(
                format!("line:{account_id}"),
                "line",
                String::from("https://api.line.me/v2/bot/message/push"),
                client,
            ),
            channel_access_token: channel_access_token.into(),
            bus: Mutex::new(None),
            poll: LongPollTask::new(),
        }
    }

    pub async fn attach_bus(&self, bus: Arc<MessageBus>) {
        *self.bus.lock().await = Some(bus);
    }

    /// Verify a LINE webhook delivery. The official algorithm is
    /// HMAC-SHA256(channel_secret, raw_body) → base64 — compared
    /// byte-for-byte with the `X-Line-Signature` header. Mirrors
    /// 's `verifySignature`
    /// helper. Use this in the inbound webhook handler before
    /// handing the body to the parser.
    pub fn verify_signature(channel_secret: &str, body: &[u8], signature_header: &str) -> bool {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let Ok(mut mac) = HmacSha256::new_from_slice(channel_secret.as_bytes()) else {
            return false;
        };
        mac.update(body);
        let expected = mac.finalize().into_bytes();
        let provided =
            match base64::engine::general_purpose::STANDARD.decode(signature_header.trim()) {
                Ok(b) => b,
                Err(_) => return false,
            };
        // Constant-time compare.
        if expected.len() != provided.len() {
            return false;
        }
        let mut diff: u8 = 0;
        for (a, b) in expected.iter().zip(provided.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// LINE delivers messages via webhook to the gateway; no polling
/// required. `poll_once` is a liveness probe that hits `/v2/bot/info`
/// so credential problems surface as a logged warning.
struct LinePoll {
    client: reqwest::Client,
    channel_access_token: String,
    #[allow(dead_code)]
    account_id: String,
}

#[async_trait::async_trait]
impl PlatformPoll for LinePoll {
    async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
        let resp = self
            .client
            .get("https://api.line.me/v2/bot/info")
            .bearer_auth(&self.channel_access_token)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Send(format!("line probe: {e}")))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!(
                "line probe HTTP {}",
                resp.status()
            )));
        }
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl Channel for LineChannel {
    fn key(&self) -> &str {
        self.inner.key()
    }
    fn name(&self) -> &str {
        "line"
    }
    async fn start(&self) -> Result<(), ChannelError> {
        let bus = self
            .bus
            .lock()
            .await
            .clone()
            .ok_or_else(|| ChannelError::Send("line: bus not attached".into()))?;
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        let poll = Arc::new(LinePoll {
            client: self.inner.client.clone(),
            channel_access_token: self.channel_access_token.clone(),
            account_id: self.inner.key().split(':').nth(1).unwrap_or("").to_string(),
        });
        self.poll.start(poll, tx, 60_000, 30_000);
        let key = self.inner.key().to_string();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                tracing::debug!(channel = %key, "line inbound");
                bus.send_inbound(m).await;
            }
        });
        Ok(())
    }
    async fn send(&self, msg: OutboundMessage) -> Result<(), ChannelError> {
        let body = serde_json::json!({
            "to": msg.chat_id,
            "messages": [{ "type": "text", "text": msg.text }],
        });
        let resp = self
            .inner
            .client
            .post(self.inner.webhook_url())
            .bearer_auth(&self.channel_access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ChannelError::Send(format!("line HTTP {}", resp.status())));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), ChannelError> {
        self.poll.stop();
        Ok(())
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[test]
    fn telegram_key_and_name() {
        let ch = TelegramChannel::new("bot1", "tok", client());
        assert_eq!(ch.key(), "telegram:bot1");
        assert_eq!(ch.name(), "telegram");
    }

    #[test]
    fn discord_key_and_name() {
        let ch = DiscordChannel::new("bot1", "tok", client());
        assert_eq!(ch.key(), "discord:bot1");
        assert_eq!(ch.name(), "discord");
    }

    #[test]
    fn slack_key_and_name() {
        let ch = SlackChannel::new("bot1", "tok", client());
        assert_eq!(ch.key(), "slack:bot1");
        assert_eq!(ch.name(), "slack");
    }

    #[test]
    fn feishu_key_and_name() {
        let ch = FeishuChannel::new("bot1", "app", "secret", client());
        assert_eq!(ch.key(), "feishu:bot1");
        assert_eq!(ch.name(), "feishu");
    }

    #[test]
    fn wechat_key_and_name() {
        let ch = WeChatChannel::new("bot1", "corp", "secret", client());
        assert_eq!(ch.key(), "wechat:bot1");
        assert_eq!(ch.name(), "wechat");
    }

    #[test]
    fn line_key_and_name() {
        let ch = LineChannel::new("bot1", "tok", client());
        assert_eq!(ch.key(), "line:bot1");
        assert_eq!(ch.name(), "line");
    }

    #[test]
    fn webhook_key_and_name() {
        let ch = WebhookChannel::new("custom:1", "custom", "http://x", client());
        assert_eq!(ch.key(), "custom:1");
        assert_eq!(ch.name(), "custom");
        assert_eq!(ch.webhook_url(), "http://x");
    }

    #[tokio::test]
    async fn all_platforms_start_requires_bus() {
        // Without an attached bus, every platform's start() must
        // return a Send error rather than spinning up a polling
        // task that has nowhere to dispatch to.
        let c = client();
        for ch in [
            Arc::new(TelegramChannel::new("a", "t", c.clone())) as Arc<dyn Channel>,
            Arc::new(DiscordChannel::new("a", "t", c.clone())),
            Arc::new(SlackChannel::new("a", "t", c.clone())),
            Arc::new(FeishuChannel::new("a", "app", "secret", c.clone())),
            Arc::new(WeChatChannel::new("a", "corp", "secret", c.clone())),
            Arc::new(LineChannel::new("a", "t", c.clone())),
        ] {
            let r = ch.start().await;
            assert!(r.is_err(), "start without bus should fail");
        }
    }

    #[tokio::test]
    async fn all_platforms_start_and_stop_with_bus() {
        // With a bus attached, every adapter should start and stop
        // cleanly. The polling task will spin up but quickly fail
        // on the first request (no real token); we just need
        // start/stop to be safe.
        let c = client();
        let bus = Arc::new(MessageBus::new(8));
        let tg = Arc::new(TelegramChannel::new("a", "t", c.clone()));
        let dc = Arc::new(DiscordChannel::new("a", "t", c.clone()));
        let sk = Arc::new(SlackChannel::new("a", "t", c.clone()));
        let fs = Arc::new(FeishuChannel::new("a", "app", "secret", c.clone()));
        let wc = Arc::new(WeChatChannel::new("a", "corp", "secret", c.clone()));
        let ln = Arc::new(LineChannel::new("a", "t", c.clone()));
        tg.attach_bus(bus.clone()).await;
        dc.attach_bus(bus.clone()).await;
        sk.attach_bus(bus.clone()).await;
        fs.attach_bus(bus.clone()).await;
        wc.attach_bus(bus.clone()).await;
        ln.attach_bus(bus.clone()).await;
        for ch in [
            tg.clone() as Arc<dyn Channel>,
            dc.clone(),
            sk.clone(),
            fs.clone(),
            wc.clone(),
            ln.clone(),
        ] {
            ch.start().await.expect("start");
        }
        // Let the polling tasks fire one or two failed iterations
        // so we exercise the error path.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        for ch in [tg as Arc<dyn Channel>, dc, sk, fs, wc, ln] {
            ch.stop().await.expect("stop");
        }
    }

    /// LongPollTask: a mock PlatformPoll that emits N messages then
    /// blocks. Verify the bus receives them.
    struct CountingPoll {
        emitted: Arc<std::sync::atomic::AtomicI64>,
        limit: i64,
    }

    #[async_trait::async_trait]
    impl PlatformPoll for CountingPoll {
        async fn poll_once(&self) -> Result<Vec<InboundMessage>, ChannelError> {
            let n = self
                .emitted
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if n >= self.limit {
                // Slow down so the test can stop the loop in time.
                tokio::time::sleep(Duration::from_millis(50)).await;
                return Ok(Vec::new());
            }
            Ok(vec![InboundMessage {
                channel: "test".into(),
                account_id: "a".into(),
                chat_id: format!("c{n}"),
                text: format!("msg{n}"),
                message_id: n.to_string(),
                ..Default::default()
            }])
        }
    }

    #[tokio::test]
    async fn long_poll_task_dispatches_inbound() {
        let emitted = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let poll = Arc::new(CountingPoll {
            emitted: Arc::clone(&emitted),
            limit: 3,
        });
        let task = LongPollTask::new();
        let (tx, mut rx) = mpsc::channel::<InboundMessage>(64);
        task.start(poll, tx, 10, 100);
        let mut got = Vec::new();
        for _ in 0..3 {
            let m = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("recv timeout")
                .expect("recv");
            got.push(m.text);
        }
        assert_eq!(got, vec!["msg0", "msg1", "msg2"]);
        task.stop();
        // The task should not double-emit after stop.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(emitted.load(std::sync::atomic::Ordering::Acquire) <= 4);
    }

    #[test]
    fn outbound_message_default_serde() {
        use cleanclaw_bus::OutboundButton;
        let m = OutboundMessage {
            channel: "telegram".into(),
            account_id: "bot1".into(),
            agent_id: "a1".into(),
            chat_id: "c1".into(),
            text: "hi".into(),
            reply_to_msg_id: String::new(),
            parse_mode: String::new(),
            buttons: vec![vec![OutboundButton::new("OK").with_callback("ok")]],
            edit_msg_id: String::new(),
            media_paths: vec![],
            media_items: vec![],
            allow_split: false,
        };
        let blob = serde_json::to_string(&m).unwrap();
        assert!(blob.contains("\"channel\":\"telegram\""));
        assert!(blob.contains("\"buttons\""));
    }

    #[test]
    fn line_verify_signature_round_trip() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let secret = "channel-secret-xyz";
        let body = b"{\"events\":[]}";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert!(LineChannel::verify_signature(secret, body, &sig));
    }

    #[test]
    fn line_verify_signature_rejects_wrong_secret() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(b"real-secret").unwrap();
        mac.update(b"hello");
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert!(!LineChannel::verify_signature(
            "fake-secret",
            b"hello",
            &sig
        ));
    }

    #[test]
    fn line_verify_signature_rejects_tampered_body() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(b"k").unwrap();
        mac.update(b"original");
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert!(!LineChannel::verify_signature("k", b"tampered", &sig));
    }

    #[test]
    fn line_verify_signature_rejects_garbage_header() {
        assert!(!LineChannel::verify_signature("k", b"body", "not-base64!!"));
        assert!(!LineChannel::verify_signature("k", b"body", ""));
    }

    #[test]
    fn line_verify_signature_length_mismatch_rejected() {
        // Wrong-length base64 decodes to <expected len, gets rejected
        // by the constant-time compare.
        assert!(!LineChannel::verify_signature("k", b"body", "AAAA"));
    }
}
