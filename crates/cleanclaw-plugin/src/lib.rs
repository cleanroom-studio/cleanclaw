//! Plugin runtime.
//!
//! Each plugin is an external subprocess that speaks JSON-RPC 2.0 over
//! its stdin/stdout. Plugins register capabilities (channel / tool /
//! provider / hook) via a `plugin.json` manifest; the manager spawns
//! the process, runs `initialize`, and routes subsequent calls.

use cleanclaw_bus::OutboundMessage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plugin {0} not found")]
    NotFound(String),
    #[error("plugin returned error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("plugin exited: {0}")]
    Exited(String),
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("not connected")]
    NotConnected,
    #[error("other: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    Channel,
    Tool,
    Provider,
    Hook,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestConfigDef {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub default: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginType,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, rename = "config")]
    pub config_schema: HashMap<String, ManifestConfigDef>,
    /// Directory containing the plugin (set by the manager, not the
    /// manifest file).
    #[serde(skip)]
    pub dir: PathBuf,
}

impl Manifest {
    pub fn from_file(path: &std::path::Path) -> Result<Self, PluginError> {
        let data = std::fs::read_to_string(path)?;
        let mut m: Manifest = serde_json::from_str(&data)?;
        m.dir = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        Ok(m)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rpc error {}: {}", self.code, self.message)
    }
}

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_SHUTDOWN: &str = "shutdown";
pub const METHOD_TOOL_LIST: &str = "tool.list";
pub const METHOD_TOOL_EXECUTE: &str = "tool.execute";
pub const METHOD_PROVIDER_LIST: &str = "provider.list";
pub const METHOD_PROVIDER_EXECUTE: &str = "provider.execute";
pub const METHOD_HOOK_FIRE: &str = "hook.fire";
pub const NOTIF_READY: &str = "ready";

// =====================================================================
// Type aliases for complex types
// =====================================================================

/// Pending requests map: JSON-RPC id -> oneshot sender for response.
type PendingMap = Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Response>>>>;

/// Notification handler for plugin subprocess.
type NotifyHandler = Arc<Mutex<Option<Box<dyn Fn(Notification) + Send + Sync>>>>;

// =====================================================================
// Process — one plugin subprocess.
// =====================================================================

#[async_trait]
pub trait PluginProcess: Send + Sync {
    fn id(&self) -> &str;
    async fn start(&self) -> Result<(), PluginError>;
    async fn call(&self, method: &str, params: Value) -> Result<Value, PluginError>;
    async fn notify(&self, method: &str, params: Value) -> Result<(), PluginError>;
    async fn stop(&self) -> Result<(), PluginError>;
}

pub struct Subprocess {
    manifest: Manifest,
    state: Mutex<Option<SubprocessState>>,
    next_id: AtomicI64,
    /// Pending `call()` requests waiting for a response. Keyed by
    /// the JSON-RPC `id` the subprocess will echo back. Wrapped
    /// in `Arc` so the read-loop task spawned in `start()` can
    /// share the same map.
    pending: PendingMap,
    /// Optional callback for unsolicited `Notification` messages
    /// (no `id`, just `method` + optional `params`). Plugins use
    /// these for streamed events (chat.send, status, …).
    on_notify: NotifyHandler,
}

struct SubprocessState {
    child: Child,
    stdin: tokio::process::ChildStdin,
    /// Reader is owned by the read-loop task; the public `Subprocess`
    /// no longer holds the `BufReader` (otherwise we'd race with the
    /// read task). The field stays here as a guard — the task detaches
    /// on `start()` and is joined indirectly via `stop()` killing the
    /// child (which closes stdout and makes `read_line` return EOF).
    _read_task: tokio::task::JoinHandle<()>,
}

impl Subprocess {
    pub fn new(manifest: Manifest) -> Self {
        Self {
            manifest,
            state: Mutex::new(None),
            next_id: AtomicI64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            on_notify: Arc::new(Mutex::new(None)),
        }
    }

    #[allow(dead_code)]
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Register a callback for plugin-initiated notifications
    /// (messages with `method` but no `id`). Mirrors
    /// `Process.SetNotifyHandler` on the Go side.
    pub async fn set_notify_handler<F>(&self, f: F)
    where
        F: Fn(Notification) + Send + Sync + 'static,
    {
        *self.on_notify.lock().await = Some(Box::new(f));
    }

    /// Spawn the background task that reads one JSON object per
    /// line from the subprocess's stdout, dispatches responses to
    /// the matching `pending` oneshot, and forwards notifications
    /// to the registered handler.
    //
    /// Mirrors `Process.readLoop` in the Go runtime.
    fn spawn_read_loop(
        manifest_id: String,
        stdout: tokio::process::ChildStdout,
        pending: PendingMap,
        on_notify: NotifyHandler,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(plugin = %manifest_id, "stdout read error: {e}");
                        break;
                    }
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Quick peek: response has "id", notification has "method".
                let parsed: Result<Value, _> = serde_json::from_str(trimmed);
                let v = match parsed {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            plugin = %manifest_id,
                            error = %e,
                            line = trimmed,
                            "plugin emitted invalid JSON, dropping"
                        );
                        continue;
                    }
                };
                let id_opt = v.get("id").and_then(|x| x.as_i64());
                let has_method = v.get("method").and_then(|x| x.as_str()).is_some();
                if let Some(id) = id_opt {
                    // Response — push to pending channel if any.
                    let resp: Response = match serde_json::from_value(v) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(plugin = %manifest_id, id, "malformed response: {e}");
                            continue;
                        }
                    };
                    let mut map = pending.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let _ = tx.send(resp);
                    }
                    // If not in map: either unknown id or already
                    // timed out — drop silently.
                } else if has_method {
                    // Notification — fire callback if registered.
                    let notif = Notification {
                        jsonrpc: "2.0",
                        method: v
                            .get("method")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        params: v.get("params").cloned(),
                    };
                    let cb = on_notify.lock().await;
                    if let Some(f) = cb.as_ref() {
                        f(notif);
                    }
                } else {
                    tracing::warn!(
                        plugin = %manifest_id,
                        line = trimmed,
                        "message has neither id nor method, dropping"
                    );
                }
            }
            // EOF reached — fail every still-pending request so
            // awaiting callers don't hang forever.
            let mut map = pending.lock().await;
            for (_id, tx) in map.drain() {
                let _ = tx.send(Response {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(RpcError {
                        code: -1,
                        message: "plugin stdout closed".into(),
                    }),
                    id: 0,
                });
            }
        })
    }
}

#[async_trait]
impl PluginProcess for Subprocess {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    async fn start(&self) -> Result<(), PluginError> {
        let mut cmd = Command::new(&self.manifest.command);
        cmd.args(&self.manifest.args)
            .current_dir(&self.manifest.dir)
            .envs(std::env::vars())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(PluginError::NotConnected)?;
        let stdout = child.stdout.take().ok_or(PluginError::NotConnected)?;

        // Spawn the read loop BEFORE the state slot, sharing the
        // `pending` map and the notify-handler slot via Arc.
        let manifest_id = self.manifest.id.clone();
        let pending_for_loop = Arc::clone(&self.pending);
        let on_notify_for_loop = Arc::clone(&self.on_notify);
        let read_task =
            Self::spawn_read_loop(manifest_id, stdout, pending_for_loop, on_notify_for_loop);

        *self.state.lock().await = Some(SubprocessState {
            child,
            stdin,
            _read_task: read_task,
        });

        // Send initialize.
        let params = serde_json::json!({
            "pluginId": self.manifest.id,
            "version": self.manifest.version,
        });
        let _ = self.call(METHOD_INITIALIZE, params).await?;
        Ok(())
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, PluginError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request {
            jsonrpc: "2.0",
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
            id,
        };
        let line = serde_json::to_string(&req)? + "\n";

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        {
            let mut state = self.state.lock().await;
            let state = state.as_mut().ok_or(PluginError::NotConnected)?;
            state.stdin.write_all(line.as_bytes()).await?;
            state.stdin.flush().await?;
        }
        let resp = rx
            .await
            .map_err(|_| PluginError::Exited("channel closed".into()))?;
        if let Some(e) = resp.error {
            return Err(PluginError::Rpc {
                code: e.code,
                message: e.message,
            });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), PluginError> {
        let line = serde_json::to_string(&Notification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        })? + "\n";
        let mut state = self.state.lock().await;
        let state = state.as_mut().ok_or(PluginError::NotConnected)?;
        state.stdin.write_all(line.as_bytes()).await?;
        state.stdin.flush().await?;
        Ok(())
    }

    async fn stop(&self) -> Result<(), PluginError> {
        let _ = self.call(METHOD_SHUTDOWN, Value::Null).await;
        let mut state = self.state.lock().await;
        if let Some(mut s) = state.take() {
            let _ = s.child.start_kill();
            let _ = s.child.wait().await;
        }
        Ok(())
    }
}

// =====================================================================
// Manager — index of loaded plugins.
// =====================================================================

pub struct Manager {
    plugins: Mutex<HashMap<String, Arc<dyn PluginProcess>>>,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            plugins: Mutex::new(HashMap::new()),
        }
    }

    pub async fn register(&self, plugin: Arc<dyn PluginProcess>) {
        let id = plugin.id().to_string();
        self.plugins.lock().await.insert(id, plugin);
    }

    pub async fn load_from_dir(&self, dir: &std::path::Path) -> Result<(), PluginError> {
        let entries = std::fs::read_dir(dir)?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name().and_then(|s| s.to_str()) == Some("plugin.json") {
                let m = Manifest::from_file(&p)?;
                let id = m.id.clone();
                let proc: Arc<dyn PluginProcess> = Arc::new(Subprocess::new(m));
                proc.start().await?;
                self.plugins.lock().await.insert(id.clone(), proc);
                tracing::info!(id = %id, dir = %p.display(), "plugin loaded");
            }
        }
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Option<Arc<dyn PluginProcess>> {
        self.plugins.lock().await.get(id).cloned()
    }

    pub async fn ids(&self) -> Vec<String> {
        self.plugins.lock().await.keys().cloned().collect()
    }

    pub async fn shutdown_all(&self) {
        for (_id, plugin) in self.plugins.lock().await.drain() {
            let _ = plugin.stop().await;
        }
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Adapter — exposes a plugin's tools as an LLM-callable set.
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(default)]
    pub text: String,
    #[serde(default, rename = "isError")]
    pub is_error: bool,
}

#[async_trait]
pub trait ToolAdapter: Send + Sync {
    async fn list_tools(&self, plugin_id: &str) -> Result<Vec<ToolDef>, PluginError>;
    async fn execute(
        &self,
        plugin_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<ToolResult, PluginError>;
}

pub struct DefaultToolAdapter {
    manager: Arc<Manager>,
}

impl DefaultToolAdapter {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl ToolAdapter for DefaultToolAdapter {
    async fn list_tools(&self, plugin_id: &str) -> Result<Vec<ToolDef>, PluginError> {
        let plugin = self
            .manager
            .get(plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
        let v = plugin.call(METHOD_TOOL_LIST, Value::Null).await?;
        let tools: Vec<ToolDef> = serde_json::from_value(v)?;
        Ok(tools)
    }
    async fn execute(
        &self,
        plugin_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<ToolResult, PluginError> {
        let plugin = self
            .manager
            .get(plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
        let v = plugin
            .call(
                METHOD_TOOL_EXECUTE,
                serde_json::json!({"name": tool, "args": args}),
            )
            .await?;
        let r: ToolResult = serde_json::from_value(v)?;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_from_str_parses() {
        let blob = r#"{
            "id": "github",
            "name": "GitHub",
            "version": "1.0",
            "type": "tool",
            "command": "gh-plugin",
            "args": ["--serve"]
        }"#;
        let m: Manifest = serde_json::from_str(blob).unwrap();
        assert_eq!(m.id, "github");
        assert_eq!(m.plugin_type, PluginType::Tool);
        assert_eq!(m.args, vec!["--serve".to_string()]);
    }

    #[test]
    fn manifest_with_config_schema() {
        let blob = r#"{
            "id": "x",
            "name": "X",
            "type": "channel",
            "command": "x",
            "config": {
                "apiKey": {"type": "string", "required": true, "sensitive": true}
            }
        }"#;
        let m: Manifest = serde_json::from_str(blob).unwrap();
        assert!(m.config_schema.contains_key("apiKey"));
        assert!(m.config_schema["apiKey"].sensitive);
    }

    #[test]
    fn plugin_type_variants_distinct() {
        let js = serde_json::to_string(&PluginType::Channel).unwrap();
        assert_eq!(js, "\"channel\"");
        let js = serde_json::to_string(&PluginType::Tool).unwrap();
        assert_eq!(js, "\"tool\"");
    }

    #[test]
    fn request_serializes_with_id() {
        let r = Request {
            jsonrpc: "2.0",
            method: "tool.list".into(),
            params: None,
            id: 42,
        };
        let blob = serde_json::to_string(&r).unwrap();
        assert!(blob.contains("\"id\":42"));
        assert!(blob.contains("\"method\":\"tool.list\""));
    }

    #[test]
    fn response_unwraps_result() {
        let blob = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
        let r: Response = serde_json::from_str(blob).unwrap();
        assert_eq!(r.result, Some(serde_json::json!({"ok": true})));
        assert!(r.error.is_none());
    }

    #[test]
    fn response_surfaces_error() {
        let blob = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"nope"}}"#;
        let r: Response = serde_json::from_str(blob).unwrap();
        assert_eq!(r.error.as_ref().unwrap().code, -1);
    }

    #[test]
    fn notification_skips_id() {
        let n = Notification {
            jsonrpc: "2.0",
            method: "ready".into(),
            params: None,
        };
        let blob = serde_json::to_string(&n).unwrap();
        assert!(!blob.contains("\"id\""));
    }

    #[test]
    fn rpc_error_display() {
        let e = RpcError {
            code: -32601,
            message: "method not found".into(),
        };
        assert_eq!(e.to_string(), "rpc error -32601: method not found");
    }

    #[tokio::test]
    async fn manager_register_and_get() {
        struct Stub;
        #[async_trait]
        impl PluginProcess for Stub {
            fn id(&self) -> &str {
                "stub"
            }
            async fn start(&self) -> Result<(), PluginError> {
                Ok(())
            }
            async fn call(&self, _m: &str, _p: Value) -> Result<Value, PluginError> {
                Ok(Value::Null)
            }
            async fn notify(&self, _m: &str, _p: Value) -> Result<(), PluginError> {
                Ok(())
            }
            async fn stop(&self) -> Result<(), PluginError> {
                Ok(())
            }
        }
        let m = Manager::new();
        let s: Arc<dyn PluginProcess> = Arc::new(Stub);
        m.register(s.clone()).await;
        let got = m.get("stub").await;
        assert!(got.is_some());
        let ids = m.ids().await;
        assert_eq!(ids, vec!["stub".to_string()]);
    }

    #[tokio::test]
    async fn tool_def_round_trip() {
        let t = ToolDef {
            name: "search".into(),
            description: "search the web".into(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let blob = serde_json::to_string(&t).unwrap();
        let back: ToolDef = serde_json::from_str(&blob).unwrap();
        assert_eq!(back.name, "search");
    }

    #[tokio::test]
    async fn tool_result_default_text_empty() {
        let r = ToolResult::default();
        assert_eq!(r.text, "");
        assert!(!r.is_error);
    }

    #[test]
    fn method_constants_distinct() {
        let all = [
            METHOD_INITIALIZE,
            METHOD_SHUTDOWN,
            METHOD_TOOL_LIST,
            METHOD_TOOL_EXECUTE,
            METHOD_PROVIDER_LIST,
            METHOD_PROVIDER_EXECUTE,
            METHOD_HOOK_FIRE,
        ];
        let mut seen = std::collections::HashSet::new();
        for m in all {
            assert!(seen.insert(m), "duplicate method constant {m}");
        }
    }

    /// End-to-end: spawn a real subprocess that speaks JSON-RPC,
    /// prove that `call()` actually receives a response (the
    /// previous version would hang forever here).
    #[tokio::test]
    async fn subprocess_call_round_trip() {
        // A shell that, for every incoming JSON-RPC request,
        // echoes a response with the same `id`. After the SECOND
        // request (which is the test's `ping` call, made AFTER
        // the handler is registered) it emits a notification.
        // This proves the read loop dispatches BOTH responses
        // (to the pending oneshot) AND notifications (to the
        // callback), and that the order is preserved.
        let script = r#"#!/bin/bash
count=0
while IFS= read -r line; do
  count=$((count+1))
  id=$(echo "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"ok\":true,\"echoed_id\":$id}}"
  if [ $count -ge 2 ]; then
    echo '{"jsonrpc":"2.0","method":"ready","params":{}}'
  fi
done
# keep stdin open a bit so we don't race stop()
sleep 1
"#;
        let dir = std::env::temp_dir().join(format!(
            "cc-plugin-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = dir.join("fake_plugin.sh");
        std::fs::write(&script_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let manifest = Manifest {
            id: "fake".into(),
            name: "Fake".into(),
            version: "0.0.1".into(),
            description: "test".into(),
            plugin_type: PluginType::Tool,
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            capabilities: vec![],
            config_schema: HashMap::new(),
            dir: dir.clone(),
        };
        let sub = Subprocess::new(manifest);
        sub.start().await.expect("start");

        // Drain notifications on a side channel.
        let (ntx, mut nrx) = tokio::sync::mpsc::unbounded_channel::<String>();
        sub.set_notify_handler(move |n| {
            let _ = ntx.send(n.method);
        })
        .await;

        // The second call returns a different id, easier to assert.
        let r = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            sub.call("ping", serde_json::json!({"x": 1})),
        )
        .await
        .expect("call timed out — read loop broken?")
        .expect("call ok");
        assert_eq!(r.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert!(r.get("echoed_id").is_some());

        // Notification should have arrived too.
        let method = tokio::time::timeout(std::time::Duration::from_secs(2), nrx.recv())
            .await
            .expect("notification timed out")
            .expect("notification channel open");
        assert_eq!(method, "ready");

        sub.stop().await.expect("stop");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// End-to-end: fire a hook through the real `HookAdapter`
    /// against a real subprocess that records the inbound
    /// `hook.fire` call. This is the regression test for the
    /// P8 stub — before P8, `fire()` would silently drop the
    /// request and the subprocess would never see it.
    #[tokio::test]
    async fn hook_adapter_fire_against_real_subprocess() {
        // The fake plugin writes every incoming JSON-RPC line to a
        // file in its working dir, then echoes a `result: null`
        // response so the call returns. We read the file after
        // `fire()` and assert the hook was actually delivered.
        let dir = std::env::temp_dir().join(format!(
            "cc-plugin-hook-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("requests.log");
        let script = format!(
            r#"#!/bin/bash
LOG="{log_path}"
touch "$LOG"
while IFS= read -r line; do
  echo "$line" >> "$LOG"
  id=$(echo "$line" | sed -n 's/.*"id":\([0-9-]*\).*/\1/p')
  echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}}"
done
# keep stdin open a bit so we don't race stop()
sleep 1
"#,
            log_path = log_path.to_string_lossy()
        );
        let script_path = dir.join("hook_target.sh");
        std::fs::write(&script_path, &script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let manifest = Manifest {
            id: "hook_target".into(),
            name: "HookTarget".into(),
            version: "0.0.1".into(),
            description: "test".into(),
            plugin_type: PluginType::Hook,
            command: script_path.to_string_lossy().into_owned(),
            args: vec![],
            capabilities: vec![],
            config_schema: HashMap::new(),
            dir: dir.clone(),
        };
        let manager = Arc::new(Manager::new());
        let proc: Arc<dyn PluginProcess> = Arc::new(Subprocess::new(manifest));
        proc.start().await.expect("start");
        manager.register(proc).await;

        // Fire the hook — this used to be the P8 stub.
        let adapter = HookAdapter::new(manager.clone());
        let n = adapter
            .fire("post_turn", serde_json::json!({"chatId": "c1"}))
            .await;
        assert_eq!(n, 1, "hook.fire should have been delivered");

        // Drain + stop the subprocess so the log file is flushed.
        manager.shutdown_all().await;
        // Give the bash subprocess a tick to exit.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // The log file must contain a `hook.fire` request with
        // `point: "post_turn"` and the merged chatId payload.
        let log = std::fs::read_to_string(&log_path).expect("log file written");
        assert!(log.contains("\"method\":\"hook.fire\""), "log: {log}");
        assert!(log.contains("\"point\":\"post_turn\""), "log: {log}");
        assert!(log.contains("\"chatId\":\"c1\""), "log: {log}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// =====================================================================
// Per-adapter protocols. Mirrors
// .
// =====================================================================

/// Hook point names — kept snake_case on the wire so plugin authors
/// don't have to deal with Rust's `PascalCase` `HookPoint` enum.
pub const HOOK_BEFORE_MODEL_CALL: &str = "before_model_call";
pub const HOOK_AFTER_MODEL_CALL: &str = "after_model_call";
pub const HOOK_BEFORE_TOOL_CALL: &str = "before_tool_call";
pub const HOOK_AFTER_TOOL_CALL: &str = "after_tool_call";
pub const HOOK_POST_TURN: &str = "post_turn";

/// Map a snake_case protocol hook point name back to a
/// numeric `HookPoint` (1..=5). Unknown names return `None`.
pub fn hook_point_from_name(name: &str) -> Option<u32> {
    match name {
        HOOK_BEFORE_MODEL_CALL => Some(1),
        HOOK_AFTER_MODEL_CALL => Some(2),
        HOOK_BEFORE_TOOL_CALL => Some(3),
        HOOK_AFTER_TOOL_CALL => Some(4),
        HOOK_POST_TURN => Some(5),
        _ => None,
    }
}

/// HookAdapter — fire a hook on every loaded plugin and collect
/// the responses. Mirrors `plugin/hook_adapter.go::Fire`.
pub struct HookAdapter {
    pub manager: Arc<Manager>,
}

/// Default per-plugin timeout for a single `hook.fire` RPC. A slow
/// plugin must never block the agent loop beyond this window.
pub const DEFAULT_HOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl HookAdapter {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self { manager }
    }

    /// Fire `hook_point_name` on every plugin. Returns the number
    /// of plugins that responded successfully (i.e. the RPC
    /// completed before the timeout). Errors and timeouts are
    /// logged + counted as fired-but-errored; the agent loop never
    /// blocks on a misbehaving plugin.
    //
    /// The wire format is `hook.fire` with params
    /// `{"point": "<name>", ...payload}`. This matches what
    /// `cleanclaw_plugin_runtime::Plugin::hook_fire` already
    /// receives (the runtime passes the full params object
    /// through, including the `point` discriminator).
    pub async fn fire(&self, hook_point_name: &str, payload: Value) -> usize {
        self.fire_with_timeout(hook_point_name, payload, DEFAULT_HOOK_TIMEOUT)
            .await
    }

    /// Same as [`fire`](Self::fire) but lets the caller override
    /// the per-plugin timeout. The default is
    /// [`DEFAULT_HOOK_TIMEOUT`] (5 seconds).
    pub async fn fire_with_timeout(
        &self,
        hook_point_name: &str,
        payload: Value,
        timeout: std::time::Duration,
    ) -> usize {
        let ids = self.manager.ids().await;
        // Build the params once — all plugins see the same shape.
        let mut params_map = serde_json::Map::new();
        params_map.insert("point".into(), Value::String(hook_point_name.to_string()));
        if let Value::Object(p) = payload {
            for (k, v) in p {
                if k != "point" {
                    params_map.insert(k, v);
                }
            }
        }
        let params = Value::Object(params_map);
        let mut fired = 0usize;
        for id in ids {
            let Some(plugin) = self.manager.get(&id).await else {
                continue;
            };
            let method = "hook.fire".to_string();
            let params_c = params.clone();
            let result = tokio::time::timeout(timeout, plugin.call(&method, params_c)).await;
            match result {
                Ok(Ok(_)) => fired += 1,
                Ok(Err(e)) => {
                    tracing::warn!(plugin = %id, point = %hook_point_name, "hook fire failed: {e}");
                }
                Err(_) => {
                    tracing::warn!(
                        plugin = %id,
                        point = %hook_point_name,
                        timeout_ms = timeout.as_millis() as u64,
                        "hook fire timed out"
                    );
                }
            }
        }
        fired
    }
}

/// ProviderAdapter — invoke a tool-provider slot (e.g. "web_search")
/// on a specific plugin.
pub struct ProviderAdapter {
    pub manager: Arc<Manager>,
}

impl ProviderAdapter {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self { manager }
    }

    /// List the provider slots a plugin claims to fill.
    pub async fn list_providers(&self, plugin_id: &str) -> Result<Value, PluginError> {
        let plugin = self
            .manager
            .get(plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
        plugin.call(METHOD_PROVIDER_LIST, Value::Null).await
    }

    /// Invoke a provider slot on the plugin. Mirrors
    /// `provider.execute` in the Go side.
    pub async fn execute_provider(
        &self,
        plugin_id: &str,
        category: &str,
        name: &str,
        args: Value,
    ) -> Result<Value, PluginError> {
        let plugin = self
            .manager
            .get(plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
        let params = serde_json::json!({
            "category": category,
            "name": name,
            "args": args,
        });
        plugin.call(METHOD_PROVIDER_EXECUTE, params).await
    }
}

/// ToolAdapter — wrap a plugin's `tool.list` / `tool.execute` to
/// look like a `cleanclaw_agent::tools::Tool`. Mirrors
/// `plugin/tool_adapter.go`.
pub struct PluginToolAdapter {
    pub manager: Arc<Manager>,
}

impl PluginToolAdapter {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self { manager }
    }

    pub async fn list_tools(&self, plugin_id: &str) -> Result<Vec<Value>, PluginError> {
        let plugin = self
            .manager
            .get(plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
        let v = plugin.call(METHOD_TOOL_LIST, Value::Null).await?;
        let arr = v.as_array().cloned().unwrap_or_default();
        Ok(arr)
    }

    pub async fn execute_tool(
        &self,
        plugin_id: &str,
        name: &str,
        args: Value,
    ) -> Result<Value, PluginError> {
        let plugin = self
            .manager
            .get(plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
        let params = serde_json::json!({"name": name, "args": args});
        plugin.call(METHOD_TOOL_EXECUTE, params).await
    }
}

/// ChannelAdapter — wrap a plugin as a `Channel`. The plugin
/// receives `channel.send` JSON-RPC calls; the adapter exposes
/// just the `key` / `name` accessors.
pub struct ChannelAdapter {
    pub manager: Arc<Manager>,
    pub plugin_id: String,
}

impl ChannelAdapter {
    pub fn new(manager: Arc<Manager>, plugin_id: impl Into<String>) -> Self {
        Self {
            manager,
            plugin_id: plugin_id.into(),
        }
    }

    pub fn key(&self) -> String {
        format!("plugin:{}", self.plugin_id)
    }

    pub fn name(&self) -> String {
        self.plugin_id.clone()
    }

    /// Send a message via the plugin. The plugin is expected to
    /// implement the `channel.send` method.
    pub async fn send(&self, msg: OutboundMessage) -> Result<Value, PluginError> {
        let plugin = self
            .manager
            .get(&self.plugin_id)
            .await
            .ok_or_else(|| PluginError::NotFound(self.plugin_id.clone()))?;
        let params = serde_json::to_value(&msg)
            .map_err(|e| PluginError::Other(format!("serialize outbound: {e}")))?;
        plugin.call("channel.send", params).await
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use crate::Manager;

    #[test]
    fn hook_point_from_name_known_values() {
        assert_eq!(hook_point_from_name(HOOK_BEFORE_MODEL_CALL), Some(1));
        assert_eq!(hook_point_from_name(HOOK_AFTER_TOOL_CALL), Some(4));
        assert_eq!(hook_point_from_name(HOOK_POST_TURN), Some(5));
    }

    #[test]
    fn hook_point_from_name_unknown_returns_none() {
        assert!(hook_point_from_name("nope").is_none());
        assert!(hook_point_from_name("").is_none());
    }

    #[test]
    fn channel_adapter_key_format() {
        let m = Arc::new(Manager::new());
        let a = ChannelAdapter::new(m, "feishu");
        assert_eq!(a.key(), "plugin:feishu");
        assert_eq!(a.name(), "feishu");
    }

    #[test]
    fn hook_adapter_construction() {
        let m = Arc::new(Manager::new());
        let h = HookAdapter::new(m);
        // Just verify it constructs; fire() needs a real plugin.
        let _ = h;
    }

    // -----------------------------------------------------------------
    // P8: real dispatch tests for HookAdapter::fire. Each test wires
    // a stub `PluginProcess` into the Manager that records (or
    // delays) the incoming `hook.fire` call so the adapter's actual
    // call path is exercised end-to-end.
    // -----------------------------------------------------------------

    /// A test plugin that captures every `call()` invocation so we
    /// can assert on the params the adapter actually sends.
    struct CapturingPlugin {
        id: String,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        delay: std::time::Duration,
    }

    #[async_trait]
    impl PluginProcess for CapturingPlugin {
        fn id(&self) -> &str {
            &self.id
        }
        async fn start(&self) -> Result<(), PluginError> {
            Ok(())
        }
        async fn call(&self, method: &str, params: Value) -> Result<Value, PluginError> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.calls.lock().await.push((method.to_string(), params));
            Ok(Value::Null)
        }
        async fn notify(&self, _m: &str, _p: Value) -> Result<(), PluginError> {
            Ok(())
        }
        async fn stop(&self) -> Result<(), PluginError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn fire_calls_each_plugin_with_hook_fire_method() {
        let calls_a: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_b: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let m = Arc::new(Manager::new());
        m.register(Arc::new(CapturingPlugin {
            id: "alpha".into(),
            calls: calls_a.clone(),
            delay: std::time::Duration::ZERO,
        }))
        .await;
        m.register(Arc::new(CapturingPlugin {
            id: "beta".into(),
            calls: calls_b.clone(),
            delay: std::time::Duration::ZERO,
        }))
        .await;
        let h = HookAdapter::new(m);
        let n = h
            .fire(
                "post_turn",
                serde_json::json!({"chatId": "c1", "channel": "telegram"}),
            )
            .await;
        assert_eq!(n, 2);
        // Both plugins received the call with method="hook.fire" and
        // params containing point="post_turn" + the original payload.
        for calls in [&calls_a, &calls_b] {
            let v = calls.lock().await;
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].0, "hook.fire");
            assert_eq!(v[0].1.get("point").unwrap(), "post_turn");
            assert_eq!(v[0].1.get("chatId").unwrap(), "c1");
            assert_eq!(v[0].1.get("channel").unwrap(), "telegram");
        }
    }

    #[tokio::test]
    async fn fire_merges_point_into_payload_without_overwriting() {
        // If the caller passes a `point` field in the payload, the
        // adapter's `point` (the hook name) wins — the caller's
        // value is dropped.
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let m = Arc::new(Manager::new());
        m.register(Arc::new(CapturingPlugin {
            id: "p".into(),
            calls: calls.clone(),
            delay: std::time::Duration::ZERO,
        }))
        .await;
        let h = HookAdapter::new(m);
        h.fire(
            "after_tool_call",
            serde_json::json!({"point": "wrong", "tool": "echo"}),
        )
        .await;
        let v = calls.lock().await;
        assert_eq!(v[0].1.get("point").unwrap(), "after_tool_call");
        assert_eq!(v[0].1.get("tool").unwrap(), "echo");
    }

    #[tokio::test]
    async fn fire_times_out_on_slow_plugin() {
        // A plugin that sleeps longer than the timeout must not
        // block the loop; the adapter counts it as fired-but-errored
        // and continues.
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let m = Arc::new(Manager::new());
        m.register(Arc::new(CapturingPlugin {
            id: "slow".into(),
            calls: calls.clone(),
            delay: std::time::Duration::from_millis(500),
        }))
        .await;
        let h = HookAdapter::new(m);
        let n = h
            .fire_with_timeout(
                "post_turn",
                Value::Null,
                std::time::Duration::from_millis(50),
            )
            .await;
        // 0 because the timeout fired before the plugin's call returned.
        assert_eq!(n, 0);
        // The plugin never got to record the call.
        assert!(calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn fire_continues_past_plugin_error() {
        /// Plugin that always errors.
        struct ErrorPlugin;
        #[async_trait]
        impl PluginProcess for ErrorPlugin {
            fn id(&self) -> &str {
                "err"
            }
            async fn start(&self) -> Result<(), PluginError> {
                Ok(())
            }
            async fn call(&self, _m: &str, _p: Value) -> Result<Value, PluginError> {
                Err(PluginError::Other("boom".into()))
            }
            async fn notify(&self, _m: &str, _p: Value) -> Result<(), PluginError> {
                Ok(())
            }
            async fn stop(&self) -> Result<(), PluginError> {
                Ok(())
            }
        }
        let m = Arc::new(Manager::new());
        m.register(Arc::new(ErrorPlugin)).await;
        let h = HookAdapter::new(m);
        let n = h.fire("post_turn", Value::Null).await;
        // The erroring plugin doesn't count, but the loop survives.
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn fire_with_no_plugins_returns_zero() {
        let m = Arc::new(Manager::new());
        let h = HookAdapter::new(m);
        let n = h.fire("post_turn", Value::Null).await;
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn fire_with_null_payload_still_includes_point() {
        // A null payload (e.g. the caller has nothing to attach) must
        // still produce a `{ "point": "..." }` params object so the
        // plugin can dispatch on it.
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let m = Arc::new(Manager::new());
        m.register(Arc::new(CapturingPlugin {
            id: "p".into(),
            calls: calls.clone(),
            delay: std::time::Duration::ZERO,
        }))
        .await;
        let h = HookAdapter::new(m);
        h.fire("post_turn", Value::Null).await;
        let v = calls.lock().await;
        assert_eq!(v[0].1.get("point").unwrap(), "post_turn");
        // The params object has exactly the one key.
        assert_eq!(v[0].1.as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fire_default_timeout_is_five_seconds() {
        // The DEFAULT_HOOK_TIMEOUT constant is what the agent loop
        // relies on. Pin it so a refactor doesn't accidentally make
        // a hook hang for minutes.
        assert_eq!(DEFAULT_HOOK_TIMEOUT, std::time::Duration::from_secs(5));
    }
}
