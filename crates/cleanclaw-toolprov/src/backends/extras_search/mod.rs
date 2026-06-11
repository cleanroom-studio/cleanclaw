//! "Extras" web-search backends.
//!
//! Providers:
//!
//!   * [`Exa`]    — `POST https://api.exa.ai/search`. Neural
//!     search results.
//!   * [`SearXNG`] — `<endpoint>/search?q=...&format=json`. No
//!     auth. Self-hosted; the operator must supply the instance
//!     URL via the `endpoint` config field (`needs_endpoint`).
use crate::Registry;

mod exa;
mod searxng;

pub use exa::Exa;
pub use searxng::SearXNG;

pub(crate) fn str_field<'a>(args: &'a serde_json::Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Tiny URL-encoder for the `q` parameter on SearXNG. Avoids
/// pulling in the `urlencoding` crate for a single call site.
pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

pub fn register(reg: &Registry, client: &reqwest::Client) {
    reg.register(std::sync::Arc::new(Exa::new(client.clone())));
    reg.register(std::sync::Arc::new(SearXNG::new(client.clone())));
}
