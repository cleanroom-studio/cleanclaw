//! Remote sandbox backends. E2B and BoxLite each expose a hosted
//! sandbox runtime over HTTP; this module holds the wire-format
//! types the cleanclaw-sandbox `Executor` impls use when a real
//! connection comes online. The trait impls still return
//! `NotConfigured` in the offline build, but every HTTP envelope
//! they would send is modelled here so the upstream switch is a
//! mechanical change.
//!
//!

use base64::Engine;
use serde::{Deserialize, Serialize};

// =====================================================================
// E2B
// =====================================================================

pub const E2B_API_BASE: &str = "https://api.e2b.dev";

/// E2B sandbox metadata. Returned by `POST /sandboxes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BSandbox {
    pub sandbox_id: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default = "default_e2b_state")]
    pub state: String,
}

fn default_e2b_state() -> String {
    "running".to_string()
}

/// `POST /sandboxes/{id}/process/exec` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BExecRequest {
    pub command: String,
    #[serde(default)]
    pub stdin: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout: u32,
}

fn default_timeout() -> u32 {
    60
}

/// Response body. E2B returns the captured stdout/stderr and
/// the exit code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub error: Option<String>,
}

impl E2BExecRequest {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            stdin: None,
            env: Default::default(),
            timeout: default_timeout(),
        }
    }
}

/// `GET /sandboxes/{id}/files/{path}` → base64 bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BFileResponse {
    pub content: String, // base64
    #[serde(default)]
    pub mime: Option<String>,
}

impl E2BFileResponse {
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.content)
            .map_err(|e| format!("e2b base64: {e}"))
    }
}

/// `POST /sandboxes/{id}/files/{path}` request — file upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BWriteFileRequest {
    pub content: String, // base64
}

/// E2B envd wire-level message. The CleanClaw `e2b_executor.go`
/// speaks envd's protobuf protocol directly; we expose the JSON
/// shape (also supported) so the runtime can switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BEnvdMessage {
    pub r#type: String, // "process.start", "process.data", "process.exit", …
    pub id: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BEnvdProcessStart {
    pub process: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub cwd: Option<String>,
}

// =====================================================================
// BoxLite
// =====================================================================

pub const BOXLITE_API_BASE: &str = "https://api.boxlite.ai/v1";

/// BoxLite REST API. The Go daemon uses a Swagger-generated client;
/// we model the surface we actually call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteSandbox {
    pub id: String,
    pub status: String, // "pending" | "running" | "stopped" | "error"
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// `POST /sandboxes/{id}/exec` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteExecRequest {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_boxlite_timeout")]
    pub timeout_ms: u32,
}

fn default_boxlite_timeout() -> u32 {
    60_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u32,
}

impl BoxLiteExecRequest {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            env: Default::default(),
            timeout_ms: default_boxlite_timeout(),
        }
    }
}

/// `GET /sandboxes/{id}/files?path=…` → multipart with bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteFileResponse {
    pub bytes: Vec<u8>,
    #[serde(default)]
    pub mime: Option<String>,
}

/// `WS /sandboxes/{id}/shell` envelope for interactive shells.
/// BoxLite streams process I/O over WebSocket; the envelope wraps
/// stdout/stderr/exit chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteShellFrame {
    pub kind: String,             // "stdout" | "stderr" | "exit" | "signal"
    pub data: Option<String>,     // text or base64 depending on `encoding`
    pub encoding: Option<String>, // "text" | "base64"
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn e2b_sandbox_round_trip() {
        let raw = json!({
            "sandbox_id": "sb_abc",
            "client_id": "cli_1",
            "template_id": "base",
            "domain": "example.e2b.dev",
            "state": "running"
        });
        let s: E2BSandbox = serde_json::from_value(raw).unwrap();
        assert_eq!(s.sandbox_id, "sb_abc");
        assert_eq!(s.state, "running");
    }

    #[test]
    fn e2b_sandbox_state_defaults_to_running() {
        let raw = json!({ "sandbox_id": "sb_x" });
        let s: E2BSandbox = serde_json::from_value(raw).unwrap();
        assert_eq!(s.state, "running");
    }

    #[test]
    fn e2b_exec_request_default_timeout() {
        let r = E2BExecRequest::new("ls -la");
        assert_eq!(r.command, "ls -la");
        assert_eq!(r.timeout, 60);
    }

    #[test]
    fn e2b_exec_response_round_trip() {
        let raw = json!({
            "stdout": "hello",
            "stderr": "",
            "exit_code": 0,
            "error": null
        });
        let r: E2BExecResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.stdout, "hello");
        assert_eq!(r.exit_code, 0);
        assert!(r.error.is_none());
    }

    #[test]
    fn e2b_file_response_base64_decodes() {
        let original = b"binary \x00\xff data";
        let b64 = base64::engine::general_purpose::STANDARD.encode(original);
        let r = E2BFileResponse {
            content: b64,
            mime: Some("application/octet-stream".into()),
        };
        assert_eq!(r.decode().unwrap(), original);
    }

    #[test]
    fn e2b_file_response_bad_base64_errors() {
        let r = E2BFileResponse {
            content: "!!not-base64!!".into(),
            mime: None,
        };
        assert!(r.decode().is_err());
    }

    #[test]
    fn e2b_write_file_request_round_trip() {
        let r = E2BWriteFileRequest {
            content: "aGVsbG8=".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("aGVsbG8="));
    }

    #[test]
    fn e2b_envd_message_round_trip() {
        let m = E2BEnvdMessage {
            r#type: "process.start".into(),
            id: "p1".into(),
            data: Some(json!({ "process": "/bin/sh" })),
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: E2BEnvdMessage = serde_json::from_str(&s).unwrap();
        assert_eq!(back.r#type, "process.start");
    }

    #[test]
    fn boxlite_sandbox_round_trip() {
        let raw = json!({
            "id": "box_1",
            "status": "running",
            "template": "python:3.12",
            "metadata": { "owner": "u_1" }
        });
        let s: BoxLiteSandbox = serde_json::from_value(raw).unwrap();
        assert_eq!(s.id, "box_1");
        assert_eq!(s.status, "running");
        assert_eq!(s.metadata["owner"], "u_1");
    }

    #[test]
    fn boxlite_exec_request_default_timeout() {
        let r = BoxLiteExecRequest::new("pwd");
        assert_eq!(r.timeout_ms, 60_000);
    }

    #[test]
    fn boxlite_exec_response_round_trip() {
        let raw = json!({
            "stdout": "/home/u",
            "stderr": "",
            "exit_code": 0,
            "duration_ms": 42
        });
        let r: BoxLiteExecResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(r.stdout, "/home/u");
        assert_eq!(r.duration_ms, 42);
    }

    #[test]
    fn boxlite_shell_frame_round_trip() {
        let f = BoxLiteShellFrame {
            kind: "stdout".into(),
            data: Some("hello".into()),
            encoding: Some("text".into()),
            exit_code: None,
            signal: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: BoxLiteShellFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, "stdout");
        assert_eq!(back.data.as_deref(), Some("hello"));
    }

    #[test]
    fn boxlite_shell_frame_exit_variant() {
        let f = BoxLiteShellFrame {
            kind: "exit".into(),
            data: None,
            encoding: None,
            exit_code: Some(0),
            signal: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: BoxLiteShellFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(back.exit_code, Some(0));
    }
}
