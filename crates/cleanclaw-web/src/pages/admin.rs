//! Admin pages.
//! usage, chats}/page.tsx`. Each page is a server-rendered table
//! driven by a typed API call; the data path lives in the W4 server
//! wiring.
//!
//! In W4 the tables render empty placeholder rows when no data is
//! available; the W4 server handler fills them in with the real
//! `cleanclaw-store` results.

use crate::html::{
    badge, card_close, card_content, card_header, card_open, card_title, esc, tabs, Theme,
};
use crate::layout::{render, NavKey};
use crate::types::TokenUsageReport;

/// Admin sub-tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminTab {
    Users,
    Usage,
    Chats,
}

impl AdminTab {
    pub fn href(self) -> &'static str {
        match self {
            AdminTab::Users => "/admin/users",
            AdminTab::Usage => "/admin/usage",
            AdminTab::Chats => "/admin/chats",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AdminTab::Users => "Users",
            AdminTab::Usage => "Usage",
            AdminTab::Chats => "Chats",
        }
    }
}

pub fn admin_tabs(active: AdminTab) -> String {
    let labels: [(&str, &str); 3] = [("users", "Users"), ("usage", "Usage"), ("chats", "Chats")];
    tabs(&labels, active.label())
}

fn shell(active: AdminTab, body: &str, theme: Theme) -> String {
    let inner = format!(
        r#"<div class="space-y-4">
<h1 class="text-2xl font-semibold tracking-tight">Admin</h1>
{tabs}
{body}
</div>"#,
        tabs = admin_tabs(active),
        body = body,
    );
    render(
        "Admin · CleanClaw",
        NavKey::AdminUsers,
        &inner,
        Some(("Ada", "admin")),
        theme,
    )
}

/// `/admin/users` — list + create / delete.
pub fn users(theme: Theme, rows: &[UserRow]) -> String {
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for r in rows {
        table_rows.push(vec![
            r.id.clone(),
            r.username.clone(),
            r.email.clone(),
            r.role.clone(),
            r.status.clone(),
        ]);
    }
    let table = crate::html::table(&["ID", "Username", "Email", "Role", "Status"], &table_rows);
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
        card_title = card_title("Users"),
        card_content = card_content(""),
        create_btn = crate::html::button(
            "Create user",
            crate::html::ButtonVariant::Default,
            crate::html::ButtonSize::Sm,
            Some("/admin/users/new")
        ),
        table = table,
        card_close = card_close(),
    );
    shell(AdminTab::Users, &body, theme)
}

/// `/admin/usage` — token usage summary.
pub fn usage(theme: Theme, report: Option<&TokenUsageReport>) -> String {
    let body = match report {
        Some(r) => usage_table(r),
        None => usage_empty(),
    };
    shell(AdminTab::Usage, &body, theme)
}

fn usage_table(r: &TokenUsageReport) -> String {
    let mut agents: Vec<Vec<String>> = Vec::new();
    for a in &r.top_agents {
        agents.push(vec![
            a.key.clone(),
            a.tokens.to_string(),
            a.request_count.to_string(),
        ]);
    }
    let mut users: Vec<Vec<String>> = Vec::new();
    for u in &r.top_users {
        users.push(vec![
            u.key.clone(),
            u.tokens.to_string(),
            u.request_count.to_string(),
        ]);
    }
    format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<div class="grid grid-cols-2 gap-4">
<div>
<h3 class="text-sm font-medium mb-2">Top agents</h3>
{t1}
</div>
<div>
<h3 class="text-sm font-medium mb-2">Top users</h3>
{t2}
</div>
</dl>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title(format!("Usage · last {range}", range = r.range.as_str()).as_str()),
        card_content = card_content(""),
        t1 = crate::html::table(&["Agent", "Tokens", "Requests"], &agents),
        t2 = crate::html::table(&["User", "Tokens", "Requests"], &users),
        card_close = card_close(),
    )
}

fn usage_empty() -> String {
    format!(
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
    )
}

/// `/admin/chats` — cross-tenant chat listing.
pub fn chats(theme: Theme, rows: &[ChatRow]) -> String {
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for r in rows {
        table_rows.push(vec![
            r.id.clone(),
            r.agent_id.clone(),
            r.agent_name.clone().unwrap_or_default(),
            r.owner_username.clone().unwrap_or_default(),
            r.preview.clone(),
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
        table = crate::html::table(
            &["ID", "Agent", "Agent name", "Owner", "Preview"],
            &table_rows
        ),
        card_close = card_close(),
    );
    shell(AdminTab::Chats, &body, theme)
}

/// Row shape for the users table. Mirrors the column set the
/// React `adminListUsers` page renders.
#[derive(Debug, Clone, Default)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub status: String,
}

/// Row shape for the admin chats table. Mirrors
/// .
#[derive(Debug, Clone, Default)]
pub struct ChatRow {
    pub id: String,
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub owner_username: Option<String>,
    pub preview: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TokenUsageTotals;

    #[test]
    fn admin_tabs_marks_active() {
        let s = admin_tabs(AdminTab::Usage);
        assert!(s.contains("Usage"));
    }

    #[test]
    fn admin_tab_hrefs() {
        assert_eq!(AdminTab::Users.href(), "/admin/users");
        assert_eq!(AdminTab::Usage.href(), "/admin/usage");
        assert_eq!(AdminTab::Chats.href(), "/admin/chats");
    }

    #[test]
    fn users_renders_empty() {
        let s = users(Theme::Light, &[]);
        assert!(s.contains("Users"));
        assert!(s.contains("Create user"));
    }

    #[test]
    fn users_renders_rows() {
        let rows = vec![UserRow {
            id: "u1".into(),
            username: "ada".into(),
            email: "ada@example.com".into(),
            role: "admin".into(),
            status: "active".into(),
        }];
        let s = users(Theme::Light, &rows);
        assert!(s.contains("ada@example.com"));
        assert!(s.contains("u1"));
    }

    #[test]
    fn usage_empty() {
        let s = usage(Theme::Light, None);
        assert!(s.contains("No usage data"));
    }

    #[test]
    fn usage_with_report() {
        let r = TokenUsageReport {
            range: crate::types::TokenUsageRange::D7,
            totals: TokenUsageTotals {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                request_count: 5,
            },
            top_agents: vec![crate::types::TokenUsageRank {
                key: "agent_1".into(),
                tokens: 100,
                input_tokens: 80,
                output_tokens: 20,
                request_count: 3,
            }],
            top_users: vec![],
        };
        let s = usage(Theme::Light, Some(&r));
        assert!(s.contains("Top agents"));
        assert!(s.contains("agent_1"));
    }

    #[test]
    fn chats_renders_empty() {
        let s = chats(Theme::Light, &[]);
        assert!(s.contains("Chats"));
    }
}
