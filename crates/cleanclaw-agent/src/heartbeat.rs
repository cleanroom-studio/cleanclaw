//! Heartbeat tick — periodic self-check the agent performs on its
//! `HEARTBEAT.md` checklist.
//!
//! The gateway
//! schedules a per-agent tick (e.g. every N minutes). The tick:
//!   1. reads HEARTBEAT.md from the agent's workspace
//!   2. presents the checklist to the LLM as a one-shot prompt
//!   3. either returns silently (nothing to do) or returns a
//!      short reply that the gateway publishes back to the chat
//!      surface as a "system" message
//!
//! For the first cut the LLM call is a stub: we just emit a debug
//! log + return the file content. Wiring up the actual prompt
//! lives in the agent loop integration (Phase M).

use chrono::{DateTime, Utc};
use cleanclaw_core::Result;
use std::path::Path;
use tracing::{debug, info};

pub const HEARTBEAT_FILE: &str = "HEARTBEAT.md";

#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    pub every_n_minutes: u32,
    pub model: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            every_n_minutes: 60,
            model: None,
        }
    }
}

pub struct HeartbeatTick {
    pub config: HeartbeatConfig,
}

impl HeartbeatTick {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self { config }
    }

    /// Run one heartbeat tick for the given agent. Returns the
    /// "system reply" the gateway should surface (empty if nothing
    /// to report).
    pub async fn run(
        &self,
        workspace_root: &str,
        agent_id: &str,
    ) -> Result<Option<String>> {
        let path = Path::new(workspace_root).join(HEARTBEAT_FILE);
        if !path.exists() {
            debug!(agent = agent_id, "no HEARTBEAT.md; skipping tick");
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path).map_err(|e| {
            cleanclaw_core::CleanClawError::Internal(format!("read HEARTBEAT.md: {e}"))
        })?;
        if content.trim().is_empty() {
            return Ok(None);
        }
        info!(
            agent = agent_id,
            bytes = content.len(),
            "heartbeat tick fired"
        );
        // Stub: the real implementation would feed content into the
        // LLM, get a short reply, and return it. We just return a
        // synthetic acknowledgment for now.
        Ok(Some(format!(
            "[heartbeat {}] HEARTBEAT.md present ({} bytes). Tick complete.",
            Utc::now().to_rfc3339(),
            content.len()
        )))
    }
}

/// A simple cron-style scheduler. The gateway calls `tick` once a
/// minute (or whatever the configured poll interval is); it
/// internally tracks the last-fire time per agent and fires when
/// `every_n_minutes` has elapsed.
pub struct HeartbeatScheduler {
    last_fire: parking_lot::Mutex<std::collections::HashMap<String, DateTime<Utc>>>,
    config: HeartbeatConfig,
}

impl HeartbeatScheduler {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            last_fire: parking_lot::Mutex::new(Default::default()),
            config,
        }
    }

    /// True if this agent is due for a heartbeat. Updates the
    /// last-fire timestamp on `true` returns.
    pub fn is_due(&self, agent_id: &str, now: DateTime<Utc>) -> bool {
        let mut last = self.last_fire.lock();
        let due = match last.get(agent_id) {
            Some(t) => (now - *t).num_minutes() >= self.config.every_n_minutes as i64,
            None => true,
        };
        if due {
            last.insert(agent_id.to_string(), now);
        }
        due
    }

    pub fn config(&self) -> &HeartbeatConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn heartbeat_runs_and_returns_acknowledgment() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("HEARTBEAT.md"), "- check inbox\n- review metrics\n").unwrap();
        let tick = HeartbeatTick::new(HeartbeatConfig::default());
        let out = tick.run(dir.path().to_str().unwrap(), "agt_1").await.unwrap();
        assert!(out.is_some());
        assert!(out.unwrap().contains("heartbeat"));
    }

    #[tokio::test]
    async fn heartbeat_skips_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let tick = HeartbeatTick::new(HeartbeatConfig::default());
        let out = tick.run(dir.path().to_str().unwrap(), "agt_1").await.unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn scheduler_fires_then_cools_down() {
        let s = HeartbeatScheduler::new(HeartbeatConfig { every_n_minutes: 5, model: None });
        let now = Utc::now();
        assert!(s.is_due("a", now));
        // Same instant: not due again.
        assert!(!s.is_due("a", now));
        // 6 minutes later: due.
        assert!(s.is_due("a", now + Duration::minutes(6)));
    }
}
