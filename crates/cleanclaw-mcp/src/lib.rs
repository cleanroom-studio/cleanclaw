//! Model Context Protocol (MCP) client. Mirrors
//! .
//!
//! Supports two transports:
//!   - `StdioClient` — spawns a subprocess and speaks JSON-RPC over
//!     its stdin/stdout.
//!   - `HttpClient` — JSON-RPC over HTTP (the Streamable HTTP transport).
//!
//! `Manager` indexes tools by `<server>__<tool>` so the agent runtime
//! can call them through a single namespace.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("server returned error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("connection closed")]
    Closed,
    #[error("not connected")]
    NotConnected,
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("server not found: {0}")]
    UnknownServer(String),
    #[error("config: {0}")]
    Config(String),
}

/// One tool returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

/// Response from a tool call. Mirrors Go's `toolCallResult.content[]`
/// (currently we only surface text content).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Client: Send + Sync {
    async fn connect(&self) -> Result<(), McpError>;
    async fn list_tools(&self) -> Result<Vec<ToolDef>, McpError>;
    async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult, McpError>;
    async fn close(&self) -> Result<(), McpError>;
}

// =====================================================================
// JSON-RPC plumbing (shared by stdio + http clients).
// =====================================================================

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Deserialize, Default)]
struct InitializeResult {
    #[serde(default, rename = "protocolVersion")]
    #[allow(dead_code)]
    protocol_version: String,
}

#[derive(Serialize)]
struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    protocol_version: &'static str,
    capabilities: Value,
    #[serde(rename = "clientInfo")]
    client_info: ClientInfo,
}

#[derive(Serialize)]
struct ClientInfo {
    name: &'static str,
    version: &'static str,
}

#[derive(Deserialize)]
struct ToolsListResult {
    tools: Vec<ToolDef>,
}

#[derive(Serialize)]
struct ToolCallParams<'a> {
    name: &'a str,
    #[serde(rename = "arguments")]
    arguments: Value,
}

#[derive(Deserialize)]
struct ToolCallResultWire {
    content: Vec<ToolContent>,
    #[serde(default, rename = "isError")]
    is_error: bool,
}

#[derive(Deserialize)]
struct ToolContent {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    #[serde(default)]
    text: String,
}

fn rpc_initialize() -> JsonRpcRequest<'static> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: 0,
        method: "initialize",
        params: serde_json::to_value(InitializeParams {
            protocol_version: "2024-11-05",
            capabilities: serde_json::json!({}),
            client_info: ClientInfo {
                name: "cleanclaw",
                version: env!("CARGO_PKG_VERSION"),
            },
        })
        .unwrap(),
    }
}

fn rpc_list_tools(id: u64) -> JsonRpcRequest<'static> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id,
        method: "tools/list",
        params: serde_json::json!({}),
    }
}

fn rpc_call_tool(id: u64, name: &str, args: Value) -> JsonRpcRequest<'_> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id,
        method: "tools/call",
        params: serde_json::to_value(ToolCallParams { name, arguments: args })
            .unwrap(),
    }
}

fn parse_rpc_response(raw: Value) -> Result<Value, McpError> {
    let resp: JsonRpcResponse = serde_json::from_value(raw)?;
    if let Some(e) = resp.error {
        return Err(McpError::Rpc {
            code: e.code,
            message: e.message,
        });
    }
    Ok(resp.result.unwrap_or(Value::Null))
}

// =====================================================================
// StdioClient — spawn subprocess, JSON-RPC over stdin/stdout.
// =====================================================================

pub struct StdioClient {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    state: Mutex<Option<StdioState>>,
    next_id: AtomicU64,
}

struct StdioState {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
}

impl StdioClient {
    pub fn new(
        command: impl Into<String>,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            env,
            state: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }
}

#[async_trait]
impl Client for StdioClient {
    async fn connect(&self) -> Result<(), McpError> {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args)
            .envs(self.env.iter())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(McpError::NotConnected)?;
        let stdout = child.stdout.take().ok_or(McpError::NotConnected)?;
        let mut state = StdioState {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };

        // Send initialize.
        let init = rpc_initialize();
        let line = serde_json::to_string(&init)? + "\n";
        state.stdin.write_all(line.as_bytes()).await?;
        state.stdin.flush().await?;
        // Read one line of response.
        let mut buf = String::new();
        state.stdout.read_line(&mut buf).await?;
        let _: InitializeResult = serde_json::from_str::<JsonRpcResponse>(&buf)
            .ok()
            .and_then(|r| r.result)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        // Send the initialized notification (no id).
        let notify = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let line = serde_json::to_string(&notify)? + "\n";
        state.stdin.write_all(line.as_bytes()).await?;
        state.stdin.flush().await?;

        *self.state.lock().await = Some(state);
        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<ToolDef>, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = rpc_list_tools(id);
        let raw = self.send_and_recv(&req).await?;
        let v: ToolsListResult = serde_json::from_value(raw)?;
        Ok(v.tools)
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = rpc_call_tool(id, name, args);
        let raw = self.send_and_recv(&req).await?;
        let res: ToolCallResultWire = serde_json::from_value(raw)?;
        let text = res
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");
        Ok(ToolResult {
            text,
            is_error: res.is_error,
        })
    }

    async fn close(&self) -> Result<(), McpError> {
        let mut state = self.state.lock().await;
        if let Some(mut s) = state.take() {
            let _ = s.child.start_kill();
            let _ = s.child.wait().await;
        }
        Ok(())
    }
}

impl StdioClient {
    async fn send_and_recv(&self, req: &JsonRpcRequest<'_>) -> Result<Value, McpError> {
        let mut state = self.state.lock().await;
        let state = state.as_mut().ok_or(McpError::NotConnected)?;
        let line = serde_json::to_string(req)? + "\n";
        state.stdin.write_all(line.as_bytes()).await?;
        state.stdin.flush().await?;
        let mut buf = String::new();
        let n = state.stdout.read_line(&mut buf).await?;
        if n == 0 {
            return Err(McpError::Closed);
        }
        let v: Value = serde_json::from_str(&buf)?;
        parse_rpc_response(v)
    }
}

// =====================================================================
// HttpClient — JSON-RPC over Streamable HTTP.
// =====================================================================

pub struct HttpClient {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    state: Mutex<Option<()>>,
    next_id: AtomicU64,
}

impl HttpClient {
    pub fn new(url: impl Into<String>, headers: HashMap<String, String>) -> Self {
        let mut expanded = HashMap::new();
        for (k, v) in headers {
            // Expand $ENV references.
            let v = expand_env(&v);
            expanded.insert(k, v);
        }
        Self {
            url: url.into(),
            headers: expanded,
            client: reqwest::Client::new(),
            state: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }
}

fn expand_env(s: &str) -> String {
    if let Some(rest) = s.strip_prefix('$') {
        if let Ok(val) = std::env::var(rest) {
            return val;
        }
    }
    s.to_string()
}

#[async_trait]
impl Client for HttpClient {
    async fn connect(&self) -> Result<(), McpError> {
        let init = rpc_initialize();
        let resp = self
            .client
            .post(&self.url)
            .headers(header_map(&self.headers))
            .json(&init)
            .send()
            .await
            .map_err(|e| McpError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(McpError::Http(format!("status {}", resp.status())));
        }
        let raw: Value = resp.json().await.map_err(|e| McpError::Http(e.to_string()))?;
        let _ = parse_rpc_response(raw)?;
        *self.state.lock().await = Some(());
        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<ToolDef>, McpError> {
        if self.state.lock().await.is_none() {
            return Err(McpError::NotConnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = rpc_list_tools(id);
        let resp = self
            .client
            .post(&self.url)
            .headers(header_map(&self.headers))
            .json(&req)
            .send()
            .await
            .map_err(|e| McpError::Http(e.to_string()))?;
        let raw: Value = resp.json().await.map_err(|e| McpError::Http(e.to_string()))?;
        let v = parse_rpc_response(raw)?;
        let v: ToolsListResult = serde_json::from_value(v)?;
        Ok(v.tools)
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult, McpError> {
        if self.state.lock().await.is_none() {
            return Err(McpError::NotConnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = rpc_call_tool(id, name, args);
        let resp = self
            .client
            .post(&self.url)
            .headers(header_map(&self.headers))
            .json(&req)
            .send()
            .await
            .map_err(|e| McpError::Http(e.to_string()))?;
        let raw: Value = resp.json().await.map_err(|e| McpError::Http(e.to_string()))?;
        let v = parse_rpc_response(raw)?;
        let res: ToolCallResultWire = serde_json::from_value(v)?;
        let text = res
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");
        Ok(ToolResult {
            text,
            is_error: res.is_error,
        })
    }

    async fn close(&self) -> Result<(), McpError> {
        *self.state.lock().await = None;
        Ok(())
    }
}

fn header_map(headers: &HashMap<String, String>) -> HeaderMap {
    let mut hm = HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            hm.insert(name, value);
        }
    }
    hm
}

// =====================================================================
// Manager — per-(user, agent) index of MCP servers + tools.
// =====================================================================

pub struct Manager {
    servers: HashMap<String, Arc<dyn Client>>,
    /// `<server>__<tool>` → (server, original tool name).
    tool_map: HashMap<String, ToolRoute>,
}

#[derive(Debug, Clone)]
struct ToolRoute {
    server: String,
    original: String,
}

impl Manager {
    /// Connect to every configured server. Failures are logged but
    /// don't block startup (matches Go's behavior).
    pub async fn connect_all(servers: &HashMap<String, McpServerConfig>) -> Self {
        let mut m = Self {
            servers: HashMap::new(),
            tool_map: HashMap::new(),
        };
        for (name, cfg) in servers {
            let client: Arc<dyn Client> = match cfg {
                McpServerConfig::Stdio { command, args, env } => {
                    Arc::new(StdioClient::new(command, args.to_vec(), env.clone()))
                }
                McpServerConfig::Http { url, headers } => {
                    Arc::new(HttpClient::new(url, headers.clone()))
                }
            };
            if let Err(e) = client.connect().await {
                tracing::warn!(server = %name, error = %e, "mcp connect failed; skipping");
                continue;
            }
            let tools = match client.list_tools().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(server = %name, error = %e, "mcp list_tools failed; skipping");
                    let _ = client.close().await;
                    continue;
                }
            };
            for t in &tools {
                let prefixed = prefixed_tool_name(name, &t.name);
                m.tool_map.insert(
                    prefixed,
                    ToolRoute {
                        server: name.clone(),
                        original: t.name.clone(),
                    },
                );
            }
            tracing::info!(server = %name, count = tools.len(), "mcp server connected");
            m.servers.insert(name.clone(), client);
        }
        m
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tool_map.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn server_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.servers.keys().cloned().collect();
        v.sort();
        v
    }

    pub async fn call(&self, prefixed: &str, args: Value) -> Result<ToolResult, McpError> {
        let route = self
            .tool_map
            .get(prefixed)
            .ok_or_else(|| McpError::UnknownTool(prefixed.to_string()))?;
        let server = self
            .servers
            .get(&route.server)
            .ok_or_else(|| McpError::UnknownServer(route.server.clone()))?;
        server.call_tool(&route.original, args).await
    }

    pub async fn close_all(&self) {
        for (_name, server) in &self.servers {
            let _ = server.close().await;
        }
    }
}

pub fn prefixed_tool_name(server: &str, tool: &str) -> String {
    format!("{server}__{tool}")
}

#[derive(Debug, Clone)]
pub enum McpServerConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn prefixed_tool_name_formats_correctly() {
        assert_eq!(prefixed_tool_name("github", "list_repos"), "github__list_repos");
    }

    #[test]
    fn tool_def_round_trip() {
        let t = ToolDef {
            name: "search".into(),
            description: "search the web".into(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let blob = serde_json::to_string(&t).unwrap();
        let back: ToolDef = serde_json::from_str(&blob).unwrap();
        assert_eq!(back.name, "search");
        assert_eq!(back.description, "search the web");
    }

    #[test]
    fn tool_result_text_and_error() {
        let t = ToolResult::from_text("ok");
        assert_eq!(t.text, "ok");
        assert!(!t.is_error);
        let e = ToolResult::error("nope");
        assert!(e.is_error);
    }

    #[test]
    fn rpc_initialize_serializes_correctly() {
        let r = rpc_initialize();
        assert_eq!(r.method, "initialize");
        assert_eq!(r.jsonrpc, "2.0");
    }

    #[test]
    fn rpc_call_tool_serializes_arguments() {
        let r = rpc_call_tool(7, "search", serde_json::json!({"q": "rust"}));
        assert_eq!(r.method, "tools/call");
        assert_eq!(r.id, 7);
    }

    #[test]
    fn parse_rpc_response_unwraps_result() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"ok": true}
        });
        let v = parse_rpc_response(raw).unwrap();
        assert_eq!(v, serde_json::json!({"ok": true}));
    }

    #[test]
    fn parse_rpc_response_surfaces_error() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32601, "message": "method not found"}
        });
        match parse_rpc_response(raw) {
            Err(McpError::Rpc { code, .. }) => assert_eq!(code, -32601),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    #[test]
    fn expand_env_resolves_known_var() {
        std::env::set_var("CLEANCLAW_TEST_EXPAND", "hello");
        assert_eq!(expand_env("$CLEANCLAW_TEST_EXPAND"), "hello");
        std::env::remove_var("CLEANCLAW_TEST_EXPAND");
    }

    #[test]
    fn expand_env_passes_through_unknown() {
        assert_eq!(expand_env("plain-value"), "plain-value");
    }

    #[tokio::test]
    async fn http_client_connect_to_bad_url_errors() {
        let client = HttpClient::new(
            "http://127.0.0.1:1/nonexistent",
            HashMap::new(),
        );
        let err = client.connect().await;
        assert!(matches!(err, Err(McpError::Http(_))));
    }

    #[tokio::test]
    async fn http_client_list_without_connect_errors() {
        let client = HttpClient::new("http://x", HashMap::new());
        let err = client.list_tools().await;
        assert!(matches!(err, Err(McpError::NotConnected)));
    }

    #[tokio::test]
    async fn http_client_close_clears_state() {
        let client = HttpClient::new("http://x", HashMap::new());
        *client.state.lock().await = Some(());
        client.close().await.unwrap();
        let err = client.list_tools().await;
        assert!(matches!(err, Err(McpError::NotConnected)));
    }

    #[tokio::test]
    async fn manager_with_no_servers_is_empty() {
        let m = Manager::connect_all(&HashMap::new()).await;
        assert!(m.tool_names().is_empty());
        assert!(m.server_names().is_empty());
    }

    #[tokio::test]
    async fn manager_call_unknown_tool_errors() {
        let m = Manager::connect_all(&HashMap::new()).await;
        let err = m.call("nope__x", serde_json::json!({})).await;
        assert!(matches!(err, Err(McpError::UnknownTool(_))));
    }

    #[test]
    fn server_config_variants_distinct() {
        let s = McpServerConfig::Stdio {
            command: "npx".into(),
            args: vec![],
            env: HashMap::new(),
        };
        let h = McpServerConfig::Http {
            url: "https://x".into(),
            headers: HashMap::new(),
        };
        assert!(matches!(s, McpServerConfig::Stdio { .. }));
        assert!(matches!(h, McpServerConfig::Http { .. }));
    }
}
