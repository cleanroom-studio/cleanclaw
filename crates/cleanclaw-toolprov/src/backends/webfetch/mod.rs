//! Built-in URL-fetch backends.
//!
//! Two providers:
//!
//!   * [`Direct`] — plain `GET` with a recognisable User-Agent.
//!     Always available; `credential_free` so the dashboard can
//!     pick it without an API key.
//!   * [`Jina`]   — Jina Reader (`https://r.jina.ai/<url>`) which
//!     returns cleaned markdown.
use crate::ProviderError;

/// Registry category key.
pub const CATEGORY: &str = "web_fetch";

mod direct;
mod jina;

pub use direct::Direct;
pub use jina::Jina;

/// Parse the LLM-supplied args blob. URL fetch takes a single
/// `url` string.
pub(crate) fn parse_args(raw: &serde_json::Value) -> Result<String, ProviderError> {
    let url = raw
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return Err(ProviderError::InvalidArgs("url is required".into()));
    }
    Ok(url)
}
