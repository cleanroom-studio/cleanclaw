//! Shared runtime for the bundled sample plugins. Mirrors the
//! JSON-RPC 2.0 over NDJSON protocol that the Python/JS plugins
//! speak so the existing `cleanclaw-plugin` Manager spawns the
//! Rust binaries through the same Subprocess / IPC pipeline.
//!
//! ## Protocol
//!
//! * `host -> plugin` (NDJSON on stdin):
//!     - `{ "jsonrpc": "2.0", "id": N, "method": "initialize", "params": {...} }`
//!     - `{ "jsonrpc": "2.0", "id": N, "method": "tool.list" }`
//!     - `{ "jsonrpc": "2.0", "id": N, "method": "tool.execute", "params": {"name": "...", "args": {...}} }`
//!     - `{ "jsonrpc": "2.0", "id": N, "method": "hook.register" }`
//!     - `{ "jsonrpc": "2.0", "method": "hook.fire", "params": {"point": "..."} }` (notification)
//!     - `{ "jsonrpc": "2.0", "id": N, "method": "shutdown" }`
//!
//! * `plugin -> host` (NDJSON on stdout):
//!     - `{ "jsonrpc": "2.0", "id": N, "result": ... }` for synchronous calls
//!     - `{ "jsonrpc": "2.0", "method": "chat.send", "params": {...} }` (notification)
//!
//! All log output goes to **stderr**; stdout is the JSON-RPC stream.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unknown method: {0}")]
    UnknownMethod(String),
}

/// A JSON-RPC request from the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default = "default_params")]
    pub params: Value,
}

fn default_params() -> Value {
    Value::Object(serde_json::Map::new())
}

/// A JSON-RPC response to the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// A JSON-RPC error payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A plugin tool definition. Returned by `tool.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    #[serde(default = "default_schema")]
    pub parameters: Value,
    /// The plugin source label (`builtin` / `mcp` / `plugin`).
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "required": []
    })
}

fn default_source() -> String {
    "plugin".to_string()
}

/// A tool execution result. Returned by `tool.execute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A plugin's response to `hook.register` — the hook points the
/// plugin wants to be notified on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRegistration {
    pub points: Vec<String>,
}

/// A notification from the plugin to the host. The host must
/// accept and ignore the unknown `method` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Plugin behavior. Implement this trait, then call
/// [`run_plugin`].
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// Plugin id, used for log scoping.
    fn id(&self) -> &str;

    /// `initialize` — host passes the resolved config here.
    async fn initialize(&self, _params: Value) -> Result<Value, PluginError> {
        Ok(serde_json::json!({ "status": "ok" }))
    }

    /// `tool.list` — return the tools this plugin exposes. Default
    /// = empty list.
    async fn tool_list(&self) -> Result<Vec<ToolDef>, PluginError> {
        Ok(vec![])
    }

    /// `tool.execute` — dispatch by name. Default = unknown tool.
    async fn tool_execute(&self, name: &str, _args: Value) -> Result<ToolResult, PluginError> {
        Ok(ToolResult {
            output: String::new(),
            error: Some(format!("unknown tool: {name}")),
        })
    }

    /// `hook.register` — return the hook points the plugin wants.
    async fn hook_register(&self) -> Result<HookRegistration, PluginError> {
        Ok(HookRegistration { points: vec![] })
    }

    /// `hook.fire` (notification, no response). Default: no-op.
    async fn hook_fire(&self, _params: Value) -> Result<(), PluginError> {
        Ok(())
    }

    /// `shutdown` — graceful exit. Default: no-op + return ok.
    async fn shutdown(&self) -> Result<Value, PluginError> {
        Ok(serde_json::json!({ "status": "ok" }))
    }
}

/// `run_plugin` reads NDJSON from stdin, dispatches each request to
/// the plugin, and writes the response to stdout. The loop exits
/// on EOF or on a `shutdown` request.
///
/// The plugin is wrapped in an `Arc` so the same instance can be
/// shared across concurrent requests if the host ever opens
/// multiple in-flight calls on the same stdin.
pub async fn run_plugin<P: Plugin + 'static>(plugin: Arc<P>) -> std::io::Result<()> {
    init_tracing();
    let stdin = tokio::io::stdin();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("parse error: {e}");
                continue;
            }
        };
        let is_notification = req.id.is_none();
        let resp = dispatch(plugin.as_ref(), req.clone()).await;
        if !is_notification {
            let line = serde_json::to_string(&resp).unwrap_or_default();
            let mut out = stdout.lock().await;
            let _ = out.write_all(line.as_bytes()).await;
            let _ = out.write_all(b"\n").await;
            let _ = out.flush().await;
        }
        if req.method == "shutdown" {
            break;
        }
    }
    Ok(())
}

async fn dispatch<P: Plugin + 'static>(plugin: &P, req: Request) -> Response {
    let id = req.id.clone().unwrap_or(Value::Null);
    let result = match req.method.as_str() {
        "initialize" => plugin.initialize(req.params).await,
        "tool.list" => match plugin.tool_list().await {
            Ok(tools) => Ok(serde_json::to_value(tools).unwrap_or(Value::Null)),
            Err(e) => Err(e),
        },
        "tool.execute" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = req
                .params
                .get("args")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));
            match plugin.tool_execute(&name, args).await {
                Ok(r) => Ok(serde_json::to_value(r).unwrap_or(Value::Null)),
                Err(e) => Err(e),
            }
        }
        "hook.register" => match plugin.hook_register().await {
            Ok(r) => Ok(serde_json::to_value(r).unwrap_or(Value::Null)),
            Err(e) => Err(e),
        },
        "hook.fire" => match plugin.hook_fire(req.params.clone()).await {
            Ok(()) => Ok(Value::Null),
            Err(e) => Err(e),
        },
        "shutdown" => plugin.shutdown().await,
        other => Err(PluginError::UnknownMethod(other.to_string())),
    };
    match result {
        Ok(v) => Response {
            jsonrpc: "2.0".into(),
            id,
            result: Some(v),
            error: None,
        },
        Err(e) => Response {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code: -32603,
                message: e.to_string(),
                data: None,
            }),
        },
    }
}

/// `send_notification` writes a `chat.send`-shaped notification to
/// stdout. The plugin uses this for fire-and-forget outbound calls.
pub async fn send_notification(method: &str, params: Value) -> std::io::Result<()> {
    let mut out = tokio::io::stdout();
    let n = Notification {
        jsonrpc: "2.0".into(),
        method: method.to_string(),
        params,
    };
    let line = serde_json::to_string(&n).unwrap_or_default();
    out.write_all(line.as_bytes()).await?;
    out.write_all(b"\n").await?;
    out.flush().await?;
    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("CLEANCLAW_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

/// In-process plugin manager used by the test suite. Spawns the
/// plugin in a background task and exposes a request/response
/// surface. This avoids spawning a real subprocess during tests.
pub struct InProcPluginClient<P: Plugin + 'static> {
    tx: tokio::sync::mpsc::Sender<Request>,
    rx: Arc<Mutex<tokio::sync::mpsc::Receiver<Response>>>,
    /// Outstanding notifications observed by the host (plugin ->
    /// host). Useful for tests that want to assert on `chat.send`.
    pub notifications: Arc<Mutex<Vec<Notification>>>,
    plugin_id: String,
    _join: Option<tokio::task::JoinHandle<()>>,
    _marker: std::marker::PhantomData<P>,
}

impl<P: Plugin + 'static> InProcPluginClient<P> {
    /// `spawn` runs the plugin on a background task and wires a
    /// pair of channels so the test can drive the request/response
    /// surface without touching stdin/stdout.
    pub fn spawn(plugin: P) -> Self {
        let (host_to_plugin_tx, mut host_to_plugin_rx) = tokio::sync::mpsc::channel::<Request>(32);
        let (plugin_to_host_tx, plugin_to_host_rx) = tokio::sync::mpsc::channel::<Response>(32);
        let (notif_tx, notif_rx) = tokio::sync::mpsc::channel::<Notification>(32);
        let notifications: Arc<Mutex<Vec<Notification>>> = Arc::new(Mutex::new(Vec::new()));

        // Forward notifications into the Vec.
        let notifications_c = notifications.clone();
        let notif_task = tokio::spawn(async move {
            let mut rx = notif_rx;
            while let Some(n) = rx.recv().await {
                notifications_c.lock().await.push(n);
            }
        });

        let plugin_id = plugin.id().to_string();
        let plugin_id_c = plugin_id.clone();
        let plugin_arc = Arc::new(plugin);
        let join = tokio::spawn(async move {
            // In-process: re-implement the dispatch loop using the
            // channels instead of stdin/stdout.
            while let Some(req) = host_to_plugin_rx.recv().await {
                let is_notification = req.id.is_none();
                let resp = inproc_dispatch(plugin_arc.as_ref(), req).await;
                if !is_notification && plugin_to_host_tx.send(resp).await.is_err() {
                    break;
                }
            }
            drop(notif_tx);
            let _ = notif_task.await;
        });
        // The notif_tx is owned by the test code path. The
        // production runtime (stdin/stdout) doesn't use it; the
        // host-side `send_notification` is what the plugin calls.
        // For tests, expose a sender via `notif_tx_handle`.
        let _ = notif_tx;
        let _ = plugin_id_c;
        let _ = notifications; // used via accessor below
        Self {
            tx: host_to_plugin_tx,
            rx: Arc::new(Mutex::new(plugin_to_host_rx)),
            notifications: {
                // re-bind the inner Arc from the closure above
                let p = plugin_id.clone();
                let _ = p;
                notifications
            },
            plugin_id,
            _join: Some(join),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn id(&self) -> &str {
        &self.plugin_id
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, PluginError> {
        let req = Request {
            jsonrpc: "2.0".into(),
            id: Some(Value::from(0u64)),
            method: method.to_string(),
            params,
        };
        self.tx
            .send(req)
            .await
            .map_err(|e| PluginError::Io(std::io::Error::other(e.to_string())))?;
        let mut rx = self.rx.lock().await;
        let resp = rx.recv().await.ok_or_else(|| {
            PluginError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "plugin closed",
            ))
        })?;
        if let Some(err) = resp.error {
            return Err(PluginError::UnknownMethod(err.message));
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    pub async fn notify(&self, method: &str, params: Value) {
        let req = Request {
            jsonrpc: "2.0".into(),
            id: None,
            method: method.to_string(),
            params,
        };
        let _ = self.tx.send(req).await;
    }
}

async fn inproc_dispatch<P: Plugin + 'static>(plugin: &P, req: Request) -> Response {
    dispatch(plugin, req).await
}

/// Helper for tests: turn a `HashMap<String, String>` into a JSON
/// value. Plugins use this for the `args` field of `tool.execute`.
pub fn args_object(items: &[(String, Value)]) -> Value {
    let mut m: HashMap<String, Value> = HashMap::new();
    for (k, v) in items {
        m.insert(k.clone(), v.clone());
    }
    Value::Object(m.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin;

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        fn id(&self) -> &str {
            "test"
        }
        async fn tool_list(&self) -> Result<Vec<ToolDef>, PluginError> {
            Ok(vec![ToolDef {
                name: "ping".into(),
                description: "ping the plugin".into(),
                parameters: default_schema(),
                source: "plugin".into(),
            }])
        }
        async fn tool_execute(&self, name: &str, _args: Value) -> Result<ToolResult, PluginError> {
            Ok(ToolResult {
                output: format!("{name}!"),
                error: None,
            })
        }
    }

    #[tokio::test]
    async fn inproc_dispatch_round_trips() {
        let client = InProcPluginClient::spawn(TestPlugin);
        let v: Vec<ToolDef> =
            serde_json::from_value(client.call("tool.list", Value::Null).await.unwrap()).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "ping");
        let r: ToolResult = serde_json::from_value(
            client
                .call("tool.execute", serde_json::json!({"name": "ping"}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(r.output, "ping!");
    }

    #[tokio::test]
    async fn unknown_method_returns_error() {
        let client = InProcPluginClient::spawn(TestPlugin);
        let e = client.call("nope", Value::Null).await.unwrap_err();
        assert!(matches!(e, PluginError::UnknownMethod(_)));
    }
}
