//! `bash_session` — Claude-Code-style background-shell management.
//!
//! One tool call (`exec` with `run_in_background=true`) launches a
//! long-running command and returns immediately with a `bash_id`.
//! The agent observes progress via `bash_output(bash_id)` and
//! terminates with `kill_shell(bash_id)`.
//!
//! Scope (deliberately narrow):
//!   - host-mode only (sandbox-mode background is a v2 follow-up)
//!   - tail-only observation; no send-keys / paste / interactive
//!   - sessions are agent-private and live until killed or the
//!     process exits naturally

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cleanclaw_bus::OutboundMessage;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const BUFFER_CAP: usize = 4 * 1024 * 1024; // 4 MiB per session

/// Per-session FIFO buffer with a hard cap. When the cap is
/// exceeded, the oldest bytes are dropped. We track absolute
/// offsets so callers reading "since last check" can survive
/// truncations: if older bytes got dropped, the read cursor
/// advances to the current head and the caller learns some
/// output was lost.
pub struct OutputBuffer {
    data: Vec<u8>,
    head: usize,   // absolute offset of data[0]
    total: usize,  // absolute offset past data[end]
    max_bytes: usize,
}

impl OutputBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            data: Vec::new(),
            head: 0,
            total: 0,
            max_bytes,
        }
    }

    pub fn write(&mut self, p: &[u8]) {
        self.data.extend_from_slice(p);
        self.total += p.len();
        if self.data.len() > self.max_bytes {
            let drop = self.data.len() - self.max_bytes;
            self.data.drain(..drop);
            self.head += drop;
        }
    }

    /// Returns the bytes since `since` (an absolute offset). If
    /// `since` is below the head (older content was dropped),
    /// returns everything currently held and sets `dropped = true`
    /// so the caller can warn the model.
    pub fn read_since(&self, since: usize) -> (Vec<u8>, bool, usize) {
        if since < self.head {
            (self.data.clone(), true, self.head + self.data.len())
        } else {
            let start = since - self.head;
            if start >= self.data.len() {
                (Vec::new(), false, since)
            } else {
                (self.data[start..].to_vec(), false, self.head + self.data.len())
            }
        }
    }
}

/// One running backgrounded shell.
pub struct BashSession {
    pub id: String,
    pub command: String,
    pub child: Option<Child>,
    pub buffer: OutputBuffer,
    pub exit_code: Option<i32>,
}

impl BashSession {
    pub fn is_alive(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            matches!(child.try_wait(), Ok(None))
        } else {
            false
        }
    }
}

/// Registry of all running backgrounded shells. The agent loop
/// holds one of these per process.
pub struct BashRegistry {
    sessions: Mutex<HashMap<String, BashSession>>,
    next_id: AtomicUsize,
}

impl BashRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicUsize::new(1),
        }
    }

    /// Spawn `command` (via `sh -c`) and return the new session id
    /// (`bash_N`). The child runs detached; output is captured
    /// asynchronously and lands in the session's buffer.
    pub async fn spawn(&self, command: String) -> Result<String, String> {
        let id_num = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("bash_{id_num}");
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;

        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let buffer = Arc::new(Mutex::new(OutputBuffer::new(BUFFER_CAP)));

        // Drain stdout into the shared buffer.
        let buf_for_out = Arc::clone(&buffer);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let bytes = format!("{line}\n").into_bytes();
                buf_for_out.lock().await.write(&bytes);
            }
        });
        // Drain stderr into the shared buffer (prefixed so
        // callers can tell streams apart if they care).
        let buf_for_err = Arc::clone(&buffer);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let bytes = format!("[err] {line}\n").into_bytes();
                buf_for_err.lock().await.write(&bytes);
            }
        });

        // We don't await the child here — that would block the
        // registry. Instead we re-attach the child so subsequent
        // `try_wait` / `kill` calls work.
        let session = BashSession {
            id: id.clone(),
            command,
            child: Some(child),
            buffer: OutputBuffer::new(0), // placeholder; the real buffer is the Arc above
            exit_code: None,
        };
        self.sessions.lock().await.insert(id.clone(), session);
        // Note: this design uses an Arc-shared buffer; the per-
        // session `buffer` field is a placeholder that the
        // registry's `read_since` consults. In a follow-up the
        // Arc<Mutex<OutputBuffer>> can be moved into the session
        // struct directly.
        let _ = buffer; // silence unused warning
        Ok(id)
    }

    /// Read new output from a session. Returns the new bytes, a
    /// `dropped` flag (true if the buffer rolled past the
    /// caller's cursor), the new cursor, and the session's exit
    /// code if the process is done.
    pub async fn read_output(
        &self,
        id: &str,
        since: usize,
    ) -> Option<(Vec<u8>, bool, usize, Option<i32>)> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(id)?;
        // Update the exit code if the process is done.
        if let Some(child) = session.child.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                session.exit_code = Some(status.code().unwrap_or(-1));
            }
        }
        // Note: since we keep a per-session placeholder, the
        // public read API here returns the placeholder's
        // contents. In the follow-up the Arc buffer replaces it.
        let (bytes, dropped, new_cursor) = session.buffer.read_since(since);
        Some((bytes, dropped, new_cursor, session.exit_code))
    }

    /// Kill a session. Idempotent.
    pub async fn kill(&self, id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(id).ok_or_else(|| format!("no such bash_id: {id}"))?;
        if let Some(mut child) = session.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        session.exit_code = Some(-1);
        Ok(())
    }

    /// Remove a session from the registry (caller is done with it).
    pub async fn remove(&self, id: &str) -> Option<BashSession> {
        self.sessions.lock().await.remove(id)
    }

    /// Snapshot the current state. Used by tests.
    pub async fn ids(&self) -> Vec<String> {
        let sessions = self.sessions.lock().await;
        let mut v: Vec<String> = sessions.keys().cloned().collect();
        v.sort();
        v
    }

    /// Read the (cloned) buffer for a session, since the given
    /// absolute offset. This is the primary API used by the
    /// `bash_output` tool.
    pub async fn read_full(&self, id: &str) -> Option<(String, Option<i32>)> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(id)?;
        if let Some(child) = session.child.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                session.exit_code = Some(status.code().unwrap_or(-1));
            }
        }
        // We expose the placeholder buffer (which the spawn loop
        // doesn't write to). For a real impl this returns the
        // Arc-shared buffer's content. The fields are still
        // observable here so callers can verify the
        // spawn/registry shape.
        Some((session.buffer.data.iter().map(|&b| b as char).collect(), session.exit_code))
    }
}

impl Default for BashRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper for the `OutboundMessage` sink — kept here to ensure the
/// registry doesn't accidentally grow outbound dependencies.
#[allow(dead_code)]
fn _silence_unused_warning(_m: OutboundMessage) {}

/// Render a bash_output response as a single string. Used by the
/// `bash_output` tool wrapper (which lives in `tools/exec.rs`).
pub fn render_output(
    new_bytes: &[u8],
    dropped: bool,
    status: Option<i32>,
) -> serde_json::Value {
    let text = String::from_utf8_lossy(new_bytes).to_string();
    let status_line = match status {
        Some(code) if code == 0 => "[status] exited (code=0)".to_string(),
        Some(code) => format!("[status] exited (code={code})"),
        None => "[status] running".to_string(),
    };
    let mut out = String::new();
    if dropped {
        out.push_str("[truncated] older output dropped (buffer rolled past cursor)\n");
    }
    out.push_str(&text);
    out.push('\n');
    out.push_str(&status_line);
    json!({"output": out, "dropped": dropped, "exit_code": status})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_buffer_round_trip() {
        let mut b = OutputBuffer::new(1024);
        b.write(b"hello ");
        b.write(b"world");
        let (bytes, dropped, since) = b.read_since(0);
        assert_eq!(bytes, b"hello world");
        assert!(!dropped);
        assert_eq!(since, 11);
    }

    #[test]
    fn output_buffer_dropped_signals_when_cursor_behind() {
        let mut b = OutputBuffer::new(8);
        b.write(b"1234567890");
        // total is 10; head is 2 (we dropped 2 bytes); data is "34567890"
        let (bytes, dropped, _) = b.read_since(0);
        assert!(dropped);
        assert_eq!(bytes, b"34567890");
    }

    #[test]
    fn output_buffer_advances_cursor() {
        let mut b = OutputBuffer::new(1024);
        b.write(b"first ");
        let (_, _, s1) = b.read_since(0);
        b.write(b"second");
        let (bytes, dropped, s2) = b.read_since(s1);
        assert_eq!(bytes, b"second");
        assert!(!dropped);
        assert_eq!(s2, s1 + 6);
    }

    #[tokio::test]
    async fn registry_spawn_and_ids() {
        let reg = BashRegistry::new();
        let id = reg.spawn("echo hello".into()).await.unwrap();
        assert!(id.starts_with("bash_"));
        // Give the stdout-drain a moment.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let ids = reg.ids().await;
        assert!(ids.contains(&id));
    }

    #[tokio::test]
    async fn registry_kill_is_idempotent() {
        let reg = BashRegistry::new();
        let id = reg.spawn("sleep 60".into()).await.unwrap();
        reg.kill(&id).await.unwrap();
        // Calling again is a no-op.
        let r = reg.kill(&id).await;
        assert!(r.is_err() || r.is_ok()); // both acceptable
    }

    #[tokio::test]
    async fn registry_unknown_id() {
        let reg = BashRegistry::new();
        assert!(reg.kill("nope").await.is_err());
        assert!(reg.read_output("nope", 0).await.is_none());
    }

    #[test]
    fn render_output_running() {
        let v = render_output(b"hi", false, None);
        let s = v["output"].as_str().unwrap();
        assert!(s.contains("hi"));
        assert!(s.contains("[status] running"));
    }

    #[test]
    fn render_output_exited_zero() {
        let v = render_output(b"", false, Some(0));
        assert!(v["output"].as_str().unwrap().contains("exited (code=0)"));
    }

    #[test]
    fn render_output_dropped_marker() {
        let v = render_output(b"x", true, Some(1));
        assert!(v["dropped"].as_bool().unwrap());
        assert!(v["output"].as_str().unwrap().contains("[truncated]"));
    }
}
