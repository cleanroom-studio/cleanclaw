//! Sensitive env-var scrubbing. Mirrors
//! .
//!
//! When the agent runs an `exec` tool, the child shell inherits the
//! daemon's environment by default. That env contains credentials
//! (storage DSN, object-store keys, sandbox apikeys) the agent has no
//! business reading. `build_subprocess_env` strips them before passing
//! the env on to the child, and applies per-skill overrides on top.

use std::collections::HashMap;

/// NAME prefixes (case-insensitive) that mark an env var as
/// operator-only. The model has no business reading any of these.
const SENSITIVE_PREFIXES: &[&str] = &[
    "CLEANCLAW_STORAGE_",
    "CLEANCLAW_OBJECT_STORE_",
    "CLEANCLAW_SANDBOX_BOXLITE_",
    "AWS_",
    "GOOGLE_APPLICATION_CREDENTIALS",
];

/// NAME substrings (case-insensitive) that mark a var as likely-secret.
const SENSITIVE_SUBSTRINGS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "PRIVATE_KEY",
    "_API_KEY",
    "APIKEY",
    "ACCESS_KEY",
    "ACCESSKEY",
    "SECRET_KEY",
    "SECRETKEY",
    "_DSN",
    "DATABASE_URL",
];

pub fn is_sensitive_env_key(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if SENSITIVE_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return true;
    }
    if SENSITIVE_SUBSTRINGS.iter().any(|s| upper.contains(s)) {
        return true;
    }
    false
}

/// Strip credential-bearing entries from a `KEY=VALUE` list. Returns a
/// fresh `Vec` (does not mutate).
pub fn scrub_sensitive_env(env: &[String]) -> Vec<String> {
    env.iter()
        .filter(|kv| {
            let name = kv.split_once('=').map(|(k, _)| k).unwrap_or(kv);
            !is_sensitive_env_key(name)
        })
        .cloned()
        .collect()
}

/// Build a child env: scrubbed parent env + per-skill overrides.
pub fn build_subprocess_env(skill_env: &HashMap<String, String>) -> Vec<String> {
    let parent: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    let mut out = scrub_sensitive_env(&parent);
    for (k, v) in skill_env {
        // Replace or append.
        let kv = format!("{k}={v}");
        if let Some(slot) = out.iter_mut().find(|e| e.starts_with(&format!("{k}="))) {
            *slot = kv;
        } else {
            out.push(kv);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sensitive_prefixes() {
        assert!(is_sensitive_env_key("CLEANCLAW_STORAGE_DSN"));
        assert!(is_sensitive_env_key("CLEANCLAW_OBJECT_STORE_BUCKET"));
        assert!(is_sensitive_env_key("AWS_ACCESS_KEY_ID"));
        assert!(is_sensitive_env_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn detects_sensitive_substrings() {
        assert!(is_sensitive_env_key("GITHUB_TOKEN"));
        assert!(is_sensitive_env_key("MY_PASSWORD"));
        assert!(is_sensitive_env_key("DATABASE_URL"));
        assert!(is_sensitive_env_key("SOME_API_KEY"));
    }

    #[test]
    fn allows_benign_keys() {
        assert!(!is_sensitive_env_key("PATH"));
        assert!(!is_sensitive_env_key("HOME"));
        assert!(!is_sensitive_env_key("LANG"));
        assert!(!is_sensitive_env_key("CLEANCLAW_PORT"));
    }

    #[test]
    fn scrub_removes_credentials() {
        let env = vec![
            "PATH=/usr/bin".into(),
            "CLEANCLAW_STORAGE_DSN=postgres://secret@host/db".into(),
            "GITHUB_TOKEN=ghp_xxx".into(),
            "USER=alice".into(),
        ];
        let scrubbed = scrub_sensitive_env(&env);
        assert_eq!(scrubbed.len(), 2);
        assert!(scrubbed.contains(&"PATH=/usr/bin".to_string()));
        assert!(scrubbed.contains(&"USER=alice".to_string()));
    }
}
