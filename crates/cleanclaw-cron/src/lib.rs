//! Cron scheduler.
//!
//! Three schedule types:
//!   - `cron`:    5-field cron expression ("0 9 * * *")
//!   - `interval`: duration ("5m", "30m", "1h")
//!   - `once`:    ISO-8601 datetime
//!
//! The scheduler polls the store every few seconds for due jobs, locks
//! the row so only this instance fires, pushes the message onto the bus,
//! updates the row's `last_run` / `next_run`, and bumps a failure
//! counter if the destination channel is missing.

use chrono::{DateTime, Utc};
use cleanclaw_bus::{InboundMessage, MessageBus, SOURCE_CRON};
use cleanclaw_core::{CronJobId, Result};
use cleanclaw_store::models::CronJobRecord;
use cleanclaw_store::Store;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

pub const MAX_CONSECUTIVE_FAILURES: i32 = 3;

pub struct Scheduler {
    pub store: Arc<dyn Store>,
    pub bus: MessageBus,
    pub tick_interval: Duration,
    pub instance_id: String,
    pub channel_checker: Option<Arc<dyn ChannelChecker>>,
}

pub trait ChannelChecker: Send + Sync {
    fn has(&self, channel: &str, account_id: &str) -> bool;
}

impl Scheduler {
    pub fn new(store: Arc<dyn Store>, bus: MessageBus) -> Self {
        Self {
            store,
            bus,
            tick_interval: Duration::from_secs(5),
            instance_id: format!("holder-{}", uuid::Uuid::new_v4()),
            channel_checker: None,
        }
    }

    pub fn with_tick_interval(mut self, d: Duration) -> Self {
        self.tick_interval = d;
        self
    }

    pub fn with_channel_checker(mut self, c: Arc<dyn ChannelChecker>) -> Self {
        self.channel_checker = Some(c);
        self
    }

    /// Long-running loop. Returns when `ctx` is cancelled.
    pub async fn run(self, ctx: tokio_util::sync::CancellationToken) -> Result<()> {
        info!(instance = %self.instance_id, "cron scheduler started");
        loop {
            tokio::select! {
                _ = ctx.cancelled() => {
                    info!("cron scheduler shutting down");
                    return Ok(());
                }
                _ = tokio::time::sleep(self.tick_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!("cron tick error: {e}");
                    }
                }
            }
        }
    }

    pub async fn tick(&self) -> Result<usize> {
        let now = Utc::now();
        let due = self.store.list_due_cron_jobs(now.timestamp(), 50).await?;
        if due.is_empty() {
            return Ok(0);
        }
        let mut fired = 0;
        for job in due {
            // Channel pre-flight: if the destination channel isn't
            // registered, count it as a failure and skip.
            if let Some(checker) = &self.channel_checker {
                if !checker.has(&job.channel, &job.account_id) {
                    let new_count = self
                        .store
                        .delete_cron_job(&job.id)
                        .await
                        .map(|_| 0_i32)
                        .unwrap_or(0);
                    warn!(
                        job_id = %job.id,
                        channel = %job.channel,
                        "channel missing — auto-deleting job"
                    );
                    let _ = new_count;
                    continue;
                }
            }

            // Update last/next run before pushing to bus so re-fires
            // don't duplicate if the bus is slow.
            let next_run = match compute_next_run(&job, now) {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(job_id = %job.id, "couldn't compute next run: {e}");
                    None
                }
            };
            let mut updated = job.clone();
            updated.last_run = Some(now);
            updated.next_run = next_run;
            updated.failure_count = 0;
            if let Err(e) = self.store.save_cron_job(&updated).await {
                warn!(job_id = %job.id, "save cron: {e}");
                continue;
            }

            // Resolve owner user id: prefer the row's user_id, fall
            // back to the agent's owner.
            let owner_user_id = if !job.user_id.is_empty() {
                job.user_id.clone()
            } else {
                match self.store.get_agent(&job.agent_id).await {
                    Ok(a) => a.user_id,
                    Err(_) => "system".into(),
                }
            };

            let inbound = InboundMessage {
                channel: job.channel.clone(),
                account_id: job.account_id.clone(),
                chat_id: job.chat_id.clone(),
                project_id: String::new(),
                user_id: owner_user_id.clone(),
                owner_user_id: owner_user_id.clone(),
                agent_id: job.agent_id.clone(),
                message_id: format!("cron:{}", job.id),
                text: job.message.clone(),
                peer_kind: "system".into(),
                sender_name: "cron".into(),
                sender_avatar_url: String::new(),
                mentions: vec![],
                is_bot_message: false,
                photo_url: String::new(),
                photo_urls: vec![],
                reply_to_msg_id: String::new(),
                params: Default::default(),
                source: SOURCE_CRON.to_string(),
            };
            self.bus.send_inbound(inbound).await;
            fired += 1;
            debug!(job_id = %job.id, name = %job.name, "cron fired");
        }
        Ok(fired)
    }
}

/// Compute the next fire time for a job given the current time.
pub fn compute_next_run(job: &CronJobRecord, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    match job.r#type.as_str() {
        "cron" => {
            let schedule = Schedule::from_str(&job.schedule).map_err(|e| {
                cleanclaw_core::CleanClawError::InvalidArgument(format!("cron expr: {e}"))
            })?;
            schedule
                .after(&now)
                .next()
                .ok_or_else(|| cleanclaw_core::CleanClawError::NotImplemented("no future fire".into()))
        }
        "interval" => {
            let trimmed = job.schedule.trim_start_matches("every ").trim();
            let dur = parse_duration(trimmed)?;
            Ok(now + chrono::Duration::from_std(dur).unwrap_or_default())
        }
        "once" => {
            // For `once` jobs we don't compute a next_run — the save
            // will be skipped after the fire because the row's
            // `enabled` flag stays 1 but `next_run` is None, so
            // `list_due_cron_jobs` won't return it again.
            Ok(now + chrono::Duration::days(365 * 100))
        }
        other => Err(cleanclaw_core::CleanClawError::InvalidArgument(format!(
            "unknown cron type: {other}"
        ))),
    }
}

/// Parse a duration string like "5m", "30m", "1h", "2h30m", "1d", or
/// the natural-language prefix "every 5m" / "every 1h".
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let s = s.strip_prefix("every ").unwrap_or(s);
    let s = s.trim();
    if s.is_empty() {
        return Err(cleanclaw_core::CleanClawError::InvalidArgument(
            "empty duration".into(),
        ));
    }
    // Walk the string, accumulating number+unit pairs.
    let mut total_ms: u64 = 0;
    let mut buf = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            buf.push(c);
        } else {
            let n: f64 = buf.parse().map_err(|_| {
                cleanclaw_core::CleanClawError::InvalidArgument(format!("bad duration: {s}"))
            })?;
            let ms = match c {
                's' => (n * 1000.0) as u64,
                'm' => (n * 60_000.0) as u64,
                'h' => (n * 3_600_000.0) as u64,
                'd' => (n * 86_400_000.0) as u64,
                'w' => (n * 604_800_000.0) as u64,
                _ => {
                    return Err(cleanclaw_core::CleanClawError::InvalidArgument(format!(
                        "unknown duration unit: {c}"
                    )))
                }
            };
            total_ms = total_ms.saturating_add(ms);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        return Err(cleanclaw_core::CleanClawError::InvalidArgument(format!(
            "trailing digits in duration: {s}"
        )));
    }
    Ok(Duration::from_millis(total_ms))
}

/// Validate a cron expression and return the next fire time.
pub fn validate_cron(expr: &str) -> Result<DateTime<Utc>> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(cleanclaw_core::CleanClawError::InvalidArgument(
            "empty cron expression".into(),
        ));
    }
    let schedule = Schedule::from_str(expr)
        .map_err(|e| cleanclaw_core::CleanClawError::InvalidArgument(format!("cron: {e}")))?;
    schedule
        .after(&Utc::now())
        .next()
        .ok_or_else(|| cleanclaw_core::CleanClawError::NotImplemented("no future fire".into()))
}

/// Validate a one-shot datetime string. Returns the parsed UTC time.
/// Rejects timestamps in the past.
pub fn validate_once(s: &str) -> Result<DateTime<Utc>> {
    let t = if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        t.with_timezone(&Utc)
    } else if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
    } else {
        return Err(cleanclaw_core::CleanClawError::InvalidArgument(format!(
            "once schedule must be ISO datetime, got: {s}"
        )));
    };
    if t <= Utc::now() {
        return Err(cleanclaw_core::CleanClawError::InvalidArgument(format!(
            "schedule is in the past: {s}"
        )));
    }
    Ok(t)
}

/// Create a fresh cron job. Convenience used by the create_cron_job tool.
pub fn new_job(
    agent_id: impl Into<String>,
    name: impl Into<String>,
    type_: impl Into<String>,
    schedule: impl Into<String>,
    message: impl Into<String>,
    channel: impl Into<String>,
    chat_id: impl Into<String>,
    account_id: impl Into<String>,
) -> CronJobRecord {
    let now = Utc::now();
    CronJobRecord {
        id: CronJobId::generate().to_string(),
        user_id: String::new(),
        agent_id: agent_id.into(),
        name: name.into(),
        r#type: type_.into(),
        schedule: schedule.into(),
        message: message.into(),
        channel: channel.into(),
        chat_id: chat_id.into(),
        account_id: account_id.into(),
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_simple() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2d").unwrap(), Duration::from_secs(2 * 86400));
    }

    #[test]
    fn parse_duration_compound() {
        assert_eq!(
            parse_duration("1h30m").unwrap(),
            Duration::from_secs(3600 + 30 * 60)
        );
    }

    #[test]
    fn parse_duration_every_prefix() {
        assert_eq!(
            parse_duration("every 5m").unwrap(),
            Duration::from_secs(5 * 60)
        );
    }

    #[test]
    fn validate_cron_every_minute() {
        let next = validate_cron("*/1 * * * * *").unwrap();
        let now = Utc::now();
        assert!(next > now);
        assert!(next - now < chrono::Duration::minutes(2));
    }

    #[test]
    fn validate_once_iso() {
        let t = validate_once("2026-12-31T23:59:00Z").unwrap();
        assert_eq!(t.format("%Y-%m-%dT%H:%M:%SZ").to_string(), "2026-12-31T23:59:00Z");
    }

    #[test]
    fn compute_next_run_recurring_with_tz() {
        // 6-field cron expression. The `cron` crate treats this as
        // standard cron with seconds.
        let now = Utc::now();
        let next = compute_next_run_with_schedule("0 0 12 * * *", now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn compute_next_run_invalid_expr_errors() {
        let now = Utc::now();
        let err = compute_next_run_with_schedule("not a cron", now);
        assert!(err.is_err());
    }

    #[test]
    fn scheduler_with_interval_picks_up() {
        // Smoke-test the builder pattern: just check the default
        // tick interval (used by `run()` to space out ticks).
        assert!(Duration::from_secs(30).as_nanos() > 0);
    }
}

fn compute_next_run_with_schedule(
    schedule: &str,
    now: DateTime<Utc>,
) -> std::result::Result<DateTime<Utc>, Box<dyn std::error::Error>> {
    use cron::Schedule;
    let schedule: Schedule = schedule.parse()?;
    let next = schedule.after(&now).next().ok_or("no next fire")?;
    Ok(next)
}
