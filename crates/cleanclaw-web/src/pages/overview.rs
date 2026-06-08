//! `/overview` — dashboard landing. Mirrors
//! .
//!
//! In W1 this is a static page. W4 wires it to the live
//! `StatusResponse` (agents count, channels count, recent activity).

use crate::html::{badge, card_close, card_content, card_header, card_open, card_title, Theme};
use crate::layout::{render as layout_render, NavKey};

/// Render the overview page. `user` is `(display_name, role)` if
/// authenticated; `None` renders the anonymous version.
pub fn render(user: Option<(&str, &str)>, theme: Theme) -> String {
    let cards = overview_cards();
    let body = format!(
        r#"<div class="space-y-4">
<h1 class="text-2xl font-semibold tracking-tight">Overview</h1>
<p class="text-muted-foreground">{welcome}</p>
<div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
{cards}
</div>
</div>"#,
        welcome = match user {
            Some((name, _)) => format!("Welcome back, {}.", name),
            None => "Sign in to see your agents, channels, and recent activity.".to_string(),
        },
        cards = cards,
    );
    layout_render("Overview · CleanClaw", NavKey::Overview, &body, user, theme)
}

fn overview_cards() -> String {
    let items = [
        ("Agents", "Configured agents and their model / workspace."),
        ("Channels", "Channel health and recent inbound messages."),
        ("Usage", "Token usage and request rate (last 24h)."),
        ("Skills", "Bundled + installed skills available to agents."),
        ("Cron", "Scheduled jobs grouped by agent and project."),
        ("Tools", "Built-in and plugin-provided tools."),
    ];
    let mut out = String::new();
    for (title, body) in items {
        out.push_str(&format!(
            r#"{open}{header}{title_}{content}<p class="text-sm text-muted-foreground">{body_}</p></div>{cclose}"#,
            open = card_open(""),
            header = card_header(),
            title_ = card_title(title),
            content = card_content(""),
            body_ = body,
            cclose = card_close(),
        ));
    }
    out
}

/// Health badge in the top-right corner. Mirrors the `badge("Healthy")`
/// in `overview/page.tsx`.
pub fn health_badge() -> String {
    badge("Healthy", crate::html::BadgeVariant::Default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_anon_omits_username() {
        let s = render(None, Theme::Light);
        assert!(s.contains("Sign in"));
        assert!(!s.contains("Welcome back"));
    }

    #[test]
    fn render_user_shows_name() {
        let s = render(Some(("Ada", "admin")), Theme::Light);
        assert!(s.contains("Welcome back, Ada"));
    }

    #[test]
    fn overview_lists_six_sections() {
        let s = render(None, Theme::Light);
        for label in ["Agents", "Channels", "Usage", "Skills", "Cron", "Tools"] {
            assert!(s.contains(label));
        }
    }
}
