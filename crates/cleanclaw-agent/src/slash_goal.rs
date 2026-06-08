//! `/goal …` slash commands. Mirrors
//! .
//!
//! Argument grammar:
//!   /goal <objective text>    → create (or update if one exists)
//!   /goal                     → show current
//!   /goal pause               → pause the active goal
//!   /goal resume              → resume a paused goal
//!   /goal clear               → delete the active goal
//!
//! `/goal budget <N>` is intentionally absent — setting a budget
//! mid-flight has ambiguous semantics (do tokens already spent
//! count?). The create path inherits the system default at
//! creation time, with the option of being omitted (None) for
//! unbounded runs.
//!
//! All dispatch is `async` — never call `futures::executor::block_on`
//! on the store from within a tokio runtime (causes deadlock).

use chrono::Utc;
use cleanclaw_store::models::GoalRecord;
use cleanclaw_store::Store;
use std::sync::Arc;
use uuid::Uuid;

use super::{SlashOutcome, SlashResult};

/// Async entry point. Mirrors the Go `slashGoal` shape but
/// returns a `SlashResult` (the slash framework applies the
/// outcome) instead of a raw `slashResult`.
pub async fn dispatch_goal(
    store: &Arc<dyn Store>,
    agent_id: &str,
    session_key: &str,
    owner_user_id: &str,
    channel: &str,
    account_id: &str,
    chat_id: &str,
    project_id: &str,
    args: &[&str],
) -> SlashResult {
    // First arg may be a sub-command. Anything else is treated
    // as objective text for the create path. `pause`, `resume`,
    // `clear` are short keywords nobody would use as an
    // objective opener.
    let sub = args.first().copied().unwrap_or("").to_ascii_lowercase();
    match sub.as_str() {
        "" => show(store, agent_id, session_key).await,
        "show" => show(store, agent_id, session_key).await,
        "pause" => pause(store, agent_id, session_key).await,
        "resume" => resume(store, agent_id, session_key).await,
        "clear" => clear(store, agent_id, session_key).await,
        // Default: treat the entire remainder as objective text.
        _ => {
            let objective = args.join(" ");
            create(
                store,
                agent_id,
                session_key,
                owner_user_id,
                channel,
                account_id,
                chat_id,
                project_id,
                &objective,
            )
            .await
        }
    }
}

async fn show(store: &Arc<dyn Store>, agent_id: &str, session_key: &str) -> SlashResult {
    let reply = match store.get_goal(agent_id, session_key).await {
        Ok(g) if g.status == "active" => format!(
            "[goal] active — objective: \"{}\" ({} tokens used)",
            g.objective, g.tokens_used
        ),
        Ok(g) if g.status == "paused" => {
            format!("[goal] paused — objective: \"{}\"", g.objective)
        }
        Ok(g) if g.status == "complete" => {
            format!("[goal] complete — objective: \"{}\"", g.objective)
        }
        Ok(g) if g.status == "budget_limited" => format!(
            "[goal] budget-limited — objective: \"{}\" ({} tokens used)",
            g.objective, g.tokens_used
        ),
        Ok(_) => "[goal] (unknown status)".to_string(),
        Err(_) => "[goal] no active goal for this session.".to_string(),
    };
    SlashResult {
        outcome: SlashOutcome::Handled { reply: reply.clone() },
        reply,
    }
}

async fn pause(store: &Arc<dyn Store>, agent_id: &str, session_key: &str) -> SlashResult {
    let reply = match store.get_goal(agent_id, session_key).await {
        Ok(mut g) => {
            g.status = "paused".into();
            g.updated_at = Utc::now();
            match store.save_goal(&g).await {
                Ok(_) => format!("[goal] paused — objective: \"{}\"", g.objective),
                Err(e) => format!("[goal] pause failed: {e}"),
            }
        }
        Err(_) => "[goal] no goal to pause.".to_string(),
    };
    SlashResult {
        outcome: SlashOutcome::Handled { reply: reply.clone() },
        reply,
    }
}

async fn resume(store: &Arc<dyn Store>, agent_id: &str, session_key: &str) -> SlashResult {
    let reply = match store.get_goal(agent_id, session_key).await {
        Ok(mut g) => {
            g.status = "active".into();
            g.updated_at = Utc::now();
            match store.save_goal(&g).await {
                Ok(_) => format!("[goal] resumed — objective: \"{}\"", g.objective),
                Err(e) => format!("[goal] resume failed: {e}"),
            }
        }
        Err(_) => "[goal] no goal to resume.".to_string(),
    };
    SlashResult {
        outcome: SlashOutcome::Handled { reply: reply.clone() },
        reply,
    }
}

async fn clear(store: &Arc<dyn Store>, agent_id: &str, session_key: &str) -> SlashResult {
    let reply = match store.delete_goal(agent_id, session_key).await {
        Ok(_) => "[goal] cleared.".to_string(),
        Err(e) => format!("[goal] clear failed: {e}"),
    };
    SlashResult {
        outcome: SlashOutcome::Handled { reply: reply.clone() },
        reply,
    }
}

async fn create(
    store: &Arc<dyn Store>,
    agent_id: &str,
    session_key: &str,
    owner_user_id: &str,
    channel: &str,
    account_id: &str,
    chat_id: &str,
    project_id: &str,
    objective: &str,
) -> SlashResult {
    let objective = objective.trim();
    if objective.is_empty() {
        return SlashResult {
            outcome: SlashOutcome::Handled {
                reply: "[goal] usage: /goal <objective>".to_string(),
            },
            reply: "[goal] usage: /goal <objective>".to_string(),
        };
    }
    let now = Utc::now();
    // If a goal already exists for this (agent, session), update
    // its objective + reset the status to active. The CleanClaw
    // reference does the same: `/goal X` after a pause / complete
    // re-activates the goal with the new text.
    let existing = store.get_goal(agent_id, session_key).await.ok();
    let id = existing
        .as_ref()
        .map(|g| g.id.clone())
        .unwrap_or_else(|| format!("goal_{}", Uuid::new_v4().simple()));
    let rec = GoalRecord {
        id,
        agent_id: agent_id.into(),
        session_key: session_key.into(),
        owner_user_id: owner_user_id.into(),
        channel: channel.into(),
        account_id: account_id.into(),
        chat_id: chat_id.into(),
        project_id: project_id.into(),
        objective: objective.into(),
        status: "active".into(),
        token_budget: existing.as_ref().and_then(|g| g.token_budget),
        tokens_used: existing.as_ref().map(|g| g.tokens_used).unwrap_or(0),
        created_at: existing.as_ref().map(|g| g.created_at).unwrap_or(now),
        updated_at: now,
    };
    let reply = match store.save_goal(&rec).await {
        Ok(_) => format!("[goal] set: \"{}\"", rec.objective),
        Err(e) => format!("[goal] create failed: {e}"),
    };
    SlashResult {
        outcome: SlashOutcome::Handled { reply: reply.clone() },
        reply,
    }
}

/// Helper: parse the `/goal` portion out of a full user text
/// (which may include a leading `/goal` and whitespace). Returns
/// the args slice (no leading `/goal`).
pub fn parse_args(input: &str) -> Vec<String> {
    let trimmed = input.trim();
    let mut parts = trimmed.split_whitespace();
    let head = parts.next().unwrap_or("");
    if !head.eq_ignore_ascii_case("/goal") {
        return Vec::new();
    }
    parts.map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_store::models::UserRecord;
    use cleanclaw_store::sqlite::SqliteStore;

    async fn fresh_store() -> Arc<SqliteStore> {
        let st = SqliteStore::open(":memory:").await.unwrap();
        st.migrate().await.unwrap();
        Arc::new(st)
    }

    async fn make_user(store: &SqliteStore, id: &str) {
        let u = UserRecord {
            id: id.to_string(),
            username: id.to_string(),
            email: format!("{id}@x"),
            password_hash: "argon2id$...".into(),
            display_name: id.into(),
            role: "user".into(),
            status: "active".into(),
            apikey_id: String::new(),
            external_id: String::new(),
            avatar_url: String::new(),
            agent_quota: -1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.create_user(&u).await.unwrap();
    }

    #[test]
    fn parse_args_extracts_subcommand_args() {
        let args = parse_args("/goal pause");
        assert_eq!(args, vec!["pause"]);
        let args = parse_args("/goal ship it by Friday");
        assert_eq!(args, vec!["ship", "it", "by", "Friday"]);
    }

    #[test]
    fn parse_args_handles_no_subcommand() {
        let args = parse_args("/goal");
        assert!(args.is_empty());
    }

    #[test]
    fn parse_args_ignores_non_goal() {
        let args = parse_args("hello world");
        assert!(args.is_empty());
    }

    #[test]
    fn parse_args_case_insensitive() {
        let args = parse_args("/GOAL Pause");
        assert_eq!(args, vec!["Pause"]);
    }

    #[test]
    fn parse_args_trims_whitespace() {
        let args = parse_args("  /goal   resume  ");
        assert_eq!(args, vec!["resume"]);
    }

    #[tokio::test]
    async fn goal_create_then_show() {
        let st = fresh_store().await;
        make_user(&st, "u1").await;
        let st: Arc<dyn Store> = st;
        let args = vec!["ship", "cleanclaw", "v1"];
        let r = dispatch_goal(
            &st, "a1", "s1", "u1", "telegram", "bot1", "c1", "p1", &args,
        )
        .await;
        assert!(matches!(r.outcome, SlashOutcome::Handled { .. }));
        assert!(r.reply.contains("set"));
        // Show the goal we just created.
        let r = dispatch_goal(&st, "a1", "s1", "u1", "telegram", "bot1", "c1", "p1", &[]).await;
        assert!(r.reply.contains("active"));
        assert!(r.reply.contains("ship cleanclaw v1"));
    }

    #[tokio::test]
    async fn goal_pause_then_resume() {
        let st = fresh_store().await;
        make_user(&st, "u1").await;
        let st: Arc<dyn Store> = st;
        let args = vec!["long task"];
        dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &args).await;
        let r =
            dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["pause"]).await;
        assert!(r.reply.contains("paused"));
        let r =
            dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["resume"]).await;
        assert!(r.reply.contains("resumed"));
    }

    #[tokio::test]
    async fn goal_clear_removes_the_goal() {
        let st = fresh_store().await;
        make_user(&st, "u1").await;
        let st: Arc<dyn Store> = st;
        dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["x"]).await;
        let r = dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["clear"]).await;
        assert!(r.reply.contains("cleared"));
        // Subsequent show reports no goal.
        let r = dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &[]).await;
        assert!(r.reply.contains("no active goal"));
    }

    #[tokio::test]
    async fn goal_empty_objective_is_rejected() {
        let st = fresh_store().await;
        make_user(&st, "u1").await;
        let st: Arc<dyn Store> = st;
        let r = dispatch_goal(
            &st,
            "a1",
            "s1",
            "u1",
            "tg",
            "b1",
            "c1",
            "p1",
            &["   "], // whitespace-only
        )
        .await;
        assert!(r.reply.contains("usage"));
    }

    #[tokio::test]
    async fn goal_pause_on_missing_returns_message() {
        let st = fresh_store().await;
        make_user(&st, "u1").await;
        let st: Arc<dyn Store> = st;
        let r =
            dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["pause"]).await;
        assert!(r.reply.contains("no goal to pause"));
    }

    #[tokio::test]
    async fn goal_create_then_update_replaces_objective() {
        let st = fresh_store().await;
        make_user(&st, "u1").await;
        let st: Arc<dyn Store> = st;
        dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["first"]).await;
        // Re-issue /goal with new text — the existing record is
        // updated, not duplicated.
        dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &["second"]).await;
        let r = dispatch_goal(&st, "a1", "s1", "u1", "tg", "b1", "c1", "p1", &[]).await;
        assert!(r.reply.contains("second"));
        assert!(!r.reply.contains("first"));
    }
}
