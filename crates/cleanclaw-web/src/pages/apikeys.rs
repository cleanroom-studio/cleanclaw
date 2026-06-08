//! API keys page. Mirrors
//! . Lists the caller's
//! apikeys and exposes create / delete / rotate / set-agents actions.

use crate::html::{card_open, card_close, card_header, card_title, card_content, esc, badge, BadgeVariant, Theme};
use crate::layout::{render, NavKey};
use crate::types::APIKey;

/// Render the apikeys page.
pub fn apikeys(theme: Theme, keys: &[APIKey]) -> String {
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
        card_title = card_title("API keys"),
        card_content = card_content(""),
        create_btn = crate::html::button("New key", crate::html::ButtonVariant::Default, crate::html::ButtonSize::Sm, Some("/apikeys/new")),
        table = render_table(keys),
        card_close = card_close(),
    );
    render("API keys · CleanClaw", NavKey::ApiKeys, &body, Some(("Ada", "user")), theme)
}

fn render_table(keys: &[APIKey]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for k in keys {
        rows.push(vec![
            esc(&k.id),
            esc(&k.name),
            esc(&k.key),
            esc(&k.created_at),
            badge("active", BadgeVariant::Default),
        ]);
    }
    crate::html::table(&["ID", "Name", "Key (masked)", "Created", "Status"], &rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apikeys_empty() {
        let s = apikeys(Theme::Light, &[]);
        assert!(s.contains("API keys"));
        assert!(s.contains("New key"));
    }

    #[test]
    fn apikeys_with_rows() {
        let keys = vec![APIKey {
            id: "k1".into(),
            name: "ci".into(),
            key: "fc_****wxyz".into(),
            created_at: "2026-01-01".into(),
        }];
        let s = apikeys(Theme::Light, &keys);
        assert!(s.contains("fc_****wxyz"));
        assert!(s.contains("k1"));
    }
}
