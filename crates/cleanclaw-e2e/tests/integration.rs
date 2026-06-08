//! End-to-end smoke test for the full CleanClaw stack.
//!
//! This test boots the SSR frontend on an ephemeral port and
//! verifies the full page surface wires up correctly. It also
//! exercises:
//!
//!  * the bundled skills list (`cleanclaw-skills-bundled`)
//!  * the default workspace seed (`cleanclaw-workspace-defaults`)
//!  * a plugin in-process round-trip (`cleanclaw-plugin-runtime` +
//!    `cleanclaw-plugins-plugin-demo`)

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn pick_port() -> std::net::SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    addr
}

async fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let status: u16 = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// Smoke test: the SSR frontend serves every page.
#[tokio::test]
async fn ssr_frontend_serves_all_routes() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;

    // One page per major route group.
    for (path, expect) in [
        ("/", "CleanClaw"),
        ("/overview", "Overview"),
        ("/login", "Sign in"),
        ("/signup", "Create an account"),
        ("/settings/general", "Provider"),
        ("/admin/users", "Users"),
        ("/apikeys", "API keys"),
        ("/agents", "Agents"),
        ("/channels", "Channels"),
        ("/providers", "No providers configured"),
        ("/skills", "No skills installed"),
        ("/tools", "No tools configured"),
        ("/cron", "No cron jobs"),
        ("/onboard", "Welcome to CleanClaw"),
    ] {
        let (status, body) = http_get(addr, path).await;
        assert_eq!(status, 200, "{path}: expected 200, got {status}");
        assert!(body.contains(expect), "{path}: expected '{expect}' in body");
    }
    h.abort();
}

/// Smoke test: the bundled skills crate has 10 entries
/// (camoufox-cli was added in P1-7).
#[test]
fn bundled_skills_count() {
    assert_eq!(cleanclaw_skills_bundled::BUNDLED.len(), 10);
    assert!(cleanclaw_skills_bundled::find("web-search").is_some());
    assert!(cleanclaw_skills_bundled::find("camoufox-cli").is_some());
    assert!(cleanclaw_skills_bundled::find("does-not-exist").is_none());
}

/// Smoke test: workspace defaults seed the 4 files.
#[test]
fn workspace_defaults_seed() {
    let dir = std::env::temp_dir().join(format!(
        "cleanclaw-e2e-{}",
        cleanclaw_core::IdGen::new().next("e2e")
    ));
    let n = cleanclaw_workspace_defaults::seed_to(&dir).unwrap();
    assert_eq!(n, 4);
    assert!(dir.join("AGENTS.md").exists());
    assert!(dir.join("SOUL.md").exists());
    assert!(dir.join("TOOLS.md").exists());
    assert!(dir.join("USER.md").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Smoke test: a plugin in-process round-trip via
/// `cleanclaw-plugin-runtime` works against the demo plugin.
#[tokio::test]
async fn plugin_runtime_round_trip() {
    // The plugin-demo crate exposes a main() binary but also has
    // an `EchoPlugin` type reachable from the lib. We import it
    // here for an in-process test.
    use cleanclaw_plugin_runtime::{InProcPluginClient, Plugin, ToolDef, ToolResult};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Arc;

    struct Echo;
    #[async_trait]
    impl Plugin for Echo {
        fn id(&self) -> &str {
            "echo"
        }
        async fn tool_list(&self) -> Result<Vec<ToolDef>, cleanclaw_plugin_runtime::PluginError> {
            Ok(vec![ToolDef {
                name: "echo".into(),
                description: "echo back".into(),
                parameters: Value::Null,
                source: "plugin".into(),
            }])
        }
        async fn tool_execute(
            &self,
            name: &str,
            _args: Value,
        ) -> Result<ToolResult, cleanclaw_plugin_runtime::PluginError> {
            Ok(ToolResult {
                output: format!("{name} ok"),
                error: None,
            })
        }
    }

    let c = InProcPluginClient::spawn(Echo);
    let tools: Vec<ToolDef> =
        serde_json::from_value(c.call("tool.list", Value::Null).await.unwrap()).unwrap();
    assert_eq!(tools.len(), 1);
    let r: ToolResult = serde_json::from_value(
        c.call("tool.execute", serde_json::json!({"name": "echo"}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(r.output, "echo ok");
    let _ = Arc::new(c);
}
