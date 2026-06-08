//! Build metadata. Stamped at compile time via `build.rs`.
//!
//!

pub const BUILD_VERSION: &str = env!("CLEANCLAW_BUILD_VERSION", "dev");
pub const BUILD_COMMIT: &str = env!("CLEANCLAW_BUILD_COMMIT", "unknown");
pub const BUILD_DATE: &str = env!("CLEANCLAW_BUILD_DATE", "unknown");

pub fn describe() -> String {
    format!("cleanclaw {} ({} {})", BUILD_VERSION, BUILD_COMMIT, BUILD_DATE)
}

fn deploy_var() -> String {
    std::env::var("CLEANCLAW_DEPLOY")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// `CLEANCLAW_DEPLOY=hosted` flips the process into hosted/multi-tenant
/// mode (cloud). Default is self-hosted. Read each call so a config-edit
/// + sighup flow can flip it without a restart.
pub fn is_hosted_deploy() -> bool {
    deploy_var() == "hosted"
}

fn host_exec_var() -> String {
    std::env::var("CLEANCLAW_ALLOW_HOST_EXEC")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// `is_host_exec_allowed` reports whether the agent runtime should
/// register the `host_exec` escape-hatch tool. Requires both:
/// 1. operator opt-in via `CLEANCLAW_ALLOW_HOST_EXEC=1|true|yes`, AND
/// 2. process is NOT a hosted multi-tenant deploy.
//
/// Default OFF. Hosted deploys are always denied.
pub fn is_host_exec_allowed() -> bool {
    if is_hosted_deploy() {
        return false;
    }
    let v = host_exec_var();
    v == "1" || v == "true" || v == "yes"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-mutating tests in Rust are racy when run in parallel, so
    // serialize the deploy / host-exec checks through a single mutex.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_is_not_hosted() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: tests in the same process; we serialize via the mutex.
        unsafe {
            std::env::remove_var("CLEANCLAW_DEPLOY");
        }
        assert!(!is_hosted_deploy());
    }

    #[test]
    fn hosted_when_set() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("CLEANCLAW_DEPLOY", "hosted");
        }
        assert!(is_hosted_deploy());
        unsafe {
            std::env::set_var("CLEANCLAW_DEPLOY", "HOSTED");
        }
        assert!(is_hosted_deploy());
        unsafe {
            std::env::set_var("CLEANCLAW_DEPLOY", "hosted  ");
        }
        assert!(is_hosted_deploy());
        unsafe {
            std::env::remove_var("CLEANCLAW_DEPLOY");
        }
    }

    #[test]
    fn self_hosted_var_is_false() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("CLEANCLAW_DEPLOY", "self-hosted");
        }
        assert!(!is_hosted_deploy());
        unsafe {
            std::env::remove_var("CLEANCLAW_DEPLOY");
        }
    }

    #[test]
    fn host_exec_default_off() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("CLEANCLAW_DEPLOY");
            std::env::remove_var("CLEANCLAW_ALLOW_HOST_EXEC");
        }
        assert!(!is_host_exec_allowed());
    }

    #[test]
    fn host_exec_on_for_self_hosted() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("CLEANCLAW_DEPLOY");
            std::env::set_var("CLEANCLAW_ALLOW_HOST_EXEC", "1");
        }
        assert!(is_host_exec_allowed());
        unsafe {
            std::env::set_var("CLEANCLAW_ALLOW_HOST_EXEC", "true");
        }
        assert!(is_host_exec_allowed());
        unsafe {
            std::env::set_var("CLEANCLAW_ALLOW_HOST_EXEC", "yes");
        }
        assert!(is_host_exec_allowed());
        unsafe {
            std::env::remove_var("CLEANCLAW_ALLOW_HOST_EXEC");
        }
    }

    #[test]
    fn host_exec_blocked_for_hosted() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("CLEANCLAW_DEPLOY", "hosted");
            std::env::set_var("CLEANCLAW_ALLOW_HOST_EXEC", "1");
        }
        assert!(!is_host_exec_allowed());
        unsafe {
            std::env::remove_var("CLEANCLAW_DEPLOY");
            std::env::remove_var("CLEANCLAW_ALLOW_HOST_EXEC");
        }
    }

    #[test]
    fn build_constants_have_defaults() {
        // Even without build.rs injection, the const! fallback gives us
        // safe defaults — important for `cargo test` and IDE workflows.
        assert!(!BUILD_VERSION.is_empty());
        assert!(!BUILD_COMMIT.is_empty());
        assert!(!BUILD_DATE.is_empty());
        let s = describe();
        assert!(s.contains("cleanclaw"));
    }
}
