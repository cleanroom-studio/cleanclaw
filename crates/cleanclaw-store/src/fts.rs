//! SQLite FTS5 full-text search over conversation history.
//!
//! The shape is the same: a content table with id/timestamp/agent/chat/text, a virtual
//! FTS5 table that indexes it with porter+unicode61, and triggers
//! that keep the two in sync. The search side exposes a `snippet()`
//! highlight + a `rank()` ordering, returned as `FtsHit` rows.
//!
//! Stays offline-only: no FTS5 query parser plugins, just the
//! built-in tokenizer.

use chrono::{DateTime, Utc};
use cleanclaw_core::{CleanClawError, Result};
use sqlx::{Pool, Sqlite, SqlitePool};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FtsHit {
    pub snippet: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub agent_id: String,
    pub chat_id: String,
    pub rank: f64,
}

/// Open (or create) the FTS5 store. The store is a separate SQLite
/// database from the main one — same on-disk format, different file
/// — so the FTS index never bloats the primary store.
pub async fn open(db_path: &Path) -> Result<FtsStore> {
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = SqlitePool::connect(&url)
        .await
        .map_err(|e| CleanClawError::Internal(format!("open fts db: {e}")))?;
    let store = FtsStore { pool };
    store.init().await?;
    Ok(store)
}

/// In-memory store, used by tests.
pub async fn open_memory() -> Result<FtsStore> {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .map_err(|e| CleanClawError::Internal(format!("open fts mem: {e}")))?;
    let store = FtsStore { pool };
    store.init().await?;
    Ok(store)
}

#[derive(Clone)]
pub struct FtsStore {
    pool: Pool<Sqlite>,
}

impl FtsStore {
    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    pub async fn init(&self) -> Result<()> {
        // Content table. The FTS5 table refers to it as `content=`.
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS messages_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                chat_id TEXT NOT NULL
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CleanClawError::Internal(format!("fts create content: {e}")))?;

        // FTS5 virtual table.
        sqlx::query(
            r#"CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                timestamp,
                agent_id,
                chat_id,
                content='messages_content',
                content_rowid='id',
                tokenize='porter unicode61'
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CleanClawError::Internal(format!("fts create virtual: {e}")))?;

        // Triggers to keep the FTS index in sync.
        sqlx::query(
            r#"CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages_content BEGIN
                INSERT INTO messages_fts(rowid, content, timestamp, agent_id, chat_id)
                VALUES (new.id, new.content, new.timestamp, new.agent_id, new.chat_id);
            END"#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CleanClawError::Internal(format!("fts trigger ai: {e}")))?;
        sqlx::query(
            r#"CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages_content BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content, timestamp, agent_id, chat_id)
                VALUES ('delete', old.id, old.content, old.timestamp, old.agent_id, old.chat_id);
            END"#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CleanClawError::Internal(format!("fts trigger ad: {e}")))?;
        Ok(())
    }

    /// Add a message to the index. `role` is prefixed to the indexed
    /// text ("user: ..." or "assistant: ...") so role-aware queries
    /// (`role:assistant foo`) work the same way the Go port did.
    pub async fn index(
        &self,
        agent_id: &str,
        chat_id: &str,
        role: &str,
        content: &str,
        ts: DateTime<Utc>,
    ) -> Result<i64> {
        if content.is_empty() {
            return Err(CleanClawError::InvalidArgument("empty content".into()));
        }
        let payload = format!("{role}: {content}");
        let result = sqlx::query(
            "INSERT INTO messages_content (content, timestamp, agent_id, chat_id) VALUES (?,?,?,?)",
        )
        .bind(payload)
        .bind(ts.to_rfc3339())
        .bind(agent_id)
        .bind(chat_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CleanClawError::Internal(format!("fts index: {e}")))?;
        Ok(result.last_insert_rowid())
    }

    /// Full-text search. `limit` defaults to 10 when zero or negative.
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<FtsHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let limit = if limit <= 0 { 10 } else { limit };
        let rows = sqlx::query(
            r#"SELECT
                snippet(messages_fts, 0, '<b>', '</b>', '...', 32) AS snippet,
                messages_fts.content,
                messages_fts.timestamp,
                messages_fts.agent_id,
                messages_fts.chat_id,
                rank
            FROM messages_fts
            WHERE messages_fts MATCH ?
            ORDER BY rank
            LIMIT ?"#,
        )
        .bind(query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CleanClawError::Internal(format!("fts search: {e}")))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            use sqlx::Row;
            let ts_str: String = row
                .try_get("timestamp")
                .map_err(|e| CleanClawError::Internal(format!("fts ts col: {e}")))?;
            let ts = DateTime::parse_from_rfc3339(&ts_str)
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            out.push(FtsHit {
                snippet: row
                    .try_get("snippet")
                    .map_err(|e| CleanClawError::Internal(format!("fts snip col: {e}")))?,
                content: row
                    .try_get("content")
                    .map_err(|e| CleanClawError::Internal(format!("fts content col: {e}")))?,
                timestamp: ts,
                agent_id: row
                    .try_get("agent_id")
                    .map_err(|e| CleanClawError::Internal(format!("fts agent col: {e}")))?,
                chat_id: row
                    .try_get("chat_id")
                    .map_err(|e| CleanClawError::Internal(format!("fts chat col: {e}")))?,
                rank: row
                    .try_get("rank")
                    .map_err(|e| CleanClawError::Internal(format!("fts rank col: {e}")))?,
            });
        }
        Ok(out)
    }

    /// Drop the index entirely. Used by tests + reinstall flows.
    pub async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM messages_content")
            .execute(&self.pool)
            .await
            .map_err(|e| CleanClawError::Internal(format!("fts clear: {e}")))?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// FTS5 reserved keywords that must not appear as bare tokens in a
/// user query. Otherwise an input like `"OR rust"` would parse as a
/// boolean expression and 500. Keep this list aligned with the FTS5
/// reference (`https://www.sqlite.org/fts5.html#fts5_strings`).
const FTS5_KEYWORDS: &[&str] = &[
    "AND", "OR", "NOT", "NEAR",
];

/// Escape a free-form query string into a safe FTS5 prefix query.
/// Strips characters that have special meaning in FTS5, drops the
/// reserved keywords, and appends `*` to every word so the query is
/// a prefix match (the same shape the Go port's `Index` → `Search`
/// chain used).
pub fn to_prefix_query(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let cleaned: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if cleaned.is_empty() {
                return String::new();
            }
            let upper = cleaned.to_ascii_uppercase();
            if FTS5_KEYWORDS.contains(&upper.as_str()) {
                return String::new();
            }
            format!("{cleaned}*")
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn init_creates_tables() {
        let s = open_memory().await.unwrap();
        s.close().await;
    }

    #[tokio::test]
    async fn index_then_search_basic() {
        let s = open_memory().await.unwrap();
        s.index("a1", "c1", "user", "hello world", Utc::now())
            .await
            .unwrap();
        s.index("a1", "c1", "assistant", "hi there", Utc::now())
            .await
            .unwrap();
        let hits = s.search("hello", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("hello"));
        assert_eq!(hits[0].agent_id, "a1");
        assert!(hits[0].snippet.contains("<b>"));
        s.close().await;
    }

    #[tokio::test]
    async fn search_role_prefixed_query() {
        let s = open_memory().await.unwrap();
        s.index("a1", "c1", "user", "fox jumps over", Utc::now())
            .await
            .unwrap();
        s.index("a1", "c1", "assistant", "the lazy dog", Utc::now())
            .await
            .unwrap();
        // The FTS5 content is "user: fox jumps over" / "assistant: …".
        // A `fox` query matches only the user-prefixed row.
        let hits = s.search("fox", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.starts_with("user:"));
        s.close().await;
    }

    #[tokio::test]
    async fn search_empty_query_returns_empty() {
        let s = open_memory().await.unwrap();
        let hits = s.search("   ", 10).await.unwrap();
        assert!(hits.is_empty());
        s.close().await;
    }

    #[tokio::test]
    async fn search_limit_caps() {
        let s = open_memory().await.unwrap();
        for i in 0..5 {
            s.index("a1", "c1", "user", &format!("fox message {i}"), Utc::now())
                .await
                .unwrap();
        }
        let hits = s.search("fox", 2).await.unwrap();
        assert!(hits.len() <= 2);
        s.close().await;
    }

    #[tokio::test]
    async fn clear_removes_rows() {
        let s = open_memory().await.unwrap();
        s.index("a", "c", "user", "alpha", Utc::now()).await.unwrap();
        s.clear().await.unwrap();
        let hits = s.search("alpha", 10).await.unwrap();
        assert!(hits.is_empty());
        s.close().await;
    }

    #[tokio::test]
    async fn index_rejects_empty_content() {
        let s = open_memory().await.unwrap();
        let err = s
            .index("a", "c", "user", "", Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, CleanClawError::InvalidArgument(_)));
        s.close().await;
    }

    #[test]
    fn prefix_query_sanitises() {
        let q = to_prefix_query("hello, world! 42");
        assert_eq!(q, "hello* world* 42*");
    }

    #[test]
    fn prefix_query_strips_specials() {
        let q = to_prefix_query("(rust) and \"fast\" OR *");
        // parens, quotes, asterisks are dropped; OR is a reserved
        // FTS5 keyword so it's filtered out as well.
        assert!(!q.contains('('));
        assert!(!q.contains('"'));
        assert!(!q.contains("OR"));
        assert!(q.contains("rust"));
        assert!(q.contains("fast"));
    }

    #[test]
    fn prefix_query_empty_input() {
        assert_eq!(to_prefix_query(""), "");
        assert_eq!(to_prefix_query("   "), "");
        assert_eq!(to_prefix_query("!@#$"), "");
    }
}
