//! Continuation prompts for active goals. Mirrors
//! .
//!
//! Two prompts:
//!   - `ContinuationPrompt` — the per-turn audit prompt injected
//!     while the goal is Active
//!   - `BudgetLimitPrompt` — the wrap-up prompt injected once on
//!     the turn that flips a goal to BudgetLimited
//!
//! Both prompts are loaded via `include_str!` from sibling
//! `templates/*.md` files (no runtime templating — the
//! variable substitution is plain `{placeholder}` replaced with
//! `format!`).
//!
//! The `EscapeXMLText` helper mirrors CleanClaw's `EscapeXMLText`:
//! it replaces `&`/`<`/`>` to keep user-supplied objective text
//! from breaking out of the `<objective>` wrapper or forging a
//! `</goal_context>` close tag.

use cleanclaw_store::models::GoalRecord;

const CONTINUATION_TEMPLATE: &str = include_str!("templates/continuation.md");
const BUDGET_LIMIT_TEMPLATE: &str = include_str!("templates/budget_limit.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Continuation,
    BudgetLimit,
}

pub fn continuation_prompt(g: &GoalRecord) -> String {
    let vars = render_vars(g);
    format_prompt(CONTINUATION_TEMPLATE, &vars)
}

pub fn budget_limit_prompt(g: &GoalRecord) -> String {
    let vars = render_vars(g);
    format_prompt(BUDGET_LIMIT_TEMPLATE, &vars)
}

struct Vars {
    objective: String,
    tokens_used: i64,
    token_budget: String,
    remaining_tokens: String,
}

fn render_vars(g: &GoalRecord) -> Vars {
    let token_budget = match g.token_budget {
        Some(b) => b.to_string(),
        None => "none".to_string(),
    };
    let remaining_tokens = match g.token_budget {
        Some(b) => (b - g.tokens_used).max(0).to_string(),
        None => "unbounded".to_string(),
    };
    Vars {
        objective: escape_xml_text(&g.objective),
        tokens_used: g.tokens_used,
        token_budget,
        remaining_tokens,
    }
}

fn format_prompt(tmpl: &str, v: &Vars) -> String {
    tmpl.replace("{{.Objective}}", &v.objective)
        .replace("{{.TokensUsed}}", &v.tokens_used.to_string())
        .replace("{{.TokenBudget}}", &v.token_budget)
        .replace("{{.RemainingTokens}}", &v.remaining_tokens)
}

pub fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn g(budget: Option<i64>, used: i64) -> GoalRecord {
        GoalRecord {
            id: "g1".into(),
            agent_id: "a".into(),
            session_key: "sk".into(),
            owner_user_id: "u".into(),
            channel: "web".into(),
            account_id: String::new(),
            chat_id: "c".into(),
            project_id: String::new(),
            objective: "ship the demo".into(),
            status: "active".into(),
            token_budget: budget,
            tokens_used: used,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn continuation_prompt_contains_objective_and_tokens() {
        let p = continuation_prompt(&g(Some(10_000), 1_500));
        assert!(p.contains("ship the demo"));
        assert!(p.contains("1500"));
        assert!(p.contains("10000"));
        assert!(p.contains("8500"));
    }

    #[test]
    fn budget_limit_prompt_uses_none_when_no_budget() {
        let p = budget_limit_prompt(&g(None, 0));
        assert!(p.contains("ship the demo"));
        // budget_limit.md renders {{.TokenBudget}} as-is. With no
        // budget set we substitute "none" via the renderer.
        assert!(p.contains("none"));
    }

    #[test]
    fn escape_xml_text_replaces_ampersand_and_angle_brackets() {
        let s = "foo & <bar> baz";
        let e = escape_xml_text(s);
        assert_eq!(e, "foo &amp; &lt;bar&gt; baz");
    }

    #[test]
    fn continuation_prompt_escapes_objective() {
        // A user-supplied objective that contains < and > must not
        // be allowed to break the wrapper.
        let mut x = g(Some(100), 0);
        x.objective = "x <y> & z".into();
        let p = continuation_prompt(&x);
        assert!(p.contains("x &lt;y&gt; &amp; z"));
    }
}
