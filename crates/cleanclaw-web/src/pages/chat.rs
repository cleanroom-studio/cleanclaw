//! Top-level `/chat` page — the chat landing.
//!
//! Mirrors the Next.js `web/src/app/chat/page.tsx`. With no
//! `?agent=<id>` query, the page lists the available agents and
//! invites the user to pick one. With a query, the server-side
//! handler (`server::chat`) redirects straight to the agent's
//! dedicated chat route.
//!
//! The page is intentionally a thin shell — the rich chat surface
//! (message rendering, streaming, tool-call display) lives in
//! `pages::agent::chat` once an agent is selected. The shell
//! here just provides the picker + a "no agents yet" empty state.

use crate::html::Theme;
use crate::layout::{self, NavKey};

/// Render the chat landing page. The agent list is static for
/// the parity sweep (the dashboard reads it via the gateway API
/// in the real impl); we surface the picker shape so the
/// `/chat` route resolves to a meaningful page.
pub fn render() -> String {
    let theme = Theme::Light;
    let body = body_html(theme);
    layout::render("Chat", NavKey::Chat, &body, None, theme)
}

fn body_html(_theme: Theme) -> String {
    // We use the same shell as `index.rs` — top app bar, no
    // sidebar (the chat is full-width once an agent is picked).
    // The color literals here match the shadcn-style tokens the
    // existing pages use (see `index.rs`).
    format!(
        r##"<header class="topbar">
  <div class="brand">🦀 CleanClaw</div>
  <nav class="topnav">
    <a href="/overview">Overview</a>
    <a href="/chat" class="active">Chat</a>
    <a href="/agents">Agents</a>
    <a href="/settings/general">Settings</a>
  </nav>
  <div class="user-menu"><a href="/logout">Sign out</a></div>
</header>
<main class="chat-landing">
  <div class="card">
    <h1>Chat</h1>
    <p class="muted">Pick an agent to start a session. The chat surface opens
       inside the agent's page, where you can review skills, plugins,
       and recent activity alongside the conversation.</p>

    <div class="agent-picker">
      <form method="get" action="/chat">
        <label for="agent">Agent</label>
        <select name="agent" id="agent" required>
          <option value="" disabled selected>Select an agent…</option>
        </select>
        <button type="submit">Open chat →</button>
      </form>
    </div>

    <div class="empty">
      <h2>No agents yet?</h2>
      <p>Create one from the <a href="/agents">Agents</a> page, or
         via the CLI:
         <code>cleanclaw agents init &lt;name&gt; --provider openai --model gpt-4o-mini</code></p>
    </div>
  </div>
</main>

<style>
  .chat-landing {{ max-width: 720px; margin: 48px auto; padding: 0 24px; }}
  .chat-landing .card {{ background: white; border: 1px solid hsl(220 13% 91%); border-radius: 12px; padding: 32px; }}
  .chat-landing h1 {{ margin: 0 0 8px 0; font-size: 28px; color: hsl(222 47% 11%); }}
  .chat-landing .muted {{ color: hsl(215 16% 47%); margin-bottom: 24px; }}
  .agent-picker form {{ display: flex; gap: 12px; align-items: flex-end; margin: 24px 0; }}
  .agent-picker label {{ font-weight: 600; color: hsl(222 47% 11%); }}
  .agent-picker select {{ padding: 8px 12px; border-radius: 6px; border: 1px solid hsl(220 13% 91%); min-width: 200px; background: white; }}
  .agent-picker button {{ padding: 8px 16px; border-radius: 6px; background: hsl(221 83% 53%); color: white; border: none; cursor: pointer; }}
  .empty {{ background: hsl(210 40% 98%); border-radius: 8px; padding: 16px; margin-top: 16px; }}
  .empty h2 {{ margin: 0 0 8px 0; font-size: 16px; color: hsl(222 47% 11%); }}
  .empty code {{ background: hsl(220 14% 96%); padding: 2px 6px; border-radius: 4px; font-size: 13px; color: hsl(222 47% 11%); }}
  .topbar {{ display: flex; align-items: center; gap: 24px; padding: 12px 24px; border-bottom: 1px solid hsl(220 13% 91%); background: white; }}
  .topbar .brand {{ font-weight: 700; font-size: 18px; color: hsl(222 47% 11%); }}
  .topbar .topnav {{ display: flex; gap: 16px; flex: 1; }}
  .topbar .topnav a {{ color: hsl(215 16% 47%); text-decoration: none; padding: 6px 8px; border-radius: 4px; }}
  .topbar .topnav a.active {{ color: hsl(222 47% 11%); background: hsl(210 40% 98%); }}
  .topbar .user-menu a {{ color: hsl(215 16% 47%); text-decoration: none; }}
</style>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_produces_html() {
        let html = render();
        assert!(html.contains("<title>Chat</title>") || html.contains(">Chat<"));
        assert!(html.contains("/chat"));
        assert!(html.contains("name=\"agent\""));
    }

    #[test]
    fn body_html_includes_picker() {
        let body = body_html(Theme::Light);
        assert!(body.contains("agent-picker"));
        assert!(body.contains("Open chat"));
    }
}
