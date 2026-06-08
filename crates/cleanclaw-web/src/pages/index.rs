//! `/` — landing page.
//! In the Next app this is the login redirector; in SSR form it
//! renders a "Get started" hero that links to `/signup` and
//! `/overview`.

use crate::css::BASE_CSS;
use crate::html::{badge, button, card_open, card_close, card_header, card_title, card_content, esc, page, ButtonSize, ButtonVariant, Theme};

/// Render the landing page. SSR-only — no client-side redirect.
pub fn render() -> String {
    let body = format!(
        r#"<div class="min-h-screen flex items-center justify-center p-6">
<div class="max-w-2xl w-full text-center space-y-4">
{badge}
<h1 class="text-3xl font-bold tracking-tight">CleanClaw</h1>
<p class="text-muted-foreground">Run your own AI agent stack — channels, skills, scheduler, and a web UI. Open source, single binary, no cloud.</p>
<div class="flex items-center justify-center gap-2 pt-2">
{btn_signup}
{btn_overview}
</div>
{cards}
</div>
</div>"#,
        badge = badge("Self-hosted", crate::html::BadgeVariant::Secondary),
        btn_signup = button("Create an account", ButtonVariant::Default, ButtonSize::Lg, Some("/signup")),
        btn_overview = button("Sign in", ButtonVariant::Outline, ButtonSize::Lg, Some("/overview")),
        cards = landing_cards(),
    );
    page("CleanClaw", &body, BASE_CSS, Theme::Light)
}

fn landing_cards() -> String {
    let mut out = String::new();
    let features = [
        ("Channels", "Telegram, Discord, Slack, Feishu, WeChat, Line, Webhook."),
        ("Skills", "Drop-in skills via tarball, GitHub, or ClawHub."),
        ("Scheduler", "Cron jobs per agent, per project, per channel."),
        ("Web UI", "Server-rendered, fast, no JS required."),
    ];
    for (title, body) in features {
        out.push_str(&format!(
            r#"{open}{header}{title_}{content}<p class="text-sm text-muted-foreground">{body_}</p>{close}{cclose}"#,
            open = card_open(""),
            header = card_header(),
            title_ = card_title(title),
            content = card_content(""),
            body_ = esc(body),
            close = "</div>",
            cclose = card_close(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_signup_link() {
        let s = render();
        assert!(s.contains(r#"href="/signup""#));
        assert!(s.contains(r#"href="/overview""#));
    }

    #[test]
    fn render_lists_features() {
        let s = render();
        assert!(s.contains("Channels"));
        assert!(s.contains("Skills"));
        assert!(s.contains("Scheduler"));
    }
}
