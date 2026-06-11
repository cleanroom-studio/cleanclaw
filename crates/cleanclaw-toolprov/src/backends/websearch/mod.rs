//! Built-in web-search backends.
//!
//! Providers:
//!
//!   * [`DuckDuckGo`] — scrapes DDG's HTML lite endpoint. No key
//!     required. Used as the default primary so the dashboard
//!     works out-of-the-box.
//!   * [`Brave`]     — `api.search.brave.com` JSON API.
//!   * [`Bing`]      — Microsoft Bing Web Search v7.
//!   * [`Google`]    — Google Programmable Search Engine (Custom
//!     Search JSON API). Reads the engine id from the
//!     `endpoint` config field (`cx=<engine-id>`).
//!   * [`Baidu`]     — HTML scrape of `baidu.com/s`. No key.
//!   * [`None`]      — chain terminator.
use crate::ProviderError;

/// Registry category key.
pub const CATEGORY: &str = "web_search";

mod baidu;
mod bing;
mod brave;
mod duckduckgo;
mod google;
mod helpers;
mod none;

pub use baidu::Baidu;
pub use bing::Bing;
pub use brave::Brave;
pub use duckduckgo::DuckDuckGo;
pub use google::Google;
pub use none::None;

// Re-export the shared byte-level helpers so the per-provider
// files can use them through `super::`.
pub(crate) use helpers::{decode_html_entities, find_from, strip_tags};

/// Parse the LLM-supplied args blob into a normalized
/// `(query, n)` pair. `n` is clamped to `1..=20`.
pub fn parse_args(raw: &serde_json::Value) -> Result<(String, usize), ProviderError> {
    let query = raw
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if query.is_empty() {
        return Err(ProviderError::InvalidArgs("query is required".into()));
    }
    let n = raw.get("n").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    Ok((query, n.clamp(1, 20)))
}
