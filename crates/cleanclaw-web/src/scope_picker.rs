//! Server-side scope picker. Mirrors
//! . The picker
//! is a `<select>` rendered server-side; the W4–W6 server handlers
//! can include it in any page that needs to switch between
//! `system`, `user`, and `agent` scopes.

use crate::html::Theme;
use crate::types::ScopeName;

/// Render a scope picker. `active` is the currently selected scope;
/// `name` is the form field name; `id` is the HTML id for the
/// `<label>`.
pub fn render(active: ScopeName, name: &str, id: &str) -> String {
    let opts = [
        (ScopeName::System, "System"),
        (ScopeName::User, "User"),
        (ScopeName::Agent, "Agent"),
    ];
    let mut out = String::new();
    out.push_str(&format!(
        r#"<label for="{id}" class="text-sm font-medium">Scope</label>"#
    ));
    out.push_str(&format!(
        r#"<select id="{id}" name="{name}" class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm">"#
    ));
    for (s, label) in opts {
        let sel = if s == active { " selected" } else { "" };
        out.push_str(&format!(
            r#"<option value="{val}"{sel}>{label}</option>"#,
            val = s.as_str(),
            sel = sel,
            label = label,
        ));
    }
    out.push_str("</select>");
    out
}

/// Render a scope picker for the API client (returns the raw HTML
/// the same way `render` does, but lets the caller choose the
/// `Theme` so the surrounding page is consistent).
pub fn themed(active: ScopeName, name: &str, id: &str, _theme: Theme) -> String {
    render(active, name, id)
}

/// Parse a `?scope=...` query value into a `ScopeName`. Returns
/// `System` (the default) for unknown values.
pub fn from_query(s: Option<&str>) -> ScopeName {
    match s {
        Some("user") => ScopeName::User,
        Some("agent") => ScopeName::Agent,
        _ => ScopeName::System,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_all_three() {
        let s = render(ScopeName::System, "scope", "scope-select");
        assert!(s.contains("System"));
        assert!(s.contains("User"));
        assert!(s.contains("Agent"));
    }

    #[test]
    fn render_marks_active() {
        let s = render(ScopeName::Agent, "scope", "scope-select");
        assert!(s.contains(r#"value="agent" selected"#));
    }

    #[test]
    fn from_query_parses() {
        assert_eq!(from_query(Some("system")), ScopeName::System);
        assert_eq!(from_query(Some("user")), ScopeName::User);
        assert_eq!(from_query(Some("agent")), ScopeName::Agent);
        assert_eq!(from_query(Some("nonsense")), ScopeName::System);
        assert_eq!(from_query(None), ScopeName::System);
    }
}
