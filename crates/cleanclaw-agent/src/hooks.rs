//! Hook registry — agent-lifecycle hooks (turn start, turn end,
//! tool pre-call, tool post-call, error). Mirrors
//! .
//!
//! Each hook is a boxed async closure that takes a JSON
//! payload and returns nothing. Hooks are wired at gateway boot
//! (system plugins register them). Failure inside a hook is
//! logged but does not fail the underlying operation — the
//! daemon stays up even if a plugin misbehaves.

use async_trait::async_trait;
use cleanclaw_core::Result;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

/// Hook phase — when the hook fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPhase {
    TurnStart,
    TurnEnd,
    ToolPreCall,
    ToolPostCall,
    Error,
}

impl HookPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookPhase::TurnStart => "turn_start",
            HookPhase::TurnEnd => "turn_end",
            HookPhase::ToolPreCall => "tool_pre_call",
            HookPhase::ToolPostCall => "tool_post_call",
            HookPhase::Error => "error",
        }
    }
}

#[async_trait]
pub trait Hook: Send + Sync {
    fn name(&self) -> &str;
    fn phase(&self) -> HookPhase;
    async fn run(&self, payload: Value) -> Result<()>;
}

pub struct HookRegistry {
    hooks: Vec<Arc<dyn Hook>>,
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }

    pub fn hooks_for(&self, phase: HookPhase) -> impl Iterator<Item = &Arc<dyn Hook>> {
        self.hooks.iter().filter(move |h| h.phase() == phase)
    }

    /// Fire all hooks for a given phase. Errors are logged, not
    /// returned — the caller doesn't act on hook failures.
    pub async fn fire(&self, phase: HookPhase, payload: Value) {
        for h in self.hooks_for(phase) {
            if let Err(e) = h.run(payload.clone()).await {
                warn!(hook = h.name(), phase = phase.as_str(), "hook failed: {e}");
            }
        }
    }

    pub fn count(&self) -> usize {
        self.hooks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CounterHook {
        name: String,
        phase: HookPhase,
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Hook for CounterHook {
        fn name(&self) -> &str { &self.name }
        fn phase(&self) -> HookPhase { self.phase }
        async fn run(&self, _payload: Value) -> Result<()> {
            self.counter.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn registry_routes_by_phase() {
        let counter_a = Arc::new(AtomicUsize::new(0));
        let counter_b = Arc::new(AtomicUsize::new(0));
        let mut r = HookRegistry::new();
        r.register(Arc::new(CounterHook {
            name: "a".into(),
            phase: HookPhase::TurnStart,
            counter: counter_a.clone(),
        }));
        r.register(Arc::new(CounterHook {
            name: "b".into(),
            phase: HookPhase::ToolPreCall,
            counter: counter_b.clone(),
        }));
        r.fire(HookPhase::TurnStart, json!({})).await;
        r.fire(HookPhase::ToolPreCall, json!({})).await;
        r.fire(HookPhase::Error, json!({})).await;
        assert_eq!(counter_a.load(Ordering::Relaxed), 1);
        assert_eq!(counter_b.load(Ordering::Relaxed), 1);
    }
}
