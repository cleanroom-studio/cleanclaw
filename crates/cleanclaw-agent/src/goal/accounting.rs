//! Goal token-accounting fold. Mirrors
//! .
//!
//! `FoldUsage` applies one model call's token counts to a goal in
//! place, returning the delta added to `TokensUsed` and whether the
//! goal just crossed its budget. Non-Active goals are skipped
//! (paused / budget_limited / complete don't get billed). Cached
//! input is excluded at the provider layer (callers pass uncached
//! input tokens).

use crate::goal::GoalStatus;
use cleanclaw_store::models::GoalRecord;

/// Fold one model call's token counts into a goal, in place.
/// Returns `(delta_added, exhausted)`. A `delta` of 0 means the
/// fold was a no-op (non-active goal, or all-zero usage).
pub fn fold_usage(
    g: &mut GoalRecord,
    input_tokens: i64,
    output_tokens: i64,
) -> (i64, bool) {
    let status = GoalStatus::parse(&g.status);
    if status != GoalStatus::Active {
        return (0, false);
    }
    let delta = input_tokens.max(0) + output_tokens.max(0);
    if delta == 0 {
        return (0, false);
    }
    g.tokens_used += delta;
    let mut exhausted = false;
    if let Some(budget) = g.token_budget {
        if g.tokens_used >= budget {
            g.status = GoalStatus::BudgetLimited.as_str().into();
            exhausted = true;
        }
    }
    (delta, exhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn g(status: &str, used: i64, budget: Option<i64>) -> GoalRecord {
        GoalRecord {
            id: "g1".into(),
            agent_id: "a1".into(),
            session_key: "s1".into(),
            owner_user_id: "u1".into(),
            channel: "web".into(),
            account_id: String::new(),
            chat_id: "c1".into(),
            project_id: String::new(),
            objective: "ship".into(),
            status: status.into(),
            token_budget: budget,
            tokens_used: used,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn active_goal_accumulates() {
        let mut x = g("active", 100, Some(1000));
        let (delta, exhausted) = fold_usage(&mut x, 50, 30);
        assert_eq!(delta, 80);
        assert!(!exhausted);
        assert_eq!(x.tokens_used, 180);
    }

    #[test]
    fn active_goal_flipping_to_budget_limited() {
        let mut x = g("active", 950, Some(1000));
        let (delta, exhausted) = fold_usage(&mut x, 30, 30);
        assert_eq!(delta, 60);
        assert!(exhausted);
        assert_eq!(x.tokens_used, 1010);
        assert_eq!(x.status, "budget_limited");
    }

    #[test]
    fn paused_goal_skipped() {
        let mut x = g("paused", 100, Some(1000));
        let (delta, exhausted) = fold_usage(&mut x, 50, 30);
        assert_eq!(delta, 0);
        assert!(!exhausted);
        assert_eq!(x.tokens_used, 100);
    }

    #[test]
    fn complete_goal_skipped() {
        let mut x = g("complete", 100, Some(1000));
        let (delta, exhausted) = fold_usage(&mut x, 50, 30);
        assert_eq!(delta, 0);
        assert!(!exhausted);
    }

    #[test]
    fn unbounded_budget_never_exhausts() {
        let mut x = g("active", 0, None);
        let (_, exhausted) = fold_usage(&mut x, 1_000_000, 1_000_000);
        assert!(!exhausted);
        assert_eq!(x.status, "active");
    }

    #[test]
    fn zero_usage_is_noop() {
        let mut x = g("active", 100, Some(1000));
        let (delta, exhausted) = fold_usage(&mut x, 0, 0);
        assert_eq!(delta, 0);
        assert!(!exhausted);
        assert_eq!(x.tokens_used, 100);
    }

    #[test]
    fn negative_inputs_clamped_to_zero() {
        let mut x = g("active", 100, Some(1000));
        let (delta, _) = fold_usage(&mut x, -50, -30);
        assert_eq!(delta, 0);
    }
}
