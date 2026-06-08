//! Agent workspace pages. Mirrors the 12 sub-routes under
//!  plus the parent
//! `/agents` list + `/agents/[id]` overview.
//!
//! Each sub-page is a server-rendered card layout that lives inside
//! the same agent shell. The shell is shared (rendered once per
//! page) and the sub-tab strip drives navigation between them.

use crate::html::{card_close, card_content, card_header, card_open, card_title, esc, tabs, Theme};
use crate::layout::{render, NavKey};
use crate::types::*;
use cleanclaw_store::models::{SessionMessageRecord, SessionRecord};

/// Agent sub-tab key. Drives the active highlight on the tab strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTab {
    Overview,
    Chat,
    Chats,
    Sessions,
    Channels,
    Scheduler,
    Skills,
    Plugins,
    Models,
    Context,
    Customize,
    Project,
    Usage,
}

impl AgentTab {
    pub fn href(self, id: &str) -> String {
        let base = format!("/agents/{}", urlencode_path(id));
        match self {
            AgentTab::Overview => base,
            AgentTab::Chat => format!("{base}/chat"),
            AgentTab::Chats => format!("{base}/chats"),
            AgentTab::Sessions => format!("{base}/sessions"),
            AgentTab::Channels => format!("{base}/channels"),
            AgentTab::Scheduler => format!("{base}/scheduler"),
            AgentTab::Skills => format!("{base}/skills"),
            AgentTab::Plugins => format!("{base}/plugins"),
            AgentTab::Models => format!("{base}/models"),
            AgentTab::Context => format!("{base}/context"),
            AgentTab::Customize => format!("{base}/customize"),
            AgentTab::Project => base, // landing
            AgentTab::Usage => format!("{base}/usage"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AgentTab::Overview => "Overview",
            AgentTab::Chat => "Chat",
            AgentTab::Chats => "Chats",
            AgentTab::Sessions => "Sessions",
            AgentTab::Channels => "Channels",
            AgentTab::Scheduler => "Scheduler",
            AgentTab::Skills => "Skills",
            AgentTab::Plugins => "Plugins",
            AgentTab::Models => "Models",
            AgentTab::Context => "Context",
            AgentTab::Customize => "Customize",
            AgentTab::Project => "Project",
            AgentTab::Usage => "Usage",
        }
    }
}

fn urlencode_path(s: &str) -> String {
    crate::client::urlencode(s)
}

/// Render the agent sub-tab strip.
pub fn agent_tabs(id: &str, active: AgentTab) -> String {
    let labels = [
        ("overview", "Overview"),
        ("chat", "Chat"),
        ("chats", "Chats"),
        ("sessions", "Sessions"),
        ("channels", "Channels"),
        ("scheduler", "Scheduler"),
        ("skills", "Skills"),
        ("plugins", "Plugins"),
        ("models", "Models"),
        ("context", "Context"),
        ("customize", "Customize"),
        ("usage", "Usage"),
    ];
    let _ = labels.iter().map(|(_, l)| *l).collect::<Vec<_>>();
    // The tabs helper expects `(&str, &str)` tuples. We patch the
    // hrefs into the buttons via a post-render transform, but for
    // SSR we just render the labels and let the agent layout wrap
    // each as an `<a>` to the right URL.
    let mut out = String::new();
    out.push_str(r#"<div class="inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground flex-wrap" role="tablist">"#);
    for (key, label) in labels {
        let is_active = label == active.label();
        let cls = if is_active {
            "inline-flex items-center justify-center whitespace-nowrap rounded-md bg-background px-3 py-1 text-sm font-medium shadow"
        } else {
            "inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-all hover:text-foreground"
        };
        let href = match key {
            "overview" => format!("/agents/{}", urlencode_path(id)),
            "chat" => format!("/agents/{}/chat", urlencode_path(id)),
            "chats" => format!("/agents/{}/chats", urlencode_path(id)),
            "sessions" => format!("/agents/{}/sessions", urlencode_path(id)),
            "channels" => format!("/agents/{}/channels", urlencode_path(id)),
            "scheduler" => format!("/agents/{}/scheduler", urlencode_path(id)),
            "skills" => format!("/agents/{}/skills", urlencode_path(id)),
            "plugins" => format!("/agents/{}/plugins", urlencode_path(id)),
            "models" => format!("/agents/{}/models", urlencode_path(id)),
            "context" => format!("/agents/{}/context", urlencode_path(id)),
            "customize" => format!("/agents/{}/customize", urlencode_path(id)),
            "usage" => format!("/agents/{}/usage", urlencode_path(id)),
            _ => String::new(),
        };
        out.push_str(&format!(
            r#"<a class="{cls}" href="{href}" role="tab">{label}</a>"#,
            cls = cls,
            href = esc(&href),
            label = esc(label),
        ));
    }
    out.push_str("</div>");
    out
}

/// Render the agent shell. Returns the body fragment. The actual
/// `<html>` envelope comes from `layout::render`.
pub fn agent_shell(id: &str, active: AgentTab, name: Option<&str>, body: &str) -> String {
    let display = name.unwrap_or(id);
    format!(
        r#"<div class="space-y-4">
<div class="flex items-center justify-between">
<h1 class="text-2xl font-semibold tracking-tight">{display}</h1>
<span class="text-sm text-muted-foreground">{id}</span>
</div>
{tabs}
{body}
</div>"#,
        display = esc(display),
        id = esc(id),
        tabs = agent_tabs(id, active),
        body = body,
    )
}

fn page(id: &str, active: AgentTab, name: Option<&str>, body: &str, theme: Theme) -> String {
    let inner = agent_shell(id, active, name, body);
    render(
        &format!("{} · CleanClaw", active.label()),
        NavKey::Agents,
        &inner,
        Some(("Ada", "user")),
        theme,
    )
}

// =====================================================================
// `/agents` — list + create
// =====================================================================

pub fn list(theme: Theme, agents: &[AgentDetail]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for a in agents {
        rows.push(vec![
            esc(&a.id),
            esc(a.name.as_deref().unwrap_or("")),
            esc(&a.model),
            esc(a.workspace.as_deref().unwrap_or("")),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{create_btn}
{table}
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Agents"),
        card_content = card_content(""),
        create_btn = crate::html::button(
            "New agent",
            crate::html::ButtonVariant::Default,
            crate::html::ButtonSize::Sm,
            Some("/agents/new")
        ),
        table = crate::html::table(&["ID", "Name", "Model", "Workspace"], &rows),
        card_close = card_close(),
    );
    let inner = format!(
        r#"<div class="space-y-4">
<h1 class="text-2xl font-semibold tracking-tight">Agents</h1>
{body}
</div>"#
    );
    render(
        "Agents · CleanClaw",
        NavKey::Agents,
        &inner,
        Some(("Ada", "user")),
        theme,
    )
}

// =====================================================================
// `/agents/{id}` — overview
// =====================================================================

pub fn overview(id: &str, agent: Option<&AgentDetail>, theme: Theme) -> String {
    let card_body = match agent {
        Some(a) => format!(
            r#"<dl class="space-y-1 text-sm">
<dt class="font-medium">Model</dt><dd class="text-muted-foreground">{model}</dd>
<dt class="font-medium">Workspace</dt><dd class="text-muted-foreground">{ws}</dd>
<dt class="font-medium">Max tokens</dt><dd class="text-muted-foreground">{mt}</dd>
<dt class="font-medium">Temperature</dt><dd class="text-muted-foreground">{temp}</dd>
</dl>"#,
            model = esc(&a.model),
            ws = esc(a.workspace.as_deref().unwrap_or("")),
            mt = a.max_tokens.unwrap_or(0),
            temp = a.temperature.unwrap_or(0.0),
        ),
        None => r#"<p class="text-sm text-muted-foreground">No agent data.</p>"#.to_string(),
    };
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{card_body}
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Overview"),
        card_content = card_content(""),
        card_body = card_body,
        card_close = card_close(),
    );
    page(
        id,
        AgentTab::Overview,
        agent.and_then(|a| a.name.as_deref()),
        &body,
        theme,
    )
}

// =====================================================================
// Sub-pages
// =====================================================================

pub fn chat(id: &str, theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<p class="text-sm text-muted-foreground">Open an existing chat by id, or start a new one.</p>
<form method="POST" action="/agents/{id_enc}/chat" class="mt-4 space-y-2">
{label}
{input}
{submit}
</form>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Chat"),
        card_content = card_content(""),
        id_enc = urlencode_path(id),
        label = crate::html::label("Session id (leave empty for new)", "session"),
        input = crate::html::input("session", "session_xxx", "", "text"),
        submit = crate::html::button(
            "Open chat",
            crate::html::ButtonVariant::Default,
            crate::html::ButtonSize::Default,
            None
        ),
        card_close = card_close(),
    );
    page(id, AgentTab::Chat, None, &body, theme)
}

/// `GET /agents/:id/sessions/:sid` — the chat surface.
//
/// Renders the message history (from the `session_messages`
/// archive) and a composer at the bottom. The embedded JS
/// client (`/static/ws-chat.js`) opens a WS to
/// `/api/ws/chat`, sends new messages, and renders streaming
/// deltas inline. This is the page the user spends most of
/// their time on.
//
/// `session` may be `None` for sessions that exist in the
/// `session_messages` archive but somehow lost their
/// `SessionRecord` (e.g. partial migration). In that case we
/// still render the history but skip the title row.
pub fn chat_surface(
    id: &str,
    sid: &str,
    session: Option<&SessionRecord>,
    messages: &[SessionMessageRecord],
    _unused: &[()],
    theme: Theme,
) -> String {
    let id_enc = urlencode_path(id);
    let sid_enc = urlencode_path(sid);
    let title = session
        .map(|s| s.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "(untitled session)".to_string());
    let title_esc = esc(&title);

    // Render the message history. User messages go on the
    // right; assistant messages + tool calls go on the left.
    // We pre-render the history server-side so the page is
    // useful even before the JS client boots.
    let mut history_html = String::new();
    for m in messages {
        let role = m.role.as_str();
        let content_esc = esc(&m.content);
        let cls = if role == "user" {
            "chat-msg chat-msg-user"
        } else {
            "chat-msg chat-msg-assistant"
        };
        let role_label = if role == "user" { "you" } else { "assistant" };
        history_html.push_str(&format!(
            r#"<div class="{cls}" data-msg-id="{seq}">
  <div class="chat-msg-role">{role_label}</div>
  <div class="chat-msg-body">{content_esc}</div>
</div>"#,
            cls = cls,
            seq = m.seq,
            role_label = role_label,
            content_esc = content_esc,
        ));
        // Tool calls (assistant only) — render as a small
        // sub-card. We escape the JSON; the JS will pretty-print
        // it on hover.
        if !m.tool_calls.is_null() && m.tool_calls != serde_json::Value::Array(Vec::new()) {
            let tc_json = esc(&m.tool_calls.to_string());
            history_html.push_str(&format!(
                r#"<div class="chat-tool" data-msg-id="{seq}">
  <div class="chat-tool-label">tool calls</div>
  <pre class="chat-tool-pre">{tc_json}</pre>
</div>"#,
                seq = m.seq,
                tc_json = tc_json,
            ));
        }
    }

    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<div class="chat-surface" data-agent-id="{id_enc}" data-session-id="{sid_enc}">
  <div class="chat-header">
    <div class="chat-title">{title_esc}</div>
    <div class="chat-meta">
      <span data-chat-status>idle</span>
      <a href="/agents/{id_enc}/chats" class="chat-back">← back to sessions</a>
    </div>
  </div>
  <div class="chat-history" data-chat-history>
{history_html}
  </div>
  <form class="chat-composer" data-chat-form>
    <textarea
      name="message"
      placeholder="Type your message…"
      required
      rows="3"
      data-chat-input
    ></textarea>
    <button type="submit" class="chat-send" data-chat-send>Send</button>
  </form>
</div>
</div>
{card_close}
<script src="/static/ws-chat.js" defer></script>"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Chat"),
        card_content = card_content(""),
        id_enc = id_enc,
        sid_enc = sid_enc,
        title_esc = title_esc,
        history_html = history_html,
        card_close = card_close(),
    );
    page(id, AgentTab::Chat, Some(&title), &body, theme)
}

pub fn chats(id: &str, sessions: &[ChatSessionEntry], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for s in sessions {
        rows.push(vec![
            esc(&s.id),
            esc(s.title.as_deref().unwrap_or("")),
            esc(&s.preview),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{table}
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Chats"),
        card_content = card_content(""),
        table = crate::html::table(&["ID", "Title", "Preview"], &rows),
        card_close = card_close(),
    );
    page(id, AgentTab::Chats, None, &body, theme)
}

pub fn sessions(id: &str, sessions: &[ChatSessionEntry], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for s in sessions {
        rows.push(vec![
            esc(&s.id),
            esc(s.project_id.as_deref().unwrap_or("")),
            esc(s.title.as_deref().unwrap_or("")),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{table}
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Sessions"),
        card_content = card_content(""),
        table = crate::html::table(&["ID", "Project", "Title"], &rows),
        card_close = card_close(),
    );
    page(id, AgentTab::Sessions, None, &body, theme)
}

pub fn channels(id: &str, channels: &[AgentChannel], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in channels {
        rows.push(vec![
            esc(&c.kind),
            esc(&c.account_id),
            esc(c.bot_username.as_deref().unwrap_or("")),
            esc(&c.bot_token),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">Type</th>
<th class="h-10 px-2 text-left">Account</th>
<th class="h-10 px-2 text-left">Username</th>
<th class="h-10 px-2 text-left">Token (masked)</th>
</tr></thead><tbody>{rows_html}</tbody></table>
<div class="mt-4 space-y-2">
{connect_buttons}
</div>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Channels"),
        card_content = card_content(""),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="4" class="p-2 text-muted-foreground text-sm">No channels connected.</td></tr>"#.to_string()
        } else {
            rows.iter()
                .map(|r| {
                    format!(
                        r#"<tr class="border-b">{}</tr>"#,
                        r.iter()
                            .map(|c| format!(r#"<td class="p-2">{}</td>"#, c))
                            .collect::<String>()
                    )
                })
                .collect::<String>()
        },
        connect_buttons = [
            ("Telegram", "telegram"),
            ("Discord", "discord"),
            ("Slack", "slack"),
            ("LINE", "line"),
            ("Feishu", "feishu"),
        ]
        .iter()
        .map(|(l, k)| crate::html::button(
            l,
            crate::html::ButtonVariant::Outline,
            crate::html::ButtonSize::Sm,
            Some(&format!("/agents/{}/channels/{}", urlencode_path(id), k)),
        ))
        .collect::<String>(),
        card_close = card_close(),
    );
    page(id, AgentTab::Channels, None, &body, theme)
}

pub fn scheduler(id: &str, jobs: &[AgentCronJob], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for j in jobs {
        rows.push(vec![
            esc(&j.name),
            esc(&j.kind),
            esc(&j.schedule),
            esc(j.next_run.as_deref().unwrap_or("")),
            if j.enabled { "yes" } else { "no" }.to_string(),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{table}
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Scheduler"),
        card_content = card_content(""),
        table = crate::html::table(&["Name", "Type", "Schedule", "Next run", "Enabled"], &rows),
        card_close = card_close(),
    );
    page(id, AgentTab::Scheduler, None, &body, theme)
}

pub fn skills(id: &str, skills: &[SkillInfo], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for s in skills {
        rows.push(vec![esc(&s.name), esc(&s.description), esc(&s.location)]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">Description</th>
<th class="h-10 px-2 text-left">Location</th>
</tr></thead><tbody>{rows_html}</tbody></table>
<p class="text-sm text-muted-foreground mt-4">Per-agent skills override the global skill table.</p>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Skills"),
        card_content = card_content(""),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="3" class="p-2 text-muted-foreground text-sm">No skills installed.</td></tr>"#.to_string()
        } else {
            rows.iter()
                .map(|r| {
                    format!(
                        r#"<tr class="border-b">{}</tr>"#,
                        r.iter()
                            .map(|c| format!(r#"<td class="p-2">{}</td>"#, c))
                            .collect::<String>()
                    )
                })
                .collect::<String>()
        },
        card_close = card_close(),
    );
    page(id, AgentTab::Skills, None, &body, theme)
}

pub fn plugins(id: &str, plugins: &[HookPlugin], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for p in plugins {
        rows.push(vec![
            esc(&p.id),
            esc(p.name.as_deref().unwrap_or("")),
            esc(p.version.as_deref().unwrap_or("")),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<p class="text-sm text-muted-foreground mb-4">Hook plugins available for this agent.</p>
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">ID</th>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">Version</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Plugins"),
        card_content = card_content(""),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="3" class="p-2 text-muted-foreground text-sm">No hook plugins enabled.</td></tr>"#.to_string()
        } else {
            rows.iter()
                .map(|r| {
                    format!(
                        r#"<tr class="border-b">{}</tr>"#,
                        r.iter()
                            .map(|c| format!(r#"<td class="p-2">{}</td>"#, c))
                            .collect::<String>()
                    )
                })
                .collect::<String>()
        },
        card_close = card_close(),
    );
    page(id, AgentTab::Plugins, None, &body, theme)
}

pub fn models(id: &str, agent: Option<&AgentFileConfig>, theme: Theme) -> String {
    let body = match agent {
        Some(c) => format!(
            r#"<dl class="space-y-1 text-sm">
<dt class="font-medium">Model</dt><dd class="text-muted-foreground">{model}</dd>
<dt class="font-medium">Max tokens</dt><dd class="text-muted-foreground">{mt}</dd>
<dt class="font-medium">Temperature</dt><dd class="text-muted-foreground">{temp}</dd>
<dt class="font-medium">Max tool iterations</dt><dd class="text-muted-foreground">{mti}</dd>
<dt class="font-medium">Workspace</dt><dd class="text-muted-foreground">{ws}</dd>
</dl>"#,
            model = esc(c.model.as_deref().unwrap_or("")),
            mt = c.max_tokens.unwrap_or(0),
            temp = c.temperature.unwrap_or(0.0),
            mti = c.max_tool_iterations.unwrap_or(0),
            ws = esc(c.workspace.as_deref().unwrap_or("")),
        ),
        None => {
            r#"<p class="text-sm text-muted-foreground">No agent config available.</p>"#.to_string()
        }
    };
    let card = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{body}
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Models"),
        card_content = card_content(""),
        body = body,
        card_close = card_close(),
    );
    page(id, AgentTab::Models, None, &card, theme)
}

pub fn context(id: &str, theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<p class="text-sm text-muted-foreground">Per-agent context: model, skills, plugins. Edit and save from the Customize tab.</p>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Context"),
        card_content = card_content(""),
        card_close = card_close(),
    );
    page(id, AgentTab::Context, None, &body, theme)
}

pub fn customize(id: &str, theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<form method="POST" action="/agents/{id_enc}/customize" class="space-y-4">
{label_name}
{input_name}
{label_desc}
{textarea_desc}
{label_prompt_mode}
<select class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" name="promptMode">
<option value="">No override</option>
<option value="agent">agent</option>
<option value="chatbot">chatbot</option>
<option value="customize">customize</option>
</select>
{label_soul}
{textarea_soul}
<button class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground">Save</button>
</form>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Customize"),
        card_content = card_content(""),
        id_enc = urlencode_path(id),
        label_name = crate::html::label("Display name", "name"),
        input_name = crate::html::input("name", "Agent name", "", "text"),
        label_desc = crate::html::label("Description", "description"),
        textarea_desc = crate::html::textarea("description", "What does this agent do?", "", 4),
        label_prompt_mode = crate::html::label("Prompt mode", "promptMode"),
        label_soul = crate::html::label("Soul (system prompt)", "soul"),
        textarea_soul = crate::html::textarea("soul", "You are...", "", 8),
        card_close = card_close(),
    );
    page(id, AgentTab::Customize, None, &body, theme)
}

pub fn project(id: &str, projects: &[ProjectEntry], theme: Theme) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for p in projects {
        rows.push(vec![
            esc(&p.id),
            esc(&p.name),
            esc(p.description.as_deref().unwrap_or("")),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">ID</th>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">Description</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Project"),
        card_content = card_content(""),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="3" class="p-2 text-muted-foreground text-sm">No projects yet.</td></tr>"#.to_string()
        } else {
            rows.iter()
                .map(|r| {
                    format!(
                        r#"<tr class="border-b">{}</tr>"#,
                        r.iter()
                            .map(|c| format!(r#"<td class="p-2">{}</td>"#, c))
                            .collect::<String>()
                    )
                })
                .collect::<String>()
        },
        card_close = card_close(),
    );
    page(id, AgentTab::Project, None, &body, theme)
}

pub fn project_id(id: &str, project_id: &str, theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<p class="text-sm text-muted-foreground">Project landing for {pid}.</p>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Project"),
        card_content = card_content(""),
        pid = esc(project_id),
        card_close = card_close(),
    );
    page(id, AgentTab::Project, None, &body, theme)
}

pub fn usage(id: &str, usage_data: Option<&AgentTokenUsage>, theme: Theme) -> String {
    let body = match usage_data {
        Some(u) => {
            let mut rows: Vec<Vec<String>> = Vec::new();
            for s in &u.sessions {
                rows.push(vec![
                    esc(&s.key),
                    s.tokens.to_string(),
                    s.request_count.to_string(),
                ]);
            }
            format!(
                r#"{card_open}
{card_header}
{card_title}
{card_content}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">Session</th>
<th class="h-10 px-2 text-left">Tokens</th>
<th class="h-10 px-2 text-left">Requests</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
                card_open = card_open(""),
                card_header = card_header(),
                card_title =
                    card_title(format!("Usage · last {range}", range = u.range.as_str()).as_str()),
                card_content = card_content(""),
                rows_html = if rows.is_empty() {
                    r#"<tr><td colspan="3" class="p-2 text-muted-foreground text-sm">No usage data.</td></tr>"#.to_string()
                } else {
                    rows.iter()
                        .map(|r| {
                            format!(
                                r#"<tr class="border-b">{}</tr>"#,
                                r.iter()
                                    .map(|c| format!(r#"<td class="p-2">{}</td>"#, c))
                                    .collect::<String>()
                            )
                        })
                        .collect::<String>()
                },
                card_close = card_close(),
            )
        }
        None => format!(
            r#"{card_open}
{card_header}
{card_title}
{card_content}
<p class="text-sm text-muted-foreground">No usage data in this range.</p>
</div>
{card_close}"#,
            card_open = card_open(""),
            card_header = card_header(),
            card_title = card_title("Usage"),
            card_content = card_content(""),
            card_close = card_close(),
        ),
    };
    page(id, AgentTab::Usage, None, &body, theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tab_hrefs() {
        assert_eq!(AgentTab::Chat.href("a1"), "/agents/a1/chat");
        assert_eq!(AgentTab::Chats.href("a 1"), "/agents/a%201/chats");
        assert_eq!(AgentTab::Usage.href("a1"), "/agents/a1/usage");
    }

    #[test]
    fn agent_tab_labels() {
        assert_eq!(AgentTab::Overview.label(), "Overview");
        assert_eq!(AgentTab::Project.label(), "Project");
    }

    #[test]
    fn list_renders_empty() {
        let s = list(Theme::Light, &[]);
        assert!(s.contains("Agents"));
        assert!(s.contains("New agent"));
    }

    #[test]
    fn overview_renders_with_agent() {
        let a = AgentDetail {
            id: "a1".into(),
            name: Some("Ada".into()),
            model: "gpt-4o".into(),
            workspace: Some("/tmp".into()),
            max_tokens: Some(1024),
            temperature: Some(0.7),
            ..Default::default()
        };
        let s = overview("a1", Some(&a), Theme::Light);
        assert!(s.contains("gpt-4o"));
        assert!(s.contains("Ada"));
    }

    #[test]
    fn chat_renders_form() {
        let s = chat("a1", Theme::Light);
        assert!(s.contains("Open chat"));
        assert!(s.contains(r#"name="session""#));
    }

    #[test]
    fn chats_renders_empty() {
        let s = chats("a1", &[], Theme::Light);
        assert!(s.contains("Chats"));
    }

    #[test]
    fn sessions_renders_rows() {
        let ses = vec![ChatSessionEntry {
            id: "s1".into(),
            title: Some("Hi".into()),
            project_id: Some("p1".into()),
            ..Default::default()
        }];
        let s = sessions("a1", &ses, Theme::Light);
        assert!(s.contains("s1"));
        assert!(s.contains("Hi"));
    }

    #[test]
    fn channels_renders_empty() {
        let s = channels("a1", &[], Theme::Light);
        assert!(s.contains("Channels"));
        assert!(s.contains("No channels connected"));
        assert!(s.contains("Telegram"));
    }

    #[test]
    fn scheduler_renders_empty() {
        let s = scheduler("a1", &[], Theme::Light);
        assert!(s.contains("Scheduler"));
    }

    #[test]
    fn skills_renders_empty() {
        let s = skills("a1", &[], Theme::Light);
        assert!(s.contains("Skills"));
        assert!(s.contains("No skills installed"));
    }

    #[test]
    fn plugins_renders_empty() {
        let s = plugins("a1", &[], Theme::Light);
        assert!(s.contains("Plugins"));
    }

    #[test]
    fn models_renders_with_config() {
        let c = AgentFileConfig {
            model: Some("gpt-4o-mini".into()),
            max_tokens: Some(2048),
            temperature: Some(0.5),
            max_tool_iterations: Some(8),
            workspace: Some("/work".into()),
            ..Default::default()
        };
        let s = models("a1", Some(&c), Theme::Light);
        assert!(s.contains("gpt-4o-mini"));
    }

    #[test]
    fn models_renders_empty() {
        let s = models("a1", None, Theme::Light);
        assert!(s.contains("No agent config available"));
    }

    #[test]
    fn context_renders() {
        let s = context("a1", Theme::Light);
        assert!(s.contains("Context"));
    }

    #[test]
    fn customize_renders_form() {
        let s = customize("a1", Theme::Light);
        assert!(s.contains("Customize"));
        assert!(s.contains(r#"name="soul""#));
        assert!(s.contains(r#"name="promptMode""#));
    }

    #[test]
    fn project_renders_empty() {
        let s = project("a1", &[], Theme::Light);
        assert!(s.contains("Project"));
        assert!(s.contains("No projects yet"));
    }

    #[test]
    fn project_id_renders() {
        let s = project_id("a1", "p1", Theme::Light);
        assert!(s.contains("p1"));
    }

    #[test]
    fn usage_renders_empty() {
        let s = usage("a1", None, Theme::Light);
        assert!(s.contains("No usage data in this range"));
    }

    #[test]
    fn usage_renders_with_data() {
        let u = AgentTokenUsage {
            range: TokenUsageRange::D7,
            agent_id: "a1".into(),
            sessions: vec![TokenUsageRank {
                key: "s1".into(),
                tokens: 500,
                input_tokens: 300,
                output_tokens: 200,
                request_count: 2,
            }],
        };
        let s = usage("a1", Some(&u), Theme::Light);
        assert!(s.contains("s1"));
        assert!(s.contains("500"));
    }
}
