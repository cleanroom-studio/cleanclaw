//! Integration tests for the W1 server. Boots the axum router on an
//! ephemeral port, sends HTTP requests with `hyper`, and asserts on
//! the response shape.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn pick_port() -> std::net::SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    addr
}

async fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, String, String) {
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
    let ctype = raw
        .lines()
        .find_map(|l| {
            let lo = l.to_ascii_lowercase();
            if lo.starts_with("content-type:") {
                Some(l[13..].trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    (status, body, ctype)
}

#[tokio::test]
async fn root_returns_landing_page() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, ctype) = http_get(addr, "/").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(ctype.contains("text/html"));
    assert!(body.contains("CleanClaw"));
}

#[tokio::test]
async fn overview_returns_dashboard() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, _) = http_get(addr, "/overview").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("Overview"));
    assert!(body.contains("Ada Lovelace"));
}

#[tokio::test]
async fn favicon_returns_png() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, _, ctype) = http_get(addr, "/favicon.ico").await;
    h.abort();
    assert_eq!(status, 200);
    // The handler serves the on-disk favicon.ico (image/x-icon)
    // when present, falling back to a 1×1 PNG otherwise. Accept
    // either — the P2-9 asset mount ships the real .ico.
    assert!(
        ctype.contains("image/png") || ctype.contains("image/x-icon"),
        "unexpected content-type: {ctype}"
    );
}

#[tokio::test]
async fn theme_query_switches_dark() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, _) = http_get(addr, "/overview?theme=dark").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains(r#"class="dark""#));
}

#[tokio::test]
async fn login_page_renders() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, _) = http_get(addr, "/login").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("Sign in"));
    assert!(body.contains(r#"action="/login""#));
}

#[tokio::test]
async fn signup_page_renders() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, _) = http_get(addr, "/signup").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("Create an account"));
}

#[tokio::test]
async fn settings_general_renders() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    for (path, expect) in [
        ("/settings/general", "Provider"),
        ("/settings/account", "Display name"),
        ("/settings/runtime", "Enable sandbox"),
        ("/settings/about", "Version"),
    ] {
        let (status, body, _) = http_get(addr, path).await;
        assert_eq!(status, 200, "{path} should be 200");
        assert!(body.contains(expect), "{path} should contain {expect}");
    }
    h.abort();
}

#[tokio::test]
async fn admin_pages_render() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    for (path, expect) in [
        ("/admin/users", "Users"),
        ("/admin/usage", "No usage data"),
        ("/admin/chats", "Chats"),
    ] {
        let (status, body, _) = http_get(addr, path).await;
        assert_eq!(status, 200, "{path} should be 200");
        assert!(body.contains(expect), "{path} should contain {expect}");
    }
    h.abort();
}

#[tokio::test]
async fn apikeys_page_renders() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, _) = http_get(addr, "/apikeys").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("API keys"));
    assert!(body.contains("New key"));
}

#[tokio::test]
async fn not_found_returns_404() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, _, _) = http_get(addr, "/this-does-not-exist").await;
    h.abort();
    assert_eq!(status, 404);
}

#[tokio::test]
async fn agents_list_renders() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let (status, body, _) = http_get(addr, "/agents").await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("Agents"));
    assert!(body.contains("New agent"));
}

#[tokio::test]
async fn agent_sub_routes_render() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    let id = "agent_xyz";
    for (path, expect) in [
        ("/agents/agent_xyz", "Overview"),
        ("/agents/agent_xyz/chat", "Open chat"),
        ("/agents/agent_xyz/chats", "Chats"),
        ("/agents/agent_xyz/sessions", "Sessions"),
        ("/agents/agent_xyz/channels", "Telegram"),
        ("/agents/agent_xyz/scheduler", "Scheduler"),
        ("/agents/agent_xyz/skills", "No skills installed"),
        ("/agents/agent_xyz/plugins", "Plugins"),
        ("/agents/agent_xyz/models", "No agent config available"),
        ("/agents/agent_xyz/context", "Context"),
        ("/agents/agent_xyz/customize", "Customize"),
        ("/agents/agent_xyz/project", "No projects yet"),
        ("/agents/agent_xyz/project/proj_1", "proj_1"),
        ("/agents/agent_xyz/usage", "No usage data in this range"),
    ] {
        let (status, body, _) = http_get(addr, path).await;
        assert_eq!(status, 200, "{path} should be 200");
        assert!(body.contains(expect), "{path} should contain {expect}");
    }
    // Avoid unused-variable warning.
    let _ = id;
    h.abort();
}

#[tokio::test]
async fn resources_pages_render() {
    let addr = pick_port().await;
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let state = cleanclaw_web::server::WebState::new(tx);
    let app = cleanclaw_web::server::full_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = cleanclaw_web::server::wait_for_ready(addr, Duration::from_secs(2)).await;
    for (path, expect) in [
        ("/channels", "Channels"),
        ("/channels-config", "Channel configuration"),
        ("/models", "No models registered"),
        ("/providers", "No providers configured"),
        ("/plugins", "No plugins installed"),
        ("/skills", "No skills installed"),
        ("/tools", "No tools configured"),
        ("/cron", "No cron jobs"),
        ("/onboard", "Welcome to CleanClaw"),
    ] {
        let (status, body, _) = http_get(addr, path).await;
        assert_eq!(status, 200, "{path} should be 200");
        assert!(body.contains(expect), "{path} should contain {expect}");
    }
    h.abort();
}
