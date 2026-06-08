//! Layout shell. ,
//! `components/app-shell.tsx`, and `components/app-sidebar.tsx`. Renders
//! the global `<html>` frame with header, sidebar, theme toggle, and
//! content slot.

use crate::css::BASE_CSS;
use crate::html::{cn, esc, page as render_page, Theme};

/// Active route key. Drives the sidebar's "active" highlight. Mirrors
/// the `usePathname()` checks in .
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Overview,
    Agents,
    Chat,
    Channels,
    ChannelsConfig,
    Cron,
    Models,
    Plugins,
    Providers,
    Skills,
    Tools,
    Settings,
    AdminUsers,
    AdminUsage,
    AdminChats,
    ApiKeys,
}

impl NavKey {
    pub fn href(self) -> &'static str {
        match self {
            NavKey::Overview => "/overview",
            NavKey::Agents => "/agents",
            NavKey::Chat => "/chat",
            NavKey::Channels => "/channels",
            NavKey::ChannelsConfig => "/channels-config",
            NavKey::Cron => "/cron",
            NavKey::Models => "/models",
            NavKey::Plugins => "/plugins",
            NavKey::Providers => "/providers",
            NavKey::Skills => "/skills",
            NavKey::Tools => "/tools",
            NavKey::Settings => "/settings",
            NavKey::AdminUsers => "/admin/users",
            NavKey::AdminUsage => "/admin/usage",
            NavKey::AdminChats => "/admin/chats",
            NavKey::ApiKeys => "/apikeys",
        }
    }
}

/// Nav item descriptor.
pub struct NavItem {
    pub key: NavKey,
    pub label: &'static str,
    pub icon: &'static str,
}

/// The 15 nav items that live in the sidebar. Mirrors
/// .
pub const NAV_ITEMS: &[NavItem] = &[
    NavItem {
        key: NavKey::Overview,
        label: "Overview",
        icon: "■",
    },
    NavItem {
        key: NavKey::Chat,
        label: "Chat",
        icon: "✦",
    },
    NavItem {
        key: NavKey::Agents,
        label: "Agents",
        icon: "✚",
    },
    NavItem {
        key: NavKey::Channels,
        label: "Channels",
        icon: "✉",
    },
    NavItem {
        key: NavKey::ChannelsConfig,
        label: "Channel config",
        icon: "⚙",
    },
    NavItem {
        key: NavKey::Cron,
        label: "Scheduler",
        icon: "⏱",
    },
    NavItem {
        key: NavKey::Models,
        label: "Models",
        icon: "◊",
    },
    NavItem {
        key: NavKey::Providers,
        label: "Providers",
        icon: "↯",
    },
    NavItem {
        key: NavKey::Skills,
        label: "Skills",
        icon: "★",
    },
    NavItem {
        key: NavKey::Tools,
        label: "Tools",
        icon: "✤",
    },
    NavItem {
        key: NavKey::Plugins,
        label: "Plugins",
        icon: "▣",
    },
    NavItem {
        key: NavKey::ApiKeys,
        label: "API keys",
        icon: "⚷",
    },
    NavItem {
        key: NavKey::Settings,
        label: "Settings",
        icon: "✦",
    },
];

/// Render the sidebar nav. The active item gets a stronger background
/// + foreground. Mirrors `app-sidebar.tsx`.
pub fn sidebar(active: NavKey) -> String {
    let mut out = String::new();
    out.push_str(r#"<aside class="hidden md:flex md:w-64 flex-col border-r bg-sidebar text-sidebar-foreground">"#);
    out.push_str(r#"<div class="flex h-14 items-center border-b px-4">"#);
    out.push_str(r#"<a class="flex items-center gap-2 font-semibold" href="/">"#);
    out.push_str(r#"<span class="inline-block size-6 rounded-md bg-primary"></span>"#);
    out.push_str(r#"<span>CleanClaw</span></a></div>"#);
    out.push_str(r#"<nav class="flex-1 overflow-auto p-2">"#);
    for item in NAV_ITEMS {
        let is_active = item.key == active;
        let cls = cn([
            "flex items-center gap-3 rounded-md px-3 py-2 text-sm transition-colors",
            if is_active {
                "bg-sidebar-accent text-sidebar-accent-foreground font-medium"
            } else {
                "hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            },
        ]);
        out.push_str(&format!(
            r#"<a class="{cls}" href="{}"><span class="w-4 text-center text-base">{icon}</span><span>{label}</span></a>"#,
            item.key.href(),
            cls = cls,
            icon = item.icon,
            label = esc(item.label),
        ));
    }
    out.push_str("</nav></aside>");
    out
}

/// Render the top bar. Contains the agent-switcher slot, the theme
/// toggle, and the user menu. Mirrors `nav-main.tsx`, `nav-user.tsx`,
/// `team-switcher.tsx`.
pub fn topbar(user_display: Option<&str>, user_role: Option<&str>) -> String {
    let display = user_display.unwrap_or("Guest");
    let role = user_role.unwrap_or("");
    let cls = "flex h-14 items-center justify-between border-b bg-background px-4";
    format!(
        r#"<header class="{cls}"><div class="flex items-center gap-2"><a class="font-semibold" href="/overview">CleanClaw</a></div><div class="flex items-center gap-2"><a class="text-sm text-muted-foreground hover:text-foreground" href="/?theme=dark" title="Switch theme">◐</a><div class="flex items-center gap-2"><span class="text-sm">{display}</span>{role_badge}</div></div></header>"#,
        cls = cls,
        display = esc(display),
        role_badge = if role.is_empty() {
            String::new()
        } else {
            format!(
                r#"<span class="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">{}</span>"#,
                esc(role)
            )
        },
    )
}

/// Render the full app shell. Returns the body fragment that goes
/// inside the `<html>` envelope.
pub fn app_shell(active: NavKey, body: &str, user: Option<(&str, &str)>) -> String {
    let user_part = user
        .map(|(d, r)| topbar(Some(d), Some(r)))
        .unwrap_or_else(|| topbar(None, None));
    format!(
        r#"<div class="flex min-h-screen">{sidebar}<div class="flex-1 flex flex-col">{topbar}<main class="flex-1 p-6">{body}</main></div></div>"#,
        sidebar = sidebar(active),
        topbar = user_part,
        body = body,
    )
}

/// Render a complete page (html envelope + shell + body).
pub fn render(
    title: &str,
    active: NavKey,
    body: &str,
    user: Option<(&str, &str)>,
    theme: Theme,
) -> String {
    let body = app_shell(active, body, user);
    render_page(title, &body, BASE_CSS, theme)
}

/// Render a centered card layout used by the auth flow (no sidebar,
/// no topbar). Mirrors `auth-guard.tsx` + `login-screen.tsx`.
pub fn auth_shell(title: &str, body: &str, theme: Theme) -> String {
    let wrap = format!(
        r#"<div class="min-h-screen flex items-center justify-center bg-muted p-6"><div class="w-full max-w-md">{body}</div></div>"#,
        body = body,
    );
    render_page(title, &wrap, BASE_CSS, theme)
}

/// Tiny avatar helper used in the user menu (avoids pulling
/// `html::avatar` into the layout module).
pub fn user_avatar(display: &str) -> String {
    let initials: String = display
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    crate::html::avatar(&initials, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_key_href_is_stable() {
        assert_eq!(NavKey::Overview.href(), "/overview");
        assert_eq!(NavKey::ApiKeys.href(), "/apikeys");
    }

    #[test]
    fn sidebar_marks_active_route() {
        let s = sidebar(NavKey::Agents);
        assert!(s.contains(r#"href="/agents""#));
        assert!(s.contains(r#"href="/overview""#));
        // Active item carries the "bg-sidebar-accent" class.
        let active_idx = s.find(r#"href="/agents""#).unwrap();
        let prefix = &s[..active_idx];
        assert!(prefix.rfind("bg-sidebar-accent").unwrap() > prefix.rfind("</a>").unwrap_or(0));
    }

    #[test]
    fn topbar_handles_anonymous() {
        let s = topbar(None, None);
        assert!(s.contains("Guest"));
    }

    #[test]
    fn topbar_renders_role_badge() {
        let s = topbar(Some("Ada"), Some("admin"));
        assert!(s.contains("Ada"));
        assert!(s.contains("admin"));
    }

    #[test]
    fn app_shell_has_sidebar_and_topbar() {
        let s = app_shell(NavKey::Overview, "<p>hi</p>", Some(("Ada", "user")));
        assert!(s.contains("CleanClaw"));
        assert!(s.contains("Ada"));
        assert!(s.contains("<p>hi</p>"));
    }

    #[test]
    fn render_wraps_full_doc() {
        let s = render(
            "Test",
            NavKey::Agents,
            "<p>x</p>",
            Some(("Ada", "user")),
            Theme::Light,
        );
        assert!(s.starts_with("<!DOCTYPE"));
        assert!(s.contains("<title>Test</title>"));
    }

    #[test]
    fn auth_shell_omits_sidebar() {
        let s = auth_shell("Sign in", "<form>...</form>", Theme::Dark);
        assert!(s.starts_with("<!DOCTYPE"));
        assert!(!s.contains(r#"href="/overview""#));
        assert!(s.contains(r#"class="dark""#));
    }
}
