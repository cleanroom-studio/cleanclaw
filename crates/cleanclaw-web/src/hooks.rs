//! Server-side equivalents of the React hooks in
//! . In an SSR app these become
//! request-time extractors / helpers rather than client-side state.
//!
//! | React hook                  | Server equivalent                  |
//! |-----------------------------|------------------------------------|
//! | `useAgentId()`              | `extract_agent_id(&Path)`          |
//! | `useAgentName(id)`          | `resolve_agent_name(client, id)`  |
//! | `useMobile()`               | `mobile_viewport(headers)`         |
//!
//! The hooks don't persist state between requests — they read the
//! request and return a value.

use axum::http::HeaderMap;

/// Extract the `:id` capture from a path of the form
/// `/agents/{id}/...`. The id may be URL-encoded; this returns the
/// decoded form.
pub fn extract_agent_id(path: &str) -> Option<String> {
    // Path shapes:
    //   /agents/{id}
    //   /agents/{id}/<tab>
    //   /agents/{id}/project/{pid}
    let rest = path.strip_prefix("/agents/")?;
    let mut parts = rest.split('/');
    let id = parts.next()?;
    if id.is_empty() {
        return None;
    }
    Some(crate::client::urlencode_decode(id))
}

/// Best-effort agent display name lookup. Returns `Some(name)` when
/// the agent exists and has a `name`; otherwise the id. The caller
/// passes a typed `ApiClient` so this can hit the real backend
/// (`GET /api/agents/{id}`); the W7 default is to return the id
/// since most callers fall back to the same value.
pub fn resolve_agent_name(id: &str, fetched_name: Option<&str>) -> String {
    fetched_name.unwrap_or(id).to_string()
}

/// `useMobile` equivalent. Returns `true` when the request advertises
/// a viewport narrower than 768px (the `md` Tailwind breakpoint).
/// SSR pages collapse the sidebar in that case.
pub fn mobile_viewport(headers: &HeaderMap) -> bool {
    // The `Sec-CH-UA-Mobile` client hint is the most reliable signal.
    if let Some(v) = headers.get("sec-ch-ua-mobile") {
        if let Ok(s) = v.to_str() {
            if s == "?1" {
                return true;
            }
        }
    }
    // Fallback: parse the `Viewport-Width` request header (rare
    // but explicit).
    if let Some(v) = headers.get("viewport-width") {
        if let Ok(s) = v.to_str() {
            if let Ok(w) = s.parse::<u32>() {
                return w < 768;
            }
        }
    }
    // Final fallback: a user-agent sniff for "Mobile" tokens.
    if let Some(ua) = headers.get("user-agent") {
        if let Ok(s) = ua.to_str() {
            let s = s.to_ascii_lowercase();
            if s.contains("mobile") || s.contains("android") || s.contains("iphone") {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_agent_id_simple() {
        assert_eq!(extract_agent_id("/agents/abc"), Some("abc".to_string()));
    }

    #[test]
    fn extract_agent_id_with_tab() {
        assert_eq!(extract_agent_id("/agents/abc/chat"), Some("abc".to_string()));
    }

    #[test]
    fn extract_agent_id_with_project() {
        assert_eq!(extract_agent_id("/agents/abc/project/p1"), Some("abc".to_string()));
    }

    #[test]
    fn extract_agent_id_rejects_other_paths() {
        assert_eq!(extract_agent_id("/agents"), None);
        assert_eq!(extract_agent_id("/overview"), None);
    }

    #[test]
    fn extract_agent_id_decodes() {
        // %20 = space
        assert_eq!(extract_agent_id("/agents/ada%20bot/chat"), Some("ada bot".to_string()));
    }

    #[test]
    fn resolve_agent_name_prefers_fetched() {
        assert_eq!(resolve_agent_name("a1", Some("Ada")), "Ada");
        assert_eq!(resolve_agent_name("a1", None), "a1");
    }

    #[test]
    fn mobile_viewport_mobile_hint() {
        let mut h = HeaderMap::new();
        h.insert("sec-ch-ua-mobile", "?1".parse().unwrap());
        assert!(mobile_viewport(&h));
    }

    #[test]
    fn mobile_viewport_desktop_hint() {
        let mut h = HeaderMap::new();
        h.insert("sec-ch-ua-mobile", "?0".parse().unwrap());
        assert!(!mobile_viewport(&h));
    }

    #[test]
    fn mobile_viewport_user_agent_fallback() {
        let mut h = HeaderMap::new();
        h.insert("user-agent", "Mozilla/5.0 (iPhone)".parse().unwrap());
        assert!(mobile_viewport(&h));
    }

    #[test]
    fn mobile_viewport_user_agent_desktop() {
        let mut h = HeaderMap::new();
        h.insert("user-agent", "Mozilla/5.0 (X11; Linux x86_64)".parse().unwrap());
        assert!(!mobile_viewport(&h));
    }

    #[test]
    fn mobile_viewport_empty_headers() {
        let h = HeaderMap::new();
        assert!(!mobile_viewport(&h));
    }
}
