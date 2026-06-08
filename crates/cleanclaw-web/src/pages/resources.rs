//! Resources + onboard pages. Mirrors
//!  channels-config, models,
//! providers, plugins, skills, tools, cron, onboard}/page.tsx`.
//!
//! Each page is a server-rendered card layout that lists the
//! corresponding resources and exposes create / delete actions.

use crate::html::{card_open, card_close, card_header, card_title, card_content, card_description, esc, Theme};
use crate::layout::{render, NavKey};
use crate::types::*;

// =====================================================================
// `/channels` — channel health dashboard
// =====================================================================

pub fn channels_list(theme: Theme, channels: &[ChannelInfo]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in channels {
        rows.push(vec![
            esc(&c.kind),
            esc(&c.bot_username),
            c.enabled.map(|b| if b { "yes" } else { "no" }.to_string()).unwrap_or_default(),
            esc(c.status.as_deref().unwrap_or("")),
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
        card_title = card_title("Channels"),
        card_content = card_content(""),
        create_btn = crate::html::button("Add channel", crate::html::ButtonVariant::Default, crate::html::ButtonSize::Sm, Some("/channels-config")),
        table = crate::html::table(&["Type", "Bot username", "Enabled", "Status"], &rows),
        card_close = card_close(),
    );
    render("Channels · CleanClaw", NavKey::Channels, &body, Some(("Ada", "admin")), theme)
}

// =====================================================================
// `/channels-config` — per-scope channel configuration
// =====================================================================

pub fn channels_config(theme: Theme, channels: &[ChannelRow]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in channels {
        rows.push(vec![
            esc(&c.id),
            esc(c.scope.as_str()),
            esc(&c.scope_id),
            esc(&c.kind),
            if c.enabled { "yes" } else { "no" }.to_string(),
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
        card_title = card_title("Channel configuration"),
        card_content = card_content(""),
        create_btn = crate::html::button("New channel", crate::html::ButtonVariant::Default, crate::html::ButtonSize::Sm, Some("/channels-config/new")),
        table = crate::html::table(&["ID", "Scope", "Scope ID", "Type", "Enabled"], &rows),
        card_close = card_close(),
    );
    render("Channel config · CleanClaw", NavKey::ChannelsConfig, &body, Some(("Ada", "admin")), theme)
}

// =====================================================================
// `/models` — model catalog
// =====================================================================

pub fn models(theme: Theme, models: &[ModelEntry]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for m in models {
        rows.push(vec![
            esc(&m.id),
            esc(&m.name),
            if m.reasoning { "yes" } else { "no" }.to_string(),
            m.context_window.to_string(),
            m.max_tokens.to_string(),
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
<th class="h-10 px-2 text-left">Reasoning</th>
<th class="h-10 px-2 text-left">Context window</th>
<th class="h-10 px-2 text-left">Max tokens</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Models"),
        card_content = card_content(""),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="5" class="p-2 text-muted-foreground text-sm">No models registered.</td></tr>"#.to_string()
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
    render("Models · CleanClaw", NavKey::Models, &body, Some(("Ada", "user")), theme)
}

// =====================================================================
// `/providers` — provider CRUD
// =====================================================================

pub fn providers(theme: Theme, providers: &[ProviderRow]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for p in providers {
        rows.push(vec![
            esc(&p.id),
            esc(p.scope.as_str()),
            esc(&p.scope_id),
            esc(&p.name),
            esc(p.api_base.as_deref().unwrap_or("")),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{create_btn}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">ID</th>
<th class="h-10 px-2 text-left">Scope</th>
<th class="h-10 px-2 text-left">Scope ID</th>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">API base</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Providers"),
        card_content = card_content(""),
        create_btn = crate::html::button("New provider", crate::html::ButtonVariant::Default, crate::html::ButtonSize::Sm, Some("/providers/new")),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="5" class="p-2 text-muted-foreground text-sm">No providers configured.</td></tr>"#.to_string()
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
    render("Providers · CleanClaw", NavKey::Providers, &body, Some(("Ada", "admin")), theme)
}

// =====================================================================
// `/plugins` — plugin registry
// =====================================================================

pub fn plugins(theme: Theme, plugins: &[PluginInfo]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for p in plugins {
        rows.push(vec![
            esc(&p.id),
            esc(&p.kind),
            esc(&p.version),
            esc(&p.status),
            if p.enabled { "yes" } else { "no" }.to_string(),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">ID</th>
<th class="h-10 px-2 text-left">Type</th>
<th class="h-10 px-2 text-left">Version</th>
<th class="h-10 px-2 text-left">Status</th>
<th class="h-10 px-2 text-left">Enabled</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Plugins"),
        card_content = card_content(""),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="5" class="p-2 text-muted-foreground text-sm">No plugins installed.</td></tr>"#.to_string()
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
    render("Plugins · CleanClaw", NavKey::Plugins, &body, Some(("Ada", "admin")), theme)
}

// =====================================================================
// `/skills` — skill catalog
// =====================================================================

pub fn skills(theme: Theme, skills: &[SkillInfo]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for s in skills {
        rows.push(vec![
            esc(&s.name),
            esc(&s.description),
            esc(&s.location),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{install_btn}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">Description</th>
<th class="h-10 px-2 text-left">Location</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Skills"),
        card_content = card_content(""),
        install_btn = crate::html::button("Install skill", crate::html::ButtonVariant::Default, crate::html::ButtonSize::Sm, Some("/skills/install")),
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
    render("Skills · CleanClaw", NavKey::Skills, &body, Some(("Ada", "user")), theme)
}

// =====================================================================
// `/tools` — tool provider catalog
// =====================================================================

pub fn tools(theme: Theme, cfg: Option<&ToolsConfig>) -> String {
    let body = match cfg {
        Some(c) => {
            let mut rows: Vec<Vec<String>> = Vec::new();
            for cat in &c.categories {
                for prov in &cat.providers {
                    rows.push(vec![
                        esc(&cat.label),
                        esc(&prov.label),
                        esc(&prov.name),
                        if prov.needs_key { "yes" } else { "no" }.to_string(),
                        if prov.needs_url { "yes" } else { "no" }.to_string(),
                    ]);
                }
            }
            format!(
                r#"{card_open}
{card_header}
{card_title}
{card_content}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">Category</th>
<th class="h-10 px-2 text-left">Provider</th>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">Needs key</th>
<th class="h-10 px-2 text-left">Needs URL</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
                card_open = card_open(""),
                card_header = card_header(),
                card_title = card_title("Tools"),
                card_content = card_content(""),
                rows_html = if rows.is_empty() {
                    r#"<tr><td colspan="5" class="p-2 text-muted-foreground text-sm">No tool providers registered.</td></tr>"#.to_string()
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
<p class="text-sm text-muted-foreground">No tools configured.</p>
</div>
{card_close}"#,
            card_open = card_open(""),
            card_header = card_header(),
            card_title = card_title("Tools"),
            card_content = card_content(""),
            card_close = card_close(),
        ),
    };
    render("Tools · CleanClaw", NavKey::Tools, &body, Some(("Ada", "user")), theme)
}

// =====================================================================
// `/cron` — global scheduler
// =====================================================================

pub fn cron(theme: Theme, jobs: &[CronJobInfo]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for j in jobs {
        rows.push(vec![
            esc(&j.name),
            esc(&j.kind),
            esc(&j.schedule),
            esc(&j.agent_id),
            if j.enabled { "yes" } else { "no" }.to_string(),
        ]);
    }
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
{create_btn}
<table class="w-full caption-bottom text-sm"><thead><tr>
<th class="h-10 px-2 text-left">Name</th>
<th class="h-10 px-2 text-left">Type</th>
<th class="h-10 px-2 text-left">Schedule</th>
<th class="h-10 px-2 text-left">Agent</th>
<th class="h-10 px-2 text-left">Enabled</th>
</tr></thead><tbody>{rows_html}</tbody></table>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Scheduler"),
        card_content = card_content(""),
        create_btn = crate::html::button("New job", crate::html::ButtonVariant::Default, crate::html::ButtonSize::Sm, Some("/cron/new")),
        rows_html = if rows.is_empty() {
            r#"<tr><td colspan="5" class="p-2 text-muted-foreground text-sm">No cron jobs.</td></tr>"#.to_string()
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
    render("Scheduler · CleanClaw", NavKey::Cron, &body, Some(("Ada", "user")), theme)
}

// =====================================================================
// `/onboard` — first-run wizard
// =====================================================================

pub fn onboard(theme: Theme, error: Option<&str>) -> String {
    use crate::layout::auth_shell;
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{desc}
{error_alert}
<form method="POST" action="/onboard" class="space-y-4 mt-4">
<fieldset class="space-y-2">
<legend class="text-sm font-medium">Account</legend>
{label_user}
{input_user}
{label_email}
{input_email}
{label_pass}
{input_pass}
</fieldset>
<fieldset class="space-y-2">
<legend class="text-sm font-medium">Provider</legend>
<select class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" name="provider">
<option value="openai">OpenAI</option>
<option value="anthropic">Anthropic</option>
</select>
{label_base}
{input_base}
{label_key}
{input_key}
{label_model}
{input_model}
</fieldset>
<button class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground">Finish setup</button>
</form>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Welcome to CleanClaw"),
        desc = card_description("Set up the admin account and connect a provider."),
        error_alert = match error {
            Some(msg) => crate::html::alert("Setup failed", msg, crate::html::AlertVariant::Destructive),
            None => String::new(),
        },
        label_user = crate::html::label("Username", "username"),
        input_user = crate::html::input("username", "ada", "", "text"),
        label_email = crate::html::label("Email", "email"),
        input_email = crate::html::input("email", "you@example.com", "", "email"),
        label_pass = crate::html::label("Password", "password"),
        input_pass = crate::html::html_password("password", "Password (min 8 chars)", ""),
        label_base = crate::html::label("API base URL", "apiBase"),
        input_base = crate::html::input("apiBase", "https://api.openai.com/v1", "https://api.openai.com/v1", "url"),
        label_key = crate::html::label("API key", "apiKey"),
        input_key = crate::html::html_password("apiKey", "API key", ""),
        label_model = crate::html::label("Default model", "model"),
        input_model = crate::html::input("model", "gpt-4o", "gpt-4o", "text"),
        card_close = card_close(),
    );
    auth_shell("Welcome · CleanClaw", &body, theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_list_empty() {
        let s = channels_list(Theme::Light, &[]);
        assert!(s.contains("Channels"));
        assert!(s.contains("Add channel"));
    }

    #[test]
    fn channels_list_with_rows() {
        let chans = vec![ChannelInfo {
            kind: "telegram".into(),
            bot_username: "mybot".into(),
            enabled: Some(true),
            status: Some("ok".into()),
        }];
        let s = channels_list(Theme::Light, &chans);
        assert!(s.contains("mybot"));
    }

    #[test]
    fn channels_config_renders() {
        let rows = vec![ChannelRow {
            id: "c1".into(),
            scope: ScopeName::User,
            scope_id: "u1".into(),
            kind: "slack".into(),
            enabled: true,
            ..Default::default()
        }];
        let s = channels_config(Theme::Light, &rows);
        assert!(s.contains("Channel configuration"));
        assert!(s.contains("c1"));
    }

    #[test]
    fn models_renders() {
        let m = vec![ModelEntry {
            id: "gpt-4o".into(),
            name: "GPT-4o".into(),
            reasoning: false,
            input: vec!["text".into()],
            cost: ModelCost { input: 0.0, output: 0.0, cache_read: 0.0, cache_write: 0.0 },
            context_window: 128000,
            max_tokens: 16384,
        }];
        let s = models(Theme::Light, &m);
        assert!(s.contains("GPT-4o"));
        assert!(s.contains("gpt-4o"));
    }

    #[test]
    fn providers_renders() {
        let p = vec![ProviderRow {
            id: "p1".into(),
            scope: ScopeName::System,
            scope_id: "system".into(),
            name: "openai".into(),
            api_base: Some("https://api.openai.com/v1".into()),
            ..Default::default()
        }];
        let s = providers(Theme::Light, &p);
        assert!(s.contains("p1"));
    }

    #[test]
    fn plugins_renders() {
        let p = vec![PluginInfo {
            id: "pl1".into(),
            kind: "hook".into(),
            version: "0.1.0".into(),
            status: "running".into(),
            enabled: true,
            ..Default::default()
        }];
        let s = plugins(Theme::Light, &p);
        assert!(s.contains("pl1"));
    }

    #[test]
    fn skills_renders_empty() {
        let s = skills(Theme::Light, &[]);
        assert!(s.contains("Skills"));
        assert!(s.contains("No skills installed"));
    }

    #[test]
    fn tools_empty() {
        let s = tools(Theme::Light, None);
        assert!(s.contains("No tools configured"));
    }

    #[test]
    fn tools_with_config() {
        let cfg = ToolsConfig {
            categories: vec![ToolCategoryCatalog {
                name: "web_search".into(),
                label: "Web search".into(),
                providers: vec![ToolProviderCatalog {
                    name: "jina".into(),
                    label: "Jina".into(),
                    needs_key: true,
                    needs_url: false,
                    models: vec!["default".into()],
                }],
            }],
            tool_providers: std::collections::HashMap::new(),
            tools: std::collections::HashMap::new(),
        };
        let s = tools(Theme::Light, Some(&cfg));
        assert!(s.contains("Jina"));
    }

    #[test]
    fn cron_renders() {
        let j = vec![CronJobInfo {
            id: "j1".into(),
            name: "daily".into(),
            kind: "cron".into(),
            schedule: "0 0 * * *".into(),
            agent_id: "a1".into(),
            channel: "telegram".into(),
            chat_id: "123".into(),
            message: "ping".into(),
            enabled: true,
            ..Default::default()
        }];
        let s = cron(Theme::Light, &j);
        assert!(s.contains("daily"));
        assert!(s.contains("0 0 * * *"));
    }

    #[test]
    fn onboard_renders_form() {
        let s = onboard(Theme::Light, None);
        assert!(s.contains("Welcome to CleanClaw"));
        assert!(s.contains("API key"));
    }

    #[test]
    fn onboard_renders_error() {
        let s = onboard(Theme::Light, Some("bad input"));
        assert!(s.contains("bad input"));
    }
}
