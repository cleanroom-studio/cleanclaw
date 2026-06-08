//! Goal continuation auto-fire. Mirrors
//! .
//!
//! `TryFireContinuation` is the gate cascade the post-turn hook
//! calls: load the goal for (agent, session), confirm it's
//! Active and has routing info, and publish a continuation
//! `InboundMessage` onto the bus tagged with `SOURCE_GOAL_CONTEXT`.
//! All failures are silent no-ops (best-effort — the next
//! `PostTurn` will retry).
//!
//! `Publish` is the lower-level helper used by both
//! `TryFireContinuation` and the budget-exhaustion branch of the
//! token-accounting hook.

use std::sync::Arc;

use cleanclaw_bus::{InboundMessage, MessageBus, SOURCE_GOAL_CONTEXT};
use cleanclaw_store::models::GoalRecord;
use cleanclaw_store::Store;
use tracing::warn;

use crate::goal::GoalStatus;
use crate::goal::prompt::continuation_prompt;

/// Try to fire a continuation prompt for the given (agent,
/// session). All errors are logged and swallowed — a failure
/// here doesn't leak into the caller's response path.
pub async fn try_fire_continuation(
    st: &Arc<dyn Store>,
    mb: &MessageBus,
    agent_id: &str,
    session_key: &str,
) {
    let g = match st.get_goal(agent_id, session_key).await {
        Ok(g) => g,
        Err(_) => return, // no goal or DB error — both are silent no-ops
    };
    if GoalStatus::parse(&g.status) != GoalStatus::Active {
        return;
    }
    if g.channel.is_empty() && g.chat_id.is_empty() {
        warn!(
            agent_id = agent_id,
            session_key = session_key,
            goal_id = %g.id,
            "goal continue: skipping — goal has no routing info"
        );
        return;
    }
    let prompt = continuation_prompt(&g);
    if !publish(mb, &g, prompt).await {
        warn!(
            agent_id = agent_id,
            session_key = session_key,
            "goal continue: bus full, dropped continuation"
        );
    }
}

/// Publish a goal-context prompt (continuation or budget-limit
/// wrap-up) onto the bus. Tagged with `SOURCE_GOAL_CONTEXT` so
/// the agent loop can distinguish runtime-injected goal prompts
/// from real user input. Returns `true` when the message was
/// accepted by the bus channel, `false` on full.
pub async fn publish(mb: &MessageBus, g: &GoalRecord, prompt: String) -> bool {
    let msg = InboundMessage {
        channel: g.channel.clone(),
        account_id: g.account_id.clone(),
        chat_id: g.chat_id.clone(),
        project_id: g.project_id.clone(),
        user_id: "goal".into(),
        owner_user_id: g.owner_user_id.clone(),
        agent_id: g.agent_id.clone(),
        message_id: format!("goal-cont:{}", g.id),
        text: prompt,
        peer_kind: "dm".into(),
        sender_name: "goal".into(),
        source: SOURCE_GOAL_CONTEXT.to_string(),
        ..Default::default()
    };
    mb.try_send_inbound(msg).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_core::now_utc;
    use cleanclaw_store::sqlite::SqliteStore;
    use chrono::Utc;

    async fn store() -> Arc<dyn Store> {
        let st = SqliteStore::open(":memory:").await.unwrap();
        st.migrate().await.unwrap();
        use cleanclaw_store::models::UserRecord;
        use cleanclaw_store::Store;
        let u = UserRecord {
            id: "u1".into(),
            username: "alice".into(),
            email: "a@x.com".into(),
            password_hash: String::new(),
            display_name: "alice".into(),
            role: "user".into(),
            status: "active".into(),
            apikey_id: String::new(),
            external_id: String::new(),
            avatar_url: String::new(),
            agent_quota: -1,
            created_at: now_utc(),
            updated_at: now_utc(),
        };
        st.create_user(&u).await.unwrap();
        Arc::new(st) as Arc<dyn Store>
    }

    fn g(status: &str, routing: bool) -> GoalRecord {
        GoalRecord {
            id: "g1".into(),
            agent_id: "a1".into(),
            session_key: "sk".into(),
            owner_user_id: "u1".into(),
            channel: if routing { "web".into() } else { String::new() },
            account_id: "u1".into(),
            chat_id: if routing { "c1".into() } else { String::new() },
            project_id: String::new(),
            objective: "ship".into(),
            status: status.into(),
            token_budget: None,
            tokens_used: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn publish_tags_with_goal_context_source() {
        use cleanclaw_bus::MessageBus;
        let bus = MessageBus::new(4);
        let goal = g("active", true);
        let ok = publish(&bus, &goal, "hi".into()).await;
        assert!(ok);
        let msg = bus.recv_inbound().await.unwrap();
        assert_eq!(msg.source, SOURCE_GOAL_CONTEXT);
        assert!(msg.text.contains("hi"));
    }

    #[tokio::test]
    async fn try_fire_no_goal_is_silent_noop() {
        use cleanclaw_bus::MessageBus;
        let st = store().await;
        let bus = MessageBus::new(4);
        try_fire_continuation(&st, &bus, "a1", "sk").await;
        // Bus should still be empty.
        let outcome =
            tokio::time::timeout(std::time::Duration::from_millis(50), bus.recv_inbound())
                .await;
        assert!(outcome.is_err() || outcome.unwrap().is_none());
    }

    #[tokio::test]
    async fn try_fire_skips_paused_goals() {
        use cleanclaw_bus::MessageBus;
        use cleanclaw_store::Store;
        let st = store().await;
        st.save_goal(&g("paused", true)).await.unwrap();
        let bus = MessageBus::new(4);
        try_fire_continuation(&st, &bus, "a1", "sk").await;
        let outcome =
            tokio::time::timeout(std::time::Duration::from_millis(50), bus.recv_inbound())
                .await;
        assert!(outcome.is_err() || outcome.unwrap().is_none());
    }

    #[tokio::test]
    async fn try_fire_skips_goals_without_routing() {
        use cleanclaw_bus::MessageBus;
        use cleanclaw_store::Store;
        let st = store().await;
        st.save_goal(&g("active", false)).await.unwrap();
        let bus = MessageBus::new(4);
        try_fire_continuation(&st, &bus, "a1", "sk").await;
        let outcome =
            tokio::time::timeout(std::time::Duration::from_millis(50), bus.recv_inbound())
                .await;
        assert!(outcome.is_err() || outcome.unwrap().is_none());
    }
}
