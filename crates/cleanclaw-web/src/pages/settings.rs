//! Settings pages.
//! account, general, runtime}/page.tsx` plus the `settings/layout.tsx`
//! tab strip.
//!
//! Each page is a server-rendered form. The forms `POST` back to
//! themselves; the server-side handler (W4 wiring) is responsible
//! for persisting via `cleanclaw-config` / `cleanclaw-auth`.

use crate::html::{card_open, card_close, card_header, card_title, card_content, esc, tabs, Theme};
use crate::layout::{render, NavKey};

/// Settings sub-tab key. Drives the active highlight on the tab strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Account,
    Runtime,
    About,
}

impl SettingsTab {
    pub fn href(self) -> &'static str {
        match self {
            SettingsTab::General => "/settings/general",
            SettingsTab::Account => "/settings/account",
            SettingsTab::Runtime => "/settings/runtime",
            SettingsTab::About => "/settings/about",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Account => "Account",
            SettingsTab::Runtime => "Runtime",
            SettingsTab::About => "About",
        }
    }
}

/// Render the settings sub-tab strip. Each entry is an `<a>`; the
/// active one gets the underline + bold treatment.
pub fn settings_tabs(active: SettingsTab) -> String {
    let labels: [(&str, &str); 4] = [
        ("general", "General"),
        ("account", "Account"),
        ("runtime", "Runtime"),
        ("about", "About"),
    ];
    tabs(&labels, active.label())
}

/// Common shell for every settings sub-page.
pub fn shell(active: SettingsTab, body: &str, theme: Theme) -> String {
    let inner = format!(
        r#"<div class="space-y-4">
<h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
{tabs}
{body}
</div>"#,
        tabs = settings_tabs(active),
        body = body,
    );
    render("Settings · CleanClaw", NavKey::Settings, &inner, Some(("Ada", "user")), theme)
}

/// `/settings/general` — top-level config (provider, channels,
/// hooks, cron jobs at the system scope).
pub fn general(theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<form method="POST" action="/settings/general" class="space-y-4">
<label class="block">
<span class="text-sm font-medium">Provider</span>
<select class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" name="provider">
<option value="openai">OpenAI</option>
<option value="anthropic">Anthropic</option>
<option value="google">Google</option>
</select>
</label>
<label class="block">
<span class="text-sm font-medium">API base URL</span>
<input class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" type="url" name="apiBase" value="https://api.openai.com/v1" />
</label>
<label class="block">
<span class="text-sm font-medium">API key</span>
<input class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" type="password" name="apiKey" />
</label>
<button class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground">Save</button>
</form>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("General"),
        card_content = card_content(""),
        card_close = card_close(),
    );
    shell(SettingsTab::General, &body, theme)
}

/// `/settings/account` — display name, avatar, password change.
pub fn account(theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<form method="POST" action="/settings/account" class="space-y-4">
<label class="block">
<span class="text-sm font-medium">Display name</span>
<input class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" type="text" name="displayName" value="Ada" />
</label>
<label class="block">
<span class="text-sm font-medium">Avatar URL</span>
<input class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" type="url" name="avatarUrl" value="" />
</label>
<button class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground">Save</button>
</form>
</div>
{card_close}

{card2_open}
{card2_header}
{card2_title}
{card2_content}
<form method="POST" action="/settings/account/password" class="space-y-4">
<label class="block">
<span class="text-sm font-medium">Current password</span>
<input class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" type="password" name="oldPassword" />
</label>
<label class="block">
<span class="text-sm font-medium">New password</span>
<input class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" type="password" name="newPassword" />
</label>
<button class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground">Change password</button>
</form>
</div>
{card2_close}"#,
        card_open = card_open(""),
        card2_open = card_open("mt-4"),
        card_header = card_header(),
        card2_header = card_header(),
        card_title = card_title("Account"),
        card2_title = card_title("Change password"),
        card_content = card_content(""),
        card2_content = card_content(""),
        card_close = card_close(),
        card2_close = card_close(),
    );
    shell(SettingsTab::Account, &body, theme)
}

/// `/settings/runtime` — sandbox, storage, hooks, etc.
pub fn runtime(theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<form method="POST" action="/settings/runtime" class="space-y-4">
<label class="flex items-center gap-2">
<input type="checkbox" name="sandboxEnabled" />
<span class="text-sm">Enable sandbox</span>
</label>
<label class="block">
<span class="text-sm font-medium">Backend</span>
<select class="mt-1 w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm" name="sandboxBackend">
<option value="local">local</option>
<option value="docker">docker</option>
<option value="e2b">e2b</option>
<option value="boxlite">boxlite</option>
</select>
</label>
<button class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground">Save</button>
</form>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("Runtime"),
        card_content = card_content(""),
        card_close = card_close(),
    );
    shell(SettingsTab::Runtime, &body, theme)
}

/// `/settings/about` — version + build info.
pub fn about(theme: Theme) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_content}
<dl class="space-y-1 text-sm">
<dt class="font-medium">Version</dt><dd class="text-muted-foreground">0.1.0</dd>
<dt class="font-medium">Mode</dt><dd class="text-muted-foreground">self-hosted</dd>
</dl>
</div>
{card_close}"#,
        card_open = card_open(""),
        card_header = card_header(),
        card_title = card_title("About"),
        card_content = card_content(""),
        card_close = card_close(),
    );
    shell(SettingsTab::About, &body, theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_tabs_marks_active() {
        let s = settings_tabs(SettingsTab::Runtime);
        assert!(s.contains("General"));
        assert!(s.contains("Runtime"));
    }

    #[test]
    fn settings_tab_hrefs() {
        assert_eq!(SettingsTab::General.href(), "/settings/general");
        assert_eq!(SettingsTab::Account.href(), "/settings/account");
        assert_eq!(SettingsTab::Runtime.href(), "/settings/runtime");
        assert_eq!(SettingsTab::About.href(), "/settings/about");
    }

    #[test]
    fn general_renders_form() {
        let s = general(Theme::Light);
        assert!(s.contains("Provider"));
        assert!(s.contains(r#"action="/settings/general""#));
    }

    #[test]
    fn account_renders_password_form() {
        let s = account(Theme::Light);
        assert!(s.contains("Display name"));
        assert!(s.contains("New password"));
    }

    #[test]
    fn runtime_renders_sandbox_toggle() {
        let s = runtime(Theme::Light);
        assert!(s.contains("Enable sandbox"));
        assert!(s.contains("docker"));
    }

    #[test]
    fn about_renders_version() {
        let s = about(Theme::Light);
        assert!(s.contains("Version"));
    }

    #[test]
    fn shell_uses_app_layout() {
        let s = general(Theme::Light);
        assert!(s.starts_with("<!DOCTYPE"));
        assert!(s.contains("CleanClaw"));
    }
}
