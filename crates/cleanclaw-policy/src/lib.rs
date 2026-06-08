//! Agent access policy.
//!
//! Defines what an agent is allowed to do (filesystem / network /
//! tools / resources) and provides an `Engine` to evaluate rules.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Policy {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub filesystem: FsPolicy,
    #[serde(default)]
    pub network: NetPolicy,
    #[serde(default)]
    pub tools: ToolsPolicy,
    #[serde(default)]
    pub resources: ResPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FsPolicy {
    #[serde(rename = "allowRead", default)]
    pub allow_read: Vec<String>,
    #[serde(rename = "allowWrite", default)]
    pub allow_write: Vec<String>,
    #[serde(rename = "denyRead", default)]
    pub deny_read: Vec<String>,
    #[serde(rename = "denyWrite", default)]
    pub deny_write: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NetPolicy {
    #[serde(default)]
    pub outbound: Vec<NetRule>,
    /// `"none"`, `"allowlist"`, or `"permissive"`.
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NetRule {
    pub host: String,
    #[serde(default)]
    pub ports: Vec<u16>,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ToolsPolicy {
    /// Tool names; `"*"` is wildcard.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Deny always wins over allow.
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ResPolicy {
    #[serde(rename = "maxCpu", default)]
    pub max_cpu: String,
    #[serde(rename = "maxMemory", default)]
    pub max_memory: String,
    #[serde(rename = "maxDiskMb", default)]
    pub max_disk_mb: i32,
    #[serde(rename = "execTimeoutSec", default)]
    pub exec_timeout_sec: i32,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("policy: write denied for {path} (matches {pattern})")]
    WriteDenied { path: String, pattern: String },
    #[error("policy: write not allowed for {0}")]
    WriteNotAllowed(String),
    #[error("policy: read denied for {path} (matches {pattern})")]
    ReadDenied { path: String, pattern: String },
    #[error("policy: read not allowed for {0}")]
    ReadNotAllowed(String),
    #[error("policy: all network access denied")]
    NetworkDenied,
    #[error("policy: network access denied for {host}:{port}")]
    NetworkHostDenied { host: String, port: u16 },
    #[error("policy: tool {0:?} denied")]
    ToolDenied(String),
    #[error("policy: tool {0:?} not allowed")]
    ToolNotAllowed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// `DefaultPolicy` — permissive, allows everything.
pub fn default_policy() -> Policy {
    Policy {
        name: "permissive".into(),
        description: "Allows all operations (default)".into(),
        network: NetPolicy {
            mode: "permissive".into(),
            ..Default::default()
        },
        tools: ToolsPolicy {
            allow: vec!["*".into()],
            ..Default::default()
        },
        ..Default::default()
    }
}

/// `RestrictedPolicy` — locked down, denies everything by default.
pub fn restricted_policy() -> Policy {
    Policy {
        name: "restricted".into(),
        description: "Denies all operations unless explicitly allowed".into(),
        filesystem: FsPolicy {
            deny_write: vec![
                "/etc/*".into(),
                "/usr/*".into(),
                "/bin/*".into(),
                "/sbin/*".into(),
                "/var/*".into(),
            ],
            deny_read: vec!["/etc/shadow".into(), "/etc/passwd".into()],
            ..Default::default()
        },
        network: NetPolicy {
            mode: "none".into(),
            ..Default::default()
        },
        tools: ToolsPolicy {
            deny: vec!["exec".into()],
            ..Default::default()
        },
        resources: ResPolicy {
            max_cpu: "1".into(),
            max_memory: "256m".into(),
            exec_timeout_sec: 30,
            ..Default::default()
        },
    }
}

/// `StandardPolicy` — sensible middle ground.
pub fn standard_policy() -> Policy {
    Policy {
        name: "standard".into(),
        description: "Sensible defaults: no write to system dirs, allowlist network".into(),
        filesystem: FsPolicy {
            deny_write: vec![
                "/etc/*".into(),
                "/usr/*".into(),
                "/bin/*".into(),
                "/sbin/*".into(),
            ],
            deny_read: vec!["/etc/shadow".into()],
            ..Default::default()
        },
        network: NetPolicy {
            mode: "permissive".into(),
            ..Default::default()
        },
        tools: ToolsPolicy {
            allow: vec!["*".into()],
            ..Default::default()
        },
        resources: ResPolicy {
            max_cpu: "2".into(),
            max_memory: "512m".into(),
            exec_timeout_sec: 60,
            ..Default::default()
        },
    }
}

pub fn load_preset(name: &str) -> Policy {
    match name.to_ascii_lowercase().as_str() {
        "restricted" => restricted_policy(),
        "standard" => standard_policy(),
        _ => default_policy(),
    }
}

pub fn load_from_file(path: impl AsRef<Path>) -> Result<Policy, PolicyError> {
    let data = std::fs::read(path)?;
    let p: Policy = serde_yaml::from_slice(&data)?;
    Ok(p)
}

pub struct Engine {
    policy: Policy,
}

impl Engine {
    pub fn new(policy: Policy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn check_filesystem(&self, path: &str, write: bool) -> Result<(), PolicyError> {
        let fs = &self.policy.filesystem;
        if write {
            for pattern in &fs.deny_write {
                if match_glob(pattern, path) {
                    return Err(PolicyError::WriteDenied {
                        path: path.to_string(),
                        pattern: pattern.clone(),
                    });
                }
            }
            if !fs.allow_write.is_empty() && !match_any(&fs.allow_write, path) {
                return Err(PolicyError::WriteNotAllowed(path.to_string()));
            }
        } else {
            for pattern in &fs.deny_read {
                if match_glob(pattern, path) {
                    return Err(PolicyError::ReadDenied {
                        path: path.to_string(),
                        pattern: pattern.clone(),
                    });
                }
            }
            if !fs.allow_read.is_empty() && !match_any(&fs.allow_read, path) {
                return Err(PolicyError::ReadNotAllowed(path.to_string()));
            }
        }
        Ok(())
    }

    pub fn check_network(
        &self,
        host: &str,
        port: u16,
        method: &str,
        path: &str,
    ) -> Result<(), PolicyError> {
        let net = &self.policy.network;
        match net.mode.as_str() {
            "none" => Err(PolicyError::NetworkDenied),
            "permissive" | "" => Ok(()),
            "allowlist" => {
                for rule in &net.outbound {
                    if !match_host(&rule.host, host) {
                        continue;
                    }
                    if !rule.ports.is_empty() && !rule.ports.contains(&port) {
                        continue;
                    }
                    if !rule.methods.is_empty()
                        && !rule.methods.iter().any(|m| m.eq_ignore_ascii_case(method))
                    {
                        continue;
                    }
                    if !rule.paths.is_empty() && !match_any(&rule.paths, path) {
                        continue;
                    }
                    return Ok(());
                }
                Err(PolicyError::NetworkHostDenied {
                    host: host.to_string(),
                    port,
                })
            }
            _ => Ok(()),
        }
    }

    pub fn check_tool(&self, tool_name: &str) -> Result<(), PolicyError> {
        let tools = &self.policy.tools;
        for d in &tools.deny {
            if d == tool_name || d == "*" {
                return Err(PolicyError::ToolDenied(tool_name.to_string()));
            }
        }
        if !tools.allow.is_empty() {
            for a in &tools.allow {
                if a == tool_name || a == "*" {
                    return Ok(());
                }
            }
            return Err(PolicyError::ToolNotAllowed(tool_name.to_string()));
        }
        Ok(())
    }
}

fn match_glob(pattern: &str, path: &str) -> bool {
    // 1. exact glob on full path
    if glob::Pattern::new(pattern)
        .map(|p| p.matches(path))
        .unwrap_or(false)
    {
        return true;
    }
    // 2. glob against basename (so "/etc/*" denies "/etc/passwd")
    let base = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    glob::Pattern::new(pattern)
        .map(|p| p.matches(base))
        .unwrap_or(false)
}

fn match_any(patterns: &[String], path: &str) -> bool {
    for p in patterns {
        if match_glob(p, path) {
            return true;
        }
        // Trailing `*` is treated as a prefix wildcard.
        if let Some(prefix) = p.strip_suffix('*') {
            if path.starts_with(prefix) {
                return true;
            }
        }
        // Plain directory prefix (e.g. "/workspace" allows "/workspace/foo").
        if path.starts_with(p) {
            return true;
        }
    }
    false
}

fn match_host(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // Wildcard subdomain: `*.example.com` matches `x.example.com` and `example.com`.
        let dot_suffix = format!(".{suffix}");
        return host.ends_with(&dot_suffix) || host == suffix;
    }
    pattern == host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_allows_everything() {
        let p = default_policy();
        let e = Engine::new(p);
        assert!(e.check_filesystem("/etc/passwd", true).is_ok());
        assert!(e.check_network("evil.example.com", 443, "GET", "/").is_ok());
        assert!(e.check_tool("exec").is_ok());
    }

    #[test]
    fn restricted_denies_etc_writes() {
        let p = restricted_policy();
        let e = Engine::new(p);
        let err = e.check_filesystem("/etc/hosts", true).unwrap_err();
        assert!(matches!(err, PolicyError::WriteDenied { .. }));
    }

    #[test]
    fn restricted_denies_shadow_read() {
        let p = restricted_policy();
        let e = Engine::new(p);
        let err = e.check_filesystem("/etc/shadow", false).unwrap_err();
        assert!(matches!(err, PolicyError::ReadDenied { .. }));
    }

    #[test]
    fn restricted_blocks_all_network() {
        let p = restricted_policy();
        let e = Engine::new(p);
        assert!(matches!(
            e.check_network("x", 80, "GET", "/"),
            Err(PolicyError::NetworkDenied)
        ));
    }

    #[test]
    fn restricted_denies_exec_tool() {
        let p = restricted_policy();
        let e = Engine::new(p);
        assert!(matches!(
            e.check_tool("exec"),
            Err(PolicyError::ToolDenied(_))
        ));
    }

    #[test]
    fn standard_permits_network() {
        let p = standard_policy();
        let e = Engine::new(p);
        assert!(e.check_network("x", 80, "GET", "/").is_ok());
    }

    #[test]
    fn load_preset_known() {
        assert_eq!(load_preset("restricted").name, "restricted");
        assert_eq!(load_preset("STANDARD").name, "standard");
        assert_eq!(load_preset("anything-else").name, "permissive");
    }

    #[test]
    fn allowlist_network_matches_wildcard_subdomain() {
        let mut p = default_policy();
        p.network.mode = "allowlist".into();
        p.network.outbound = vec![NetRule {
            host: "*.example.com".into(),
            ports: vec![443],
            methods: vec!["GET".into()],
            paths: vec!["/api/*".into()],
        }];
        let e = Engine::new(p);
        assert!(e
            .check_network("api.example.com", 443, "GET", "/api/v1/x")
            .is_ok());
        assert!(e
            .check_network("example.com", 443, "GET", "/api/v1/x")
            .is_ok());
        assert!(e
            .check_network("api.example.com", 80, "GET", "/api/v1/x")
            .is_err());
        assert!(e
            .check_network("api.example.com", 443, "POST", "/api/v1/x")
            .is_err());
        assert!(e
            .check_network("evil.com", 443, "GET", "/api/v1/x")
            .is_err());
    }

    #[test]
    fn wildcard_host_matches_everything() {
        let mut p = default_policy();
        p.network.mode = "allowlist".into();
        p.network.outbound = vec![NetRule {
            host: "*".into(),
            ..Default::default()
        }];
        let e = Engine::new(p);
        assert!(e.check_network("anything.test", 1, "GET", "/").is_ok());
    }

    #[test]
    fn allow_read_with_prefix_pattern() {
        let mut p = default_policy();
        p.filesystem.allow_read = vec!["/workspace".into()];
        let e = Engine::new(p);
        assert!(e.check_filesystem("/workspace/file.txt", false).is_ok());
        assert!(e.check_filesystem("/etc/hosts", false).is_err());
    }

    #[test]
    fn tool_deny_wins_over_allow() {
        let p = Policy {
            tools: ToolsPolicy {
                allow: vec!["*".into()],
                deny: vec!["exec".into()],
            },
            ..default_policy()
        };
        let e = Engine::new(p);
        assert!(e.check_tool("read_file").is_ok());
        assert!(matches!(
            e.check_tool("exec"),
            Err(PolicyError::ToolDenied(_))
        ));
    }

    #[test]
    fn yaml_round_trip() {
        let p = standard_policy();
        let blob = serde_yaml::to_string(&p).unwrap();
        let back: Policy = serde_yaml::from_str(&blob).unwrap();
        assert_eq!(back.name, "standard");
        assert_eq!(back.filesystem.deny_write, p.filesystem.deny_write);
    }
}
