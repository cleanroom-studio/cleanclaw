//! Goal lifecycle hook. Mirrors
//! .
//!
//! `GoalHook` is a small callback type: given a `GoalRecord` and a
//! resolved `Session`, run a side-effect (typically: enqueue a
//! synthetic continuation `InboundMessage` onto the bus so the next
//! turn picks up the new state). The gateway sweeps goal status
//! changes periodically and fires registered hooks for ones that
//! transitioned.

use std::sync::Arc;

use cleanclaw_bus::MessageBus;
use cleanclaw_store::models::GoalRecord;
use tokio::sync::Mutex;

use crate::goal::GoalStatus;

/// Lightweight session-shaped payload. We avoid a hard dependency
/// on `cleanclaw_session::Session` here so the hook crate stays
/// testable without a full store; production callers pass a closure
/// that closes over an `Arc<cleanclaw_session::Session>` and returns
/// this as `Some(payload)`.
#[derive(Clone)]
pub struct GoalSessionPayload {
    pub session_key: String,
}

pub trait GoalSessionLike: Send + Sync {
    fn key(&self) -> &str;
}

impl GoalSessionLike for GoalSessionPayload {
    fn key(&self) -> &str {
        &self.session_key
    }
}

#[derive(Clone)]
pub struct GoalHook {
    inner: Arc<GoalHookInner>,
}

struct GoalHookInner {
    callback: Arc<
        dyn Fn(GoalRecord, Arc<dyn GoalSessionLike>) -> tokio::task::JoinHandle<()>
            + Send
            + Sync,
    >,
}

impl std::fmt::Debug for GoalHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoalHook").finish_non_exhaustive()
    }
}

impl GoalHook {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(GoalRecord, Arc<dyn GoalSessionLike>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let cb = Arc::new(move |g, s| {
            let fut = f(g, s);
            tokio::spawn(fut)
        });
        Self {
            inner: Arc::new(GoalHookInner { callback: cb }),
        }
    }

    pub fn noop() -> Self {
        Self::new(|_, _| async {})
    }

    pub async fn fire(&self, goal: GoalRecord, session: Arc<dyn GoalSessionLike>) {
        let cb = self.inner.callback.clone();
        cb(goal, session).await;
    }
}

impl Default for GoalHook {
    fn default() -> Self {
        Self::noop()
    }
}

pub struct GoalHookSubscription {
    hook: GoalHook,
    session_resolver: Arc<
        dyn Fn(&str, &str) -> Option<Arc<dyn GoalSessionLike>> + Send + Sync,
    >,
    last_status: Mutex<std::collections::HashMap<String, GoalStatus>>,
}

impl std::fmt::Debug for GoalHookSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoalHookSubscription")
            .field("hook", &self.hook)
            .finish_non_exhaustive()
    }
}

impl GoalHookSubscription {
    pub fn new(
        hook: GoalHook,
        session_resolver: Arc<
            dyn Fn(&str, &str) -> Option<Arc<dyn GoalSessionLike>> + Send + Sync,
        >,
    ) -> Self {
        Self {
            hook,
            session_resolver,
            last_status: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub async fn sweep(&self, goals: Vec<GoalRecord>) -> usize {
        let mut fired = 0;
        for g in goals {
            let new_status = GoalStatus::parse(&g.status);
            let prev = {
                let mut g_map = self.last_status.lock().await;
                let prev = g_map.get(&g.id).copied();
                g_map.insert(g.id.clone(), new_status);
                prev
            };
            if prev == Some(new_status) {
                continue;
            }
            if let Some(session) =
                (self.session_resolver)(&g.agent_id, &g.session_key)
            {
                self.hook.fire(g, session).await;
                fired += 1;
            }
        }
        fired
    }
}

/// Convenience constructor: build a hook that pushes a synthetic
/// continuation `InboundMessage` onto the bus so the next
/// `process_inbound` picks it up. Used by the production gateway.
pub fn continuation_hook(bus: Arc<MessageBus>) -> GoalHook {
    GoalHook::new(move |g, _session| {
        let bus = bus.clone();
        async move {
            let inbound = build_continuation_inbound(&g);
            let _ = bus.send_inbound(inbound).await;
        }
    })
}

fn build_continuation_inbound(g: &GoalRecord) -> cleanclaw_bus::InboundMessage {
    let text = format!(
        "[goal continuation] objective is still active\n\nObjective: {}\n\nStatus: {}\n\nContinue working on the objective.",
        g.objective, g.status
    );
    let mut m = cleanclaw_bus::InboundMessage::default();
    m.channel = g.channel.clone();
    m.account_id = g.account_id.clone();
    m.chat_id = g.chat_id.clone();
    m.user_id = g.owner_user_id.clone();
    m.owner_user_id = g.owner_user_id.clone();
    m.agent_id = g.agent_id.clone();
    m.message_id = format!("goal-cont:{}", g.id);
    m.text = text;
    m.peer_kind = "system".into();
    m.sender_name = "goal".into();
    m.source = cleanclaw_bus::SOURCE_GOAL_CONTEXT.to_string();
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn goal(id: &str, status: &str) -> GoalRecord {
        GoalRecord {
            id: id.to_string(),
            agent_id: "a1".into(),
            session_key: "s1".into(),
            owner_user_id: "u1".into(),
            channel: "web".into(),
            account_id: String::new(),
            chat_id: "c1".into(),
            project_id: String::new(),
            objective: "do thing".into(),
            status: status.into(),
            token_budget: None,
            tokens_used: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn dummy_session(key: &str) -> Arc<dyn GoalSessionLike> {
        Arc::new(GoalSessionPayload {
            session_key: key.to_string(),
        })
    }

    #[tokio::test]
    async fn continuation_hook_produces_valid_inbound() {
        let bus = Arc::new(MessageBus::new(4));
        let _hook = continuation_hook(bus.clone());
        let g = goal("g1", "active");
        let inbound = build_continuation_inbound(&g);
        assert_eq!(inbound.source, cleanclaw_bus::SOURCE_GOAL_CONTEXT);
        assert!(inbound.text.contains("do thing"));
        assert!(inbound.text.contains("active"));
    }

    #[tokio::test]
    async fn subscription_fires_when_session_resolves() {
        let sub = GoalHookSubscription::new(
            GoalHook::noop(),
            Arc::new(|_, k| Some(dummy_session(k))),
        );
        let n = sub.sweep(vec![goal("g1", "active")]).await;
        assert_eq!(n, 1);
        // Second sweep with same status is a no-op.
        let n = sub.sweep(vec![goal("g1", "active")]).await;
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn subscription_no_session_skips_fire() {
        let sub = GoalHookSubscription::new(
            GoalHook::noop(),
            Arc::new(|_, _| None),
        );
        let n = sub.sweep(vec![goal("g1", "active")]).await;
        assert_eq!(n, 0);
    }

    #[test]
    fn noop_hook_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GoalHook>();
    }
}
