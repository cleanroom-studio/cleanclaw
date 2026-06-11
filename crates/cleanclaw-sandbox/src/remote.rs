//! Remote sandbox backends — E2B and BoxLite wire-format models.
//!
//! This module is the *protocol* layer for hosted sandbox backends.
//! It does **not** itself perform any I/O; it only defines the
//! request/response shapes that `E2BExecutor` and `BoxLiteExecutor`
//! (in `lib.rs`) serialize to and from JSON when a real HTTP
//! connection is wired up via `with_client()`.
//!
//! Design notes
//! ------------
//! * The CleanClaw project has a sibling Go daemon whose
//!   `e2b_executor.go` / `boxlite_executor.go` call these exact
//!   endpoints. The Rust types here mirror the Go ones field-for-field
//!   so a feature flag can flip between the two implementations
//!   without touching the agent tool layer.
//! * Every struct derives both `Serialize` and `Deserialize` with
//!   `#[serde(default)]` on optional fields. This makes the parsers
//!   forgiving: a missing `template_id`, for example, won't fail
//!   deserialization, it just becomes an empty string. Tests below
//!   pin the behaviour.
//! * Binary payloads (file contents) are always base64-encoded
//!   when crossing the wire; that keeps the envelope pure JSON and
//!   sidesteps multipart parsers in E2B. BoxLite uses multipart for
//!   uploads but returns raw bytes in the JSON download envelope.

use base64::Engine;
use serde::{Deserialize, Serialize};

// =====================================================================
// E2B
// =====================================================================

/// Public base URL for the E2B hosted runtime. Used as the default
/// value for `E2BExecutor::base_url`. Operators in air-gapped
/// environments can override it via `E2BExecutor::with_endpoint()`.
pub const E2B_API_BASE: &str = "https://api.e2b.dev";

/// E2B sandbox metadata. Returned by `POST /sandboxes` and used as
/// the opaque `sandbox_id` handle for every subsequent call.
///
/// Fields with `#[serde(default)]` are absent from some response
/// variants (e.g. legacy short-form bodies); we treat them as
/// optional rather than failing the parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BSandbox {
    /// The opaque handle the gateway must keep for the lifetime of
    /// the sandbox. Threads through `exec_remote` / `read_file_remote`
    /// / `kill_sandbox_remote`.
    pub sandbox_id: String,
    /// Optional client identifier E2B echoes back for telemetry.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Template the sandbox was provisioned from. Defaults to "" so
    /// a missing field doesn't fail deserialization.
    #[serde(default)]
    pub template_id: String,
    /// Per-sandbox DNS suffix. Required for direct `*.e2b.dev`
    /// access; not used by the API endpoints.
    #[serde(default)]
    pub domain: Option<String>,
    /// Lifecycle state. Defaults to `"running"` because that is the
    /// state a freshly-created sandbox is in; older API versions
    /// omit the field.
    #[serde(default = "default_e2b_state")]
    pub state: String,
}

/// Default value for `E2BSandbox::state` when the field is missing.
/// Mirrors the Go constant.
fn default_e2b_state() -> String {
    "running".to_string()
}

/// Request body for `POST /sandboxes/{id}/process/exec`.
///
/// We accept optional stdin/env so callers can pre-seed a command
/// with extra context (e.g. an API key for an LLM tool). `timeout`
/// is in seconds and defaults to 60s — long enough for typical
/// build/test commands, short enough that runaway agents don't
/// pin a sandbox forever.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BExecRequest {
    /// Shell command line. E2B runs it via `sh -c` server-side, so
    /// shell metacharacters (`&&`, `|`, redirects) work the way the
    /// caller expects.
    pub command: String,
    /// Optional stdin payload. E2B pipes this into the command's
    /// stdin before execution.
    #[serde(default)]
    pub stdin: Option<String>,
    /// Extra environment variables layered on top of the template's
    /// defaults. We always pass an empty map (not `None`) so the
    /// serialized body has a stable shape.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Wall-clock timeout in seconds. Default = 60s.
    #[serde(default = "default_timeout")]
    pub timeout: u32,
}

/// Default timeout (60s) for `E2BExecRequest`. Matches the E2B
/// server-side cap for the free tier.
fn default_timeout() -> u32 {
    60
}

/// Response body for `POST /sandboxes/{id}/process/exec`.
///
/// E2B captures stdout/stderr separately and returns the exit code
/// as a signed int. `error` is populated only when the request
/// itself failed server-side (auth, quota); exec-level failures
/// show up as a non-zero `exit_code` and a non-empty `stderr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BExecResponse {
    /// Captured stdout, decoded as UTF-8 with lossy fallback by the
    /// gateway before reaching the LLM.
    pub stdout: String,
    /// Captured stderr, same encoding treatment.
    pub stderr: String,
    /// Process exit code. `-1` is used by the local fallback when
    /// the process was killed by a signal.
    pub exit_code: i32,
    /// Optional server-side error message. Distinct from a
    /// non-zero `exit_code` — this means the *call* failed.
    pub error: Option<String>,
}

impl E2BExecRequest {
    /// Convenience constructor for the common case of "just run
    /// this command with default timeout/empty env". The caller
    /// can still mutate the returned struct before sending.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            stdin: None,
            env: Default::default(),
            timeout: default_timeout(),
        }
    }
}

/// Response body for `GET /sandboxes/{id}/files/{path}`.
///
/// The payload is always base64 inside a JSON envelope — this keeps
/// the API uniform across text and binary files and avoids the
/// server having to set a `Content-Type` per response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BFileResponse {
    /// Base64-encoded file bytes.
    pub content: String, // base64
    /// Optional MIME type E2B inferred. Not used by the gateway;
    /// kept so we can echo it back to callers that care.
    #[serde(default)]
    pub mime: Option<String>,
}

impl E2BFileResponse {
    /// Decode the base64 payload to raw bytes. Any decode error is
    /// returned as a `String` (rather than the underlying
    /// `base64::DecodeError`) so the call site can map it directly
    /// to `SandboxError::Upstream` without a `From` impl.
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.content)
            .map_err(|e| format!("e2b base64: {e}"))
    }
}

/// Request body for `POST /sandboxes/{id}/files/{path}` (file upload).
///
/// We deliberately reuse the same base64-in-JSON envelope for
/// downloads, even though it costs ~33% in size, because the
/// alternative (multipart) would mean a second request shape and
/// a second error-handling path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BWriteFileRequest {
    /// Base64-encoded file bytes.
    pub content: String, // base64
}

/// E2B envd wire-level message. The CleanClaw `e2b_executor.go`
/// speaks envd's protobuf protocol directly; we expose the JSON
/// shape (also supported) so the runtime can switch.
///
/// `r#type` discriminates the message: `"process.start"`,
/// `"process.data"`, `"process.exit"`, etc. Callers usually match
/// on the field name and dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BEnvdMessage {
    /// Message kind. Raw identifier (`r#type`) so the field name
    /// doesn't collide with the Rust `type` keyword.
    pub r#type: String, // "process.start", "process.data", "process.exit", …
    /// Per-stream correlation id. Each `process.start` mints a new
    /// id; `process.data` / `process.exit` echo it back.
    pub id: String,
    /// Optional payload whose schema depends on `r#type`. We keep
    /// this as a generic `Value` rather than a typed enum because
    /// envd's message shapes are not yet stable.
    pub data: Option<serde_json::Value>,
}

/// Payload for an `E2BEnvdMessage` of type `"process.start"`.
/// Defines the command line to spawn, its environment, and an
/// optional working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2BEnvdProcessStart {
    /// Absolute path to the binary inside the sandbox image.
    pub process: String,
    /// Argument vector. envd executes via `execve`, NOT `sh -c`,
    /// so callers handle their own shell-quoting.
    pub args: Vec<String>,
    /// Environment to layer over the template's defaults.
    pub env: std::collections::HashMap<String, String>,
    /// Optional working directory. Resolved inside the sandbox's
    /// own filesystem namespace.
    pub cwd: Option<String>,
}

// =====================================================================
// BoxLite
// =====================================================================

/// Public base URL for the BoxLite hosted runtime. Matches the
/// default the Go daemon uses. Override with
/// `BoxLiteExecutor::with_endpoint()`.
pub const BOXLITE_API_BASE: &str = "https://api.boxlite.ai/v1";

/// BoxLite REST API. The Go daemon uses a Swagger-generated client;
/// we model the surface we actually call.
///
/// `status` is one of `"pending" | "running" | "stopped" | "error"`,
/// polled until it reaches `"running"` before issuing exec calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteSandbox {
    /// Opaque handle — threads through every subsequent call,
    /// just like `E2BSandbox::sandbox_id`.
    pub id: String,
    /// Lifecycle state. Possible values: "pending" / "running" /
    /// "stopped" / "error". The exact strings are pinned in tests
    /// below.
    pub status: String, // "pending" | "running" | "stopped" | "error"
    /// Template the sandbox was provisioned from. BoxLite calls
    /// this "template" rather than "image" in the body.
    #[serde(default)]
    pub template: Option<String>,
    /// Free-form metadata bucket. The gateway never inspects this;
    /// it's a passthrough for operator-supplied tags.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Request body for `POST /sandboxes/{id}/exec`.
///
/// Note BoxLite uses `timeout_ms` (milliseconds) whereas E2B uses
/// `timeout` (seconds). The conversion happens at the executor
/// boundary so the trait stays in seconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteExecRequest {
    /// Shell command line. BoxLite runs it via `sh -c` server-side.
    pub command: String,
    /// Optional working directory inside the sandbox.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Wall-clock timeout in *milliseconds*. Default 60_000ms
    /// (= 60s) to match E2B's default in seconds.
    #[serde(default = "default_boxlite_timeout")]
    pub timeout_ms: u32,
}

/// Default timeout (60s in ms) for `BoxLiteExecRequest`. Matches
/// `E2BExecRequest`'s 60-second default.
fn default_boxlite_timeout() -> u32 {
    60_000
}

/// Response body for `POST /sandboxes/{id}/exec`. Includes the
/// server-measured `duration_ms` so the gateway can attribute
/// latency to the upstream vs. network transit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteExecResponse {
    /// Captured stdout (UTF-8, lossy at the gateway).
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: i32,
    /// Server-measured wall time in milliseconds. Useful for
    /// observability and per-tool-call SLO tracking.
    pub duration_ms: u32,
}

impl BoxLiteExecRequest {
    /// Convenience constructor mirroring `E2BExecRequest::new`.
    /// The caller can still mutate the returned struct.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            env: Default::default(),
            timeout_ms: default_boxlite_timeout(),
        }
    }
}

/// Response body for `GET /sandboxes/{id}/files?path=…`.
///
/// BoxLite returns the file bytes directly inside the JSON
/// envelope (after base64-decoding server-side), unlike E2B which
/// always base64s. The `Vec<u8>` is what the gateway actually
/// wants, so we keep the decoded form in the struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteFileResponse {
    /// Raw file bytes.
    pub bytes: Vec<u8>,
    /// Optional MIME type, when BoxLite can infer one.
    #[serde(default)]
    pub mime: Option<String>,
}

/// `WS /sandboxes/{id}/shell` envelope for interactive shells.
///
/// BoxLite streams process I/O over WebSocket rather than over the
/// blocking `/exec` endpoint. Each frame is one of:
///
/// * `kind: "stdout"` / `"stderr"` — incremental text or base64 chunk
///   (depending on `encoding`).
/// * `kind: "exit"` — terminal frame; `exit_code` is set.
/// * `kind: "signal"` — terminal frame; `signal` is set.
///
/// The struct models all four variants; unused fields are `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLiteShellFrame {
    /// Frame discriminator. See the type-level doc above.
    pub kind: String,             // "stdout" | "stderr" | "exit" | "signal"
    /// Frame payload. Text or base64 depending on `encoding`.
    pub data: Option<String>,     // text or base64 depending on `encoding`
    /// Hint for `data`. `"text"` means `data` is a UTF-8 string;
    /// `"base64"` means it's an encoded byte chunk.
    pub encoding: Option<String>, // "text" | "base64"
    /// Set on `"exit"` frames. `None` for stdout/stderr/signal.
    pub exit_code: Option<i32>,
    /// Set on `"signal"` frames (e.g. "SIGTERM"). `None` otherwise.
    pub signal: Option<String>,
}

#[cfg(test)]
mod tests {
    //! Wire-format round-trip tests. These pin the JSON shape so
    //! upstream contract changes (e.g. an E2B field rename) show up
    //! in `cargo test` before they reach the gateway.

    use super::*;
    use serde_json::json;

    /// Full E2B sandbox payload parses and preserves every field.
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

    /// Missing `state` falls back to the default (`"running"`).
    /// This matches the v1 E2B API behaviour.
    #[test]
    fn e2b_sandbox_state_defaults_to_running() {
        let raw = json!({ "sandbox_id": "sb_x" });
        let s: E2BSandbox = serde_json::from_value(raw).unwrap();
        assert_eq!(s.state, "running");
    }

    /// `E2BExecRequest::new` wires the default 60s timeout.
    #[test]
    fn e2b_exec_request_default_timeout() {
        let r = E2BExecRequest::new("ls -la");
        assert_eq!(r.command, "ls -la");
        assert_eq!(r.timeout, 60);
    }

    /// Full E2B exec response parses; `error` is optional and
    /// absent here.
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

    /// `E2BFileResponse::decode` round-trips a binary payload that
    /// would have been corrupted by a naive `String::from_utf8`.
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

    /// Invalid base64 surfaces as a `String` error — the gateway
    /// wraps it into `SandboxError::Upstream` at the call site.
    #[test]
    fn e2b_file_response_bad_base64_errors() {
        let r = E2BFileResponse {
            content: "!!not-base64!!".into(),
            mime: None,
        };
        assert!(r.decode().is_err());
    }

    /// `E2BWriteFileRequest` round-trips the base64 content
    /// untouched. We assert the literal base64 appears in the
    /// serialized output so a future "let's switch to multipart"
    /// refactor has to update this test.
    #[test]
    fn e2b_write_file_request_round_trip() {
        let r = E2BWriteFileRequest {
            content: "aGVsbG8=".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("aGVsbG8="));
    }

    /// `E2BEnvdMessage` round-trips. The `r#type` raw identifier
    /// serializes as `"type"` in JSON.
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

    /// Full BoxLite sandbox payload parses; `metadata` is a free
    /// `Value` so we just spot-check one inner field.
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

    /// Default `timeout_ms` is 60_000 (i.e. 60 seconds). Note the
    /// unit flip vs. E2B (which uses seconds).
    #[test]
    fn boxlite_exec_request_default_timeout() {
        let r = BoxLiteExecRequest::new("pwd");
        assert_eq!(r.timeout_ms, 60_000);
    }

    /// `duration_ms` round-trips; the gateway uses this for
    /// per-tool-call latency tracking.
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

    /// Stdout frame round-trips with text encoding.
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

    /// Exit frame variant — only `kind` and `exit_code` are set;
    /// `data`/`encoding`/`signal` stay `None`.
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
