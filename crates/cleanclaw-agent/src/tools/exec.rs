//! `exec` tool — run a shell command and return stdout / stderr /
//! exit_code.
//!
//! For the first cut (no sandbox backend) the command runs on the
//! host shell with `tokio::process::Command`. The env is scrubbed
//! via `env_scrub::build_subprocess_env` so credentials don't leak.

use super::env_scrub::build_subprocess_env;
use super::{Tool, ToolContext};
use async_trait::async_trait;
use cleanclaw_core::{CleanClawError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct ExecTool;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_BYTES: usize = 256 * 1024; // truncate stdout/stderr at 256 KiB

#[derive(Deserialize)]
struct ExecArgs {
    command: String,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }
    fn description(&self) -> &str {
        "Execute a shell command and return stdout/stderr/exit_code. For binary or image output (PNG, JPEG, PDF, audio, video), write the file into the workspace and reference it by relative path in your reply — do NOT base64-encode into stdout."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to execute"},
                "stdin": {"type": "string", "description": "Optional: piped to the command's stdin"},
                "timeout": {"type": "integer", "description": "Seconds (default 120)"}
            },
            "required": ["command"]
        })
    }
    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value> {
        let a: ExecArgs = serde_json::from_value(args)
            .map_err(|e| CleanClawError::InvalidArgument(format!("parse args: {e}")))?;

        let timeout = Duration::from_secs(a.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS));
        let skill_env = std::collections::HashMap::new();
        let env = build_subprocess_env(&skill_env);

        // Run via /bin/sh -c for shell semantics (pipes, env-var
        // expansion, globbing). Spawn through tokio so the timeout
        // works without blocking the runtime.
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(&a.command)
            .env_clear()
            .envs(env.iter().map(|s| {
                let (k, v) = s.split_once('=').unwrap_or((s, ""));
                (k, v)
            }))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| CleanClawError::Internal(format!("exec spawn: {e}")))?;

        if let Some(s) = &a.stdin {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(s.as_bytes()).await;
            }
        }

        let result = tokio::time::timeout(timeout, async {
            let out = child.wait_with_output().await;
            out
        })
        .await;

        let output = match result {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(CleanClawError::Internal(format!("exec wait: {e}"))),
            Err(_) => {
                // Timeout — best-effort kill. We don't get the partial
                // output back; the model sees "timeout" + the timeout
                // duration so it can retry with a smaller batch.
                return Err(CleanClawError::Upstream(format!(
                    "exec timed out after {timeout_secs}s",
                    timeout_secs = timeout.as_secs()
                )));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout_s = truncate(&stdout, MAX_OUTPUT_BYTES);
        let stderr_s = truncate(&stderr, MAX_OUTPUT_BYTES);

        let mut result = json!({
            "exit_code": output.status.code(),
            "stdout": stdout_s,
            "stderr": stderr_s,
            "truncated": stdout.len() > MAX_OUTPUT_BYTES || stderr.len() > MAX_OUTPUT_BYTES,
        });
        if !output.status.success() {
            // Surface non-zero exit as a "soft" error so the model sees
            // the exit code in a structured field. The Tool dispatcher
            // marks the tool result as is_error=true; the agent loop
            // handles retries.
            result["is_error"] = json!(true);
        }
        Ok(result)
    }
}

fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Find a safe char boundary near the limit.
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 32);
    out.push_str(&s[..cut]);
    out.push_str("\n... [truncated]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        // "héllo" — each "é" is 2 bytes. Truncating to 5 bytes yields
        // the first 4 bytes + 1 byte of the 'é'. We back off to 4 to
        // land on a char boundary, so the prefix is "héll".
        let s = "héllo wörld";
        let out = truncate(s, 5);
        assert!(out.starts_with("héll"));
        assert!(out.contains("[truncated]"));
    }
}
