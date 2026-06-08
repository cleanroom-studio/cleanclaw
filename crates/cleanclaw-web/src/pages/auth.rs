//! Auth pages. and
//! the `/api/login` / `/api/register` POSTs the React UI dispatches.
//! In the SSR build, both pages are server-rendered forms that
//! `POST` back to themselves and redirect on success.

use crate::html::{esc, Theme};
use crate::layout::auth_shell;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoginForm {
    pub login: String,
    pub password: String,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SignupForm {
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

/// Render the login form. Mirrors `login-screen.tsx` + the React
/// `login()` function in `lib/api.ts`.
pub fn login_page(theme: Theme, error: Option<&str>, prefill: Option<&str>) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_description}
{error_alert}
<form method="POST" action="/login" class="space-y-4 mt-4">
{label_user}
{input_user}
{label_pass}
{input_pass}
{submit}
<p class="text-sm text-muted-foreground mt-2">No account? <a class="text-primary underline" href="/signup">Create one</a></p>
</form>
{card_close}"#,
        card_open = crate::html::card_open(""),
        card_header = crate::html::card_header(),
        card_title = crate::html::card_title("Sign in"),
        card_description = crate::html::card_description("Use your username or email to sign in."),
        error_alert = match error {
            Some(msg) => crate::html::alert(
                "Sign-in failed",
                msg,
                crate::html::AlertVariant::Destructive
            ),
            None => String::new(),
        },
        label_user = crate::html::label("Username or email", "login"),
        input_user = crate::html::input("login", "you@example.com", prefill.unwrap_or(""), "text"),
        label_pass = crate::html::label("Password", "password"),
        input_pass = crate::html::html_password("password", "password", ""),
        submit = crate::html::button(
            "Sign in",
            crate::html::ButtonVariant::Default,
            crate::html::ButtonSize::Default,
            None
        ),
        card_close = crate::html::card_close(),
    );
    auth_shell("Sign in · CleanClaw", &body, theme)
}

/// Render the signup form. Mirrors `signup/page.tsx` + the React
/// `register()` function in `lib/api.ts`.
pub fn signup_page(
    theme: Theme,
    error: Option<&str>,
    prefill_username: Option<&str>,
    prefill_email: Option<&str>,
) -> String {
    let body = format!(
        r#"{card_open}
{card_header}
{card_title}
{card_description}
{error_alert}
<form method="POST" action="/signup" class="space-y-4 mt-4">
{label_user}
{input_user}
{label_email}
{input_email}
{label_pass}
{input_pass}
{submit}
<p class="text-sm text-muted-foreground mt-2">Already have an account? <a class="text-primary underline" href="/login">Sign in</a></p>
</form>
{card_close}"#,
        card_open = crate::html::card_open(""),
        card_header = crate::html::card_header(),
        card_title = crate::html::card_title("Create an account"),
        card_description =
            crate::html::card_description("Sign up to manage agents, channels, and skills."),
        error_alert = match error {
            Some(msg) =>
                crate::html::alert("Signup failed", msg, crate::html::AlertVariant::Destructive),
            None => String::new(),
        },
        label_user = crate::html::label("Username", "username"),
        input_user = crate::html::input("username", "ada", prefill_username.unwrap_or(""), "text"),
        label_email = crate::html::label("Email", "email"),
        input_email = crate::html::input(
            "email",
            "you@example.com",
            prefill_email.unwrap_or(""),
            "email"
        ),
        label_pass = crate::html::label("Password", "password"),
        input_pass = crate::html::html_password("password", "Password (min 8 chars)", ""),
        submit = crate::html::button(
            "Create account",
            crate::html::ButtonVariant::Default,
            crate::html::ButtonSize::Default,
            None
        ),
        card_close = crate::html::card_close(),
    );
    auth_shell("Sign up · CleanClaw", &body, theme)
}

/// Validate the login form. Returns the cleaned `(login, password)`
/// tuple. Empty `login` or `password` is rejected.
pub fn validate_login(form: &LoginForm) -> std::result::Result<(&str, &str), String> {
    if form.login.trim().is_empty() {
        return Err("Username or email required".into());
    }
    if form.password.is_empty() {
        return Err("Password required".into());
    }
    Ok((form.login.trim(), form.password.as_str()))
}

/// Validate the signup form. Mirrors the password / email rules the
/// Go server applies in `auth.Register`.
pub fn validate_signup(form: &SignupForm) -> std::result::Result<(), String> {
    if form.username.trim().len() < 3 {
        return Err("Username must be at least 3 characters".into());
    }
    if !form
        .username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Username may only contain letters, digits, _ and -".into());
    }
    if !form.email.contains('@') || form.email.len() < 5 {
        return Err("Invalid email address".into());
    }
    if form.password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }
    Ok(())
}

/// Escape a redirect target. Only allow same-origin paths starting
/// with `/`. Mirrors the helper in `app/api/login.go`.
pub fn safe_redirect(target: Option<&str>) -> &str {
    match target {
        Some(t) if t.starts_with('/') && !t.starts_with("//") => t,
        _ => "/overview",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_page_renders_form() {
        let s = login_page(Theme::Light, None, None);
        assert!(s.contains("Sign in"));
        assert!(s.contains(r#"action="/login""#));
        assert!(s.contains(r#"name="login""#));
        assert!(s.contains(r#"name="password""#));
    }

    #[test]
    fn login_page_renders_error() {
        let s = login_page(Theme::Light, Some("bad creds"), None);
        assert!(s.contains("bad creds"));
        assert!(s.contains("Sign-in failed"));
    }

    #[test]
    fn login_page_prefills_username() {
        let s = login_page(Theme::Light, None, Some("ada"));
        assert!(s.contains(r#"value="ada""#));
    }

    #[test]
    fn signup_page_renders_form() {
        let s = signup_page(Theme::Light, None, None, None);
        assert!(s.contains("Create an account"));
        assert!(s.contains(r#"action="/signup""#));
        assert!(s.contains(r#"name="email""#));
    }

    #[test]
    fn signup_page_renders_error() {
        let s = signup_page(Theme::Light, Some("password too short"), None, None);
        assert!(s.contains("password too short"));
    }

    #[test]
    fn validate_login_rejects_empty() {
        let f = LoginForm::default();
        assert!(validate_login(&f).is_err());
        let mut f2 = LoginForm::default();
        f2.login = "ada".into();
        assert!(validate_login(&f2).is_err());
        f2.password = "x".into();
        assert!(validate_login(&f2).is_ok());
    }

    #[test]
    fn validate_signup_enforces_min_password() {
        let mut f = SignupForm {
            username: "ada".into(),
            email: "ada@example.com".into(),
            password: "short".into(),
            display_name: None,
        };
        assert!(validate_signup(&f).is_err());
        f.password = "long-enough-1".into();
        assert!(validate_signup(&f).is_ok());
    }

    #[test]
    fn validate_signup_enforces_username_charset() {
        let f = SignupForm {
            username: "bad name!".into(),
            email: "ada@example.com".into(),
            password: "long-enough-1".into(),
            display_name: None,
        };
        assert!(validate_signup(&f).is_err());
    }

    #[test]
    fn validate_signup_rejects_short_username() {
        let f = SignupForm {
            username: "ad".into(),
            email: "ada@example.com".into(),
            password: "long-enough-1".into(),
            display_name: None,
        };
        assert!(validate_signup(&f).is_err());
    }

    #[test]
    fn validate_signup_rejects_invalid_email() {
        let f = SignupForm {
            username: "ada".into(),
            email: "no-at-sign".into(),
            password: "long-enough-1".into(),
            display_name: None,
        };
        assert!(validate_signup(&f).is_err());
    }

    #[test]
    fn safe_redirect_rejects_external() {
        assert_eq!(safe_redirect(Some("/overview")), "/overview");
        assert_eq!(safe_redirect(Some("//evil.com/x")), "/overview");
        assert_eq!(safe_redirect(Some("https://evil.com")), "/overview");
        assert_eq!(safe_redirect(None), "/overview");
    }

    #[test]
    fn login_form_deserializes() {
        let j = r#"{"login":"ada","password":"secret"}"#;
        let f: LoginForm = serde_json::from_str(j).unwrap();
        assert_eq!(f.login, "ada");
        assert_eq!(f.password, "secret");
    }

    #[test]
    fn signup_form_deserializes() {
        let j =
            r#"{"username":"ada","email":"a@b.co","password":"long-enough","display_name":"Ada"}"#;
        let f: SignupForm = serde_json::from_str(j).unwrap();
        assert_eq!(f.username, "ada");
        assert_eq!(f.display_name.as_deref(), Some("Ada"));
    }
}
