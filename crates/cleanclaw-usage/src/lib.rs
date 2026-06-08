//! LLM token usage metering. Mirrors
//! .

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsageError {
    #[error("usage: provider error: {0}")]
    Provider(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_creation: i64,
}

impl Tokens {
    pub fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_creation
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Range {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

impl Range {
    pub fn last_n(n: i64) -> Self {
        let today = day_bucket(Utc::now());
        Self {
            since: Some(today - chrono::Duration::days(n - 1)),
            until: Some(today),
        }
    }

    pub fn contains(&self, day: DateTime<Utc>) -> bool {
        if let Some(s) = self.since {
            if day < s {
                return false;
            }
        }
        if let Some(u) = self.until {
            if day > u {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Totals {
    #[serde(rename = "inputTokens")]
    pub input: i64,
    #[serde(rename = "outputTokens")]
    pub output: i64,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read: i64,
    #[serde(rename = "cacheCreationTokens")]
    pub cache_creation: i64,
    #[serde(rename = "requestCount")]
    pub requests: i64,
}

impl Totals {
    pub fn total_tokens(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_creation
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rank {
    pub key: String,
    pub tokens: i64,
    pub input: i64,
    pub output: i64,
    pub requests: i64,
}

#[async_trait::async_trait]
pub trait Meter: Send + Sync + 'static {
    async fn record_tokens(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        provider: &str,
        model: &str,
        t: Tokens,
    ) -> Result<(), UsageError>;

    async fn totals(&self, r: Range) -> Result<Totals, UsageError>;

    async fn top_agents(&self, r: Range, limit: usize) -> Result<Vec<Rank>, UsageError>;

    async fn top_users(&self, r: Range, limit: usize) -> Result<Vec<Rank>, UsageError>;

    async fn sessions_for_agent(
        &self,
        agent_id: &str,
        user_id: &str,
        r: Range,
        limit: usize,
    ) -> Result<Vec<Rank>, UsageError>;

    async fn close(&self) -> Result<(), UsageError>;
}

fn day_bucket(t: DateTime<Utc>) -> DateTime<Utc> {
    let d = t.date_naive();
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct MemKey {
    day: DateTime<Utc>,
    user_id: String,
    agent_id: String,
    session_key: String,
    provider: String,
    model: String,
}

#[derive(Debug, Clone, Default)]
struct MemCell {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_creation: i64,
    requests: i64,
}

/// In-process meter. Lost on restart; useful for tests + dev runs.
pub struct MemMeter {
    data: Mutex<HashMap<MemKey, MemCell>>,
}

impl MemMeter {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Meter for MemMeter {
    async fn record_tokens(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        provider: &str,
        model: &str,
        t: Tokens,
    ) -> Result<(), UsageError> {
        let k = MemKey {
            day: day_bucket(Utc::now()),
            user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
            session_key: session_key.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
        };
        let mut g = self.data.lock().expect("mem meter poisoned");
        let c = g.entry(k).or_default();
        c.input += t.input;
        c.output += t.output;
        c.cache_read += t.cache_read;
        c.cache_creation += t.cache_creation;
        c.requests += 1;
        Ok(())
    }

    async fn totals(&self, r: Range) -> Result<Totals, UsageError> {
        let g = self.data.lock().expect("mem meter poisoned");
        let mut out = Totals::default();
        for (k, c) in g.iter() {
            if !r.contains(k.day) {
                continue;
            }
            out.input += c.input;
            out.output += c.output;
            out.cache_read += c.cache_read;
            out.cache_creation += c.cache_creation;
            out.requests += c.requests;
        }
        Ok(out)
    }

    async fn top_agents(&self, r: Range, limit: usize) -> Result<Vec<Rank>, UsageError> {
        self.rank(r, limit, |k| k.agent_id.clone()).await
    }

    async fn top_users(&self, r: Range, limit: usize) -> Result<Vec<Rank>, UsageError> {
        self.rank(r, limit, |k| k.user_id.clone()).await
    }

    async fn sessions_for_agent(
        &self,
        agent_id: &str,
        user_id: &str,
        r: Range,
        limit: usize,
    ) -> Result<Vec<Rank>, UsageError> {
        let g = self.data.lock().expect("mem meter poisoned");
        let mut agg: HashMap<String, Rank> = HashMap::new();
        for (k, c) in g.iter() {
            if k.agent_id != agent_id {
                continue;
            }
            if !user_id.is_empty() && k.user_id != user_id {
                continue;
            }
            if !r.contains(k.day) {
                continue;
            }
            let row = agg.entry(k.session_key.clone()).or_insert_with(|| Rank {
                key: k.session_key.clone(),
                ..Default::default()
            });
            row.input += c.input;
            row.output += c.output;
            row.tokens += c.input + c.output + c.cache_read + c.cache_creation;
            row.requests += c.requests;
        }
        let mut out: Vec<Rank> = agg.into_values().collect();
        out.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        if limit > 0 && out.len() > limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    async fn close(&self) -> Result<(), UsageError> {
        Ok(())
    }
}

impl MemMeter {
    async fn rank<F>(&self, r: Range, limit: usize, key: F) -> Result<Vec<Rank>, UsageError>
    where
        F: Fn(&MemKey) -> String,
    {
        let g = self.data.lock().expect("mem meter poisoned");
        let mut agg: HashMap<String, Rank> = HashMap::new();
        for (k, c) in g.iter() {
            if !r.contains(k.day) {
                continue;
            }
            let id = key(k);
            let row = agg.entry(id.clone()).or_insert_with(|| Rank {
                key: id,
                ..Default::default()
            });
            row.input += c.input;
            row.output += c.output;
            row.tokens += c.input + c.output + c.cache_read + c.cache_creation;
            row.requests += c.requests;
        }
        let mut out: Vec<Rank> = agg.into_values().collect();
        out.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        if limit > 0 && out.len() > limit {
            out.truncate(limit);
        }
        Ok(out)
    }
}

#[allow(dead_code)]
fn _day_bucket_naive() -> NaiveDate {
    Utc::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mem_meter_records_and_totals() {
        let m = MemMeter::new();
        m.record_tokens(
            "u1",
            "a1",
            "s1",
            "anthropic",
            "claude-3",
            Tokens {
                input: 100,
                output: 50,
                cache_read: 20,
                cache_creation: 10,
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u1",
            "a1",
            "s1",
            "anthropic",
            "claude-3",
            Tokens {
                input: 200,
                output: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u2",
            "a2",
            "s2",
            "openai",
            "gpt-4",
            Tokens {
                input: 30,
                output: 15,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let totals = m.totals(Range::last_n(1)).await.unwrap();
        assert_eq!(totals.input, 330);
        assert_eq!(totals.output, 165);
        assert_eq!(totals.cache_read, 20);
        assert_eq!(totals.cache_creation, 10);
        assert_eq!(totals.requests, 3);
        assert_eq!(totals.total_tokens(), 525);
    }

    #[tokio::test]
    async fn top_agents_sorts_by_tokens_desc() {
        let m = MemMeter::new();
        m.record_tokens(
            "u",
            "a1",
            "s",
            "p",
            "m",
            Tokens {
                input: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u",
            "a2",
            "s",
            "p",
            "m",
            Tokens {
                input: 500,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u",
            "a3",
            "s",
            "p",
            "m",
            Tokens {
                input: 200,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let r = m.top_agents(Range::last_n(1), 10).await.unwrap();
        assert_eq!(r[0].key, "a2");
        assert_eq!(r[0].tokens, 500);
        assert_eq!(r[1].key, "a3");
        assert_eq!(r[2].key, "a1");
    }

    #[tokio::test]
    async fn top_agents_limit_caps_results() {
        let m = MemMeter::new();
        for n in 0..5 {
            m.record_tokens(
                "u",
                &format!("a{n}"),
                "s",
                "p",
                "m",
                Tokens {
                    input: n,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let r = m.top_agents(Range::last_n(1), 2).await.unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].key, "a4");
        assert_eq!(r[1].key, "a3");
    }

    #[tokio::test]
    async fn sessions_for_agent_filters_by_user() {
        let m = MemMeter::new();
        m.record_tokens(
            "u1",
            "a1",
            "s1",
            "p",
            "m",
            Tokens {
                input: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u2",
            "a1",
            "s2",
            "p",
            "m",
            Tokens {
                input: 20,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u1",
            "a1",
            "s1",
            "p",
            "m",
            Tokens {
                input: 30,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.record_tokens(
            "u1",
            "a2",
            "s3",
            "p",
            "m",
            Tokens {
                input: 999,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // All users for a1
        let r = m
            .sessions_for_agent("a1", "", Range::last_n(1), 0)
            .await
            .unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].key, "s1"); // tokens 40 > 20
        assert_eq!(r[0].tokens, 40);

        // u1 only
        let r = m
            .sessions_for_agent("a1", "u1", Range::last_n(1), 0)
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].key, "s1");
    }

    #[tokio::test]
    async fn range_last_n_spans_n_days() {
        let r = Range::last_n(7);
        let since = r.since.unwrap();
        let until = r.until.unwrap();
        let diff = (until - since).num_days();
        assert_eq!(diff, 6);
    }

    #[tokio::test]
    async fn zero_token_call_still_increments_request_count() {
        let m = MemMeter::new();
        m.record_tokens("u", "a", "s", "p", "m", Tokens::default())
            .await
            .unwrap();
        let t = m.totals(Range::last_n(1)).await.unwrap();
        assert_eq!(t.requests, 1);
        assert_eq!(t.total_tokens(), 0);
    }

    #[test]
    fn range_contains_filters_edges() {
        // Use day-bucketed timestamps since the meter stores at day
        // granularity; comparing second-precision time against a
        // midnight-truncated `until` would fail.
        let now = day_bucket(Utc::now());
        let r = Range {
            since: Some(now - chrono::Duration::days(2)),
            until: Some(now),
        };
        let day_past = now - chrono::Duration::days(5);
        let day_now = now;
        let day_future = now + chrono::Duration::days(1);
        assert!(!r.contains(day_past));
        assert!(r.contains(day_now));
        assert!(!r.contains(day_future));
    }
}

// =====================================================================
// SQLMeter — Postgres / SQLite backed Meter. Mirrors
// .
// =====================================================================

/// Backed by a `sqlx::SqlitePool` (Postgres pool type-erased via the
/// `sqlx` trait). Placeholders are rewritten `?` → `$N` when running
/// on Postgres. The test suite uses SQLite in-memory.
pub struct SqlMeter {
    pool: SqlxPool,
    dialect: SqlDialect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    Sqlite,
    Postgres,
}

/// Type-erased pool so we can swap SQLite/Postgres without changing
/// the public API. Internally it's `sqlx::SqlitePool` for tests
/// (which is the most-deployed config); a Postgres variant can be
/// added by enabling the `postgres` feature on `sqlx`.
pub type SqlxPool = sqlx::SqlitePool;

impl SqlMeter {
    pub async fn connect_sqlite(url: &str) -> Result<Self, UsageError> {
        let pool = sqlx::SqlitePool::connect(url)
            .await
            .map_err(|e| UsageError::Provider(format!("sqlite connect: {e}")))?;
        Ok(Self {
            pool,
            dialect: SqlDialect::Sqlite,
        })
    }

    pub fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    pub fn pool(&self) -> &SqlxPool {
        &self.pool
    }

    fn rebind(&self, q: &str) -> String {
        Self::rebind_static(self.dialect, q)
    }

    /// Pure rebind function for use in tests / static contexts
    /// where constructing a `SqlMeter` is overkill.
    pub fn rebind_static(dialect: SqlDialect, q: &str) -> String {
        if dialect != SqlDialect::Postgres {
            return q.to_string();
        }
        let mut out = String::with_capacity(q.len() + 8);
        let mut n = 0;
        for c in q.chars() {
            if c == '?' {
                n += 1;
                out.push_str(&format!("${n}"));
            } else {
                out.push(c);
            }
        }
        out
    }
}

#[async_trait::async_trait]
impl Meter for SqlMeter {
    async fn record_tokens(
        &self,
        user_id: &str,
        agent_id: &str,
        session_key: &str,
        provider: &str,
        model: &str,
        t: Tokens,
    ) -> Result<(), UsageError> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let q = self.rebind(
            "INSERT INTO token_usage_daily
                 (day, user_id, agent_id, session_key, provider, model,
                  input_tokens, output_tokens, cache_read_tokens, cache_create_tokens, request_count)
               VALUES (?,?,?,?,?,?,?,?,?,?,1)
               ON CONFLICT (day, user_id, agent_id, session_key, provider, model) DO UPDATE SET
                   input_tokens = token_usage_daily.input_tokens + EXCLUDED.input_tokens,
                   output_tokens = token_usage_daily.output_tokens + EXCLUDED.output_tokens,
                   cache_read_tokens = token_usage_daily.cache_read_tokens + EXCLUDED.cache_read_tokens,
                   cache_create_tokens = token_usage_daily.cache_create_tokens + EXCLUDED.cache_create_tokens,
                   request_count = token_usage_daily.request_count + 1",
        );
        sqlx::query(&q)
            .bind(day)
            .bind(user_id)
            .bind(agent_id)
            .bind(session_key)
            .bind(provider)
            .bind(model)
            .bind(t.input)
            .bind(t.output)
            .bind(t.cache_read)
            .bind(t.cache_creation)
            .bind(1i64)
            .execute(&self.pool)
            .await
            .map_err(|e| UsageError::Provider(format!("upsert: {e}")))?;
        Ok(())
    }

    async fn totals(&self, r: Range) -> Result<Totals, UsageError> {
        let since = r
            .since
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "1970-01-01".to_string());
        let until = r
            .until
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "2999-12-31".to_string());
        let q = self.rebind(
            "SELECT
                 COALESCE(SUM(input_tokens),0),
                 COALESCE(SUM(output_tokens),0),
                 COALESCE(SUM(cache_read_tokens),0),
                 COALESCE(SUM(cache_create_tokens),0),
                 COALESCE(SUM(request_count),0)
             FROM token_usage_daily
             WHERE day >= ? AND day <= ?",
        );
        let row: (i64, i64, i64, i64, i64) = sqlx::query_as(&q)
            .bind(since)
            .bind(until)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| UsageError::Provider(format!("totals: {e}")))?;
        Ok(Totals {
            input: row.0,
            output: row.1,
            cache_read: row.2,
            cache_creation: row.3,
            requests: row.4,
        })
    }

    async fn top_agents(&self, r: Range, limit: usize) -> Result<Vec<Rank>, UsageError> {
        self.top_by(r, limit, "agent_id").await
    }

    async fn top_users(&self, r: Range, limit: usize) -> Result<Vec<Rank>, UsageError> {
        self.top_by(r, limit, "user_id").await
    }

    async fn sessions_for_agent(
        &self,
        _agent_id: &str,
        _user_id: &str,
        _r: Range,
        _limit: usize,
    ) -> Result<Vec<Rank>, UsageError> {
        Ok(Vec::new())
    }

    async fn close(&self) -> Result<(), UsageError> {
        self.pool.close().await;
        Ok(())
    }
}

impl SqlMeter {
    async fn top_by(&self, r: Range, limit: usize, col: &str) -> Result<Vec<Rank>, UsageError> {
        if !matches!(col, "agent_id" | "user_id") {
            return Err(UsageError::Provider(format!("unsupported rank col {col}")));
        }
        let since = r
            .since
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "1970-01-01".to_string());
        let until = r
            .until
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "2999-12-31".to_string());
        let q = self.rebind(&format!(
            "SELECT {col},
                    COALESCE(SUM(input_tokens),0),
                    COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(input_tokens+output_tokens+cache_read_tokens+cache_create_tokens),0),
                    COALESCE(SUM(request_count),0)
             FROM token_usage_daily
             WHERE day >= ? AND day <= ?
             GROUP BY {col}
             ORDER BY SUM(input_tokens+output_tokens+cache_read_tokens+cache_create_tokens) DESC
             LIMIT ?"
        ));
        let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(&q)
            .bind(since)
            .bind(until)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| UsageError::Provider(format!("top_by: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|r| Rank {
                key: r.0,
                input: r.1,
                output: r.2,
                tokens: r.3,
                requests: r.4,
            })
            .collect())
    }
}

#[cfg(test)]
mod sql_meter_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlxPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS token_usage_daily (
                 day TEXT NOT NULL,
                 user_id TEXT NOT NULL,
                 agent_id TEXT NOT NULL,
                 session_key TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 model TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL DEFAULT 0,
                 output_tokens INTEGER NOT NULL DEFAULT 0,
                 cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                 cache_create_tokens INTEGER NOT NULL DEFAULT 0,
                 request_count INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (day, user_id, agent_id, session_key, provider, model)
               )"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn sql_meter_round_trip() {
        let pool = fresh_pool().await;
        let meter = SqlMeter {
            pool,
            dialect: SqlDialect::Sqlite,
        };
        meter
            .record_tokens(
                "u1",
                "a1",
                "s1",
                "openai",
                "gpt-4o",
                Tokens {
                    input: 100,
                    output: 50,
                    cache_read: 0,
                    cache_creation: 0,
                },
            )
            .await
            .unwrap();
        meter
            .record_tokens(
                "u1",
                "a1",
                "s1",
                "openai",
                "gpt-4o",
                Tokens {
                    input: 50,
                    output: 25,
                    cache_read: 0,
                    cache_creation: 0,
                },
            )
            .await
            .unwrap();
        let totals = meter
            .totals(Range {
                since: Some(Utc::now() - chrono::Duration::days(1)),
                until: Some(Utc::now() + chrono::Duration::days(1)),
            })
            .await
            .unwrap();
        assert_eq!(totals.input, 150);
        assert_eq!(totals.output, 75);
        assert_eq!(totals.requests, 2);
    }

    #[tokio::test]
    async fn sql_meter_top_agents() {
        let pool = fresh_pool().await;
        let meter = SqlMeter {
            pool,
            dialect: SqlDialect::Sqlite,
        };
        meter
            .record_tokens(
                "u1",
                "a1",
                "s1",
                "p",
                "m",
                Tokens {
                    input: 100,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        meter
            .record_tokens(
                "u1",
                "a2",
                "s1",
                "p",
                "m",
                Tokens {
                    input: 500,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let r = meter
            .top_agents(
                Range {
                    since: Some(Utc::now() - chrono::Duration::days(1)),
                    until: Some(Utc::now() + chrono::Duration::days(1)),
                },
                5,
            )
            .await
            .unwrap();
        assert_eq!(r[0].key, "a2");
        assert_eq!(r[0].tokens, 500);
    }

    #[test]
    fn rebind_postgres_rewrites_placeholders() {
        let fake = SqlMeter::rebind_static(SqlDialect::Postgres, "SELECT ?, ?, ?");
        assert_eq!(fake, "SELECT $1, $2, $3");
    }

    #[test]
    fn rebind_sqlite_passthrough() {
        let fake = SqlMeter::rebind_static(SqlDialect::Sqlite, "SELECT ?, ?");
        assert_eq!(fake, "SELECT ?, ?");
    }
}
