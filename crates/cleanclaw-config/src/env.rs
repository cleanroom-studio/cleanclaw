//! Boot-time environment configuration.
//!
//! Bootstrap is env-only —
//! everything user-facing (providers, channels, agents) lives in the DB.

use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvConfig {
    pub gateway: EnvGateway,
    pub storage: EnvStorage,
    pub sandbox: EnvSandbox,
    pub log: EnvLog,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvGateway {
    pub port: u16,
    pub bind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvStorage {
    pub r#type: String,
    pub dsn: String,
    pub auto_migrate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvSandbox {
    pub enabled: bool,
    pub backend: String,
    pub image: String,
    pub e2b_key: String,
    pub boxlite_url: String,
    pub boxlite_client_id: String,
    pub boxlite_key: String,
    pub boxlite_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvLog {
    pub level: String,
}

fn env_or<K: AsRef<str>>(key: K, default: &str) -> String {
    env::var(key.as_ref()).unwrap_or_else(|_| default.to_string())
}

fn env_bool<K: AsRef<str>>(key: K, default: bool) -> bool {
    match env::var(key.as_ref()) {
        Ok(v) => matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on"),
        Err(_) => default,
    }
}

fn env_u16<K: AsRef<str>>(key: K, default: u16) -> u16 {
    env::var(key.as_ref())
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(default)
}

pub fn load_env() -> EnvConfig {
    let storage_type = env_or("CLEANCLAW_STORAGE_TYPE", "sqlite");
    let auto_migrate = env_bool("CLEANCLAW_STORAGE_AUTO_MIGRATE", true);

    let sandbox_backend = env_or("CLEANCLAW_SANDBOX_BACKEND", "");
    let sandbox_enabled_explicit = env_bool("CLEANCLAW_SANDBOX_ENABLED", false);
    // Setting a backend implies the operator wants sandbox on.
    let sandbox_enabled = sandbox_enabled_explicit || !sandbox_backend.is_empty();

    EnvConfig {
        gateway: EnvGateway {
            port: env_u16("CLEANCLAW_PORT", 18953),
            bind: env_or("CLEANCLAW_BIND", "loopback"),
        },
        storage: EnvStorage {
            r#type: storage_type,
            dsn: env_or("CLEANCLAW_STORAGE_DSN", ""),
            auto_migrate,
        },
        sandbox: EnvSandbox {
            enabled: sandbox_enabled,
            backend: sandbox_backend,
            image: env_or("CLEANCLAW_SANDBOX_IMAGE", ""),
            e2b_key: env_or("E2B_API_KEY", ""),
            boxlite_url: env_or("CLEANCLAW_SANDBOX_BOXLITE_URL", ""),
            boxlite_client_id: env_or("CLEANCLAW_SANDBOX_BOXLITE_CLIENT_ID", "default"),
            boxlite_key: env_or("BOXLITE_API_KEY", ""),
            boxlite_prefix: env_or("CLEANCLAW_SANDBOX_BOXLITE_PREFIX", "default"),
        },
        log: EnvLog {
            level: env_or("CLEANCLAW_LOG_LEVEL", "info"),
        },
    }
}

/// `CLEANCLAW_HOME` directory, defaulting to `~/.cleanclaw`.
pub fn home_dir() -> std::path::PathBuf {
    env::var_os("CLEANCLAW_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cleanclaw")))
        .unwrap_or_else(|| std::path::PathBuf::from(".cleanclaw"))
}

/// Remove credential-bearing env vars from the process environment AFTER
/// bootstrap config has been read.
//
/// Closes the `/proc/<pid>/environ` path a shell-having LLM could otherwise
/// use to recover the daemon's storage DSN and object-store keys.
pub fn scrub_boot_secrets() {
    for k in [
        "CLEANCLAW_STORAGE_DSN",
        "CLEANCLAW_OBJECT_STORE_TYPE",
        "CLEANCLAW_OBJECT_STORE_LOCAL_ROOT",
        "CLEANCLAW_OBJECT_STORE_REGION",
        "CLEANCLAW_OBJECT_STORE_BUCKET",
        "CLEANCLAW_OBJECT_STORE_PREFIX",
        "CLEANCLAW_OBJECT_STORE_ACCESSKEY",
        "CLEANCLAW_OBJECT_STORE_SECRETKEY",
        "CLEANCLAW_OBJECT_STORE_ACCOUNTID",
        "CLEANCLAW_OBJECT_STORE_ENDPOINT",
        "CLEANCLAW_OBJECT_STORE_USESSL",
        "CLEANCLAW_OBJECT_STORE_ALIYUN_INTERNAL",
        "BOXLITE_API_KEY",
        "E2B_API_KEY",
    ] {
        env::remove_var(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_env_defaults_to_sqlite_loopback() {
        let cfg = load_env();
        assert_eq!(cfg.gateway.port, 18953);
        assert_eq!(cfg.gateway.bind, "loopback");
        assert_eq!(cfg.storage.r#type, "sqlite");
        assert!(cfg.storage.auto_migrate);
    }

    #[test]
    fn scrub_does_not_panic_when_keys_absent() {
        for k in [
            "CLEANCLAW_STORAGE_DSN",
            "BOXLITE_API_KEY",
            "E2B_API_KEY",
        ] {
            env::remove_var(k);
        }
        scrub_boot_secrets();
    }

    #[test]
    fn home_dir_respects_override() {
        env::set_var("CLEANCLAW_HOME", "/tmp/cleanclaw-test-home");
        let p = home_dir();
        env::remove_var("CLEANCLAW_HOME");
        assert_eq!(p, std::path::PathBuf::from("/tmp/cleanclaw-test-home"));
    }
}
