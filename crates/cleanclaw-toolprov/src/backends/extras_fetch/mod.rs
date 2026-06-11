//! "Extras" URL-fetch backends.
//!
//! Providers:
//!
//!   * [`Firecrawl`] — `POST https://api.firecrawl.dev/v1/scrape`.
//!     Returns the page as cleaned markdown.
use crate::Registry;

mod firecrawl;

pub use firecrawl::Firecrawl;

pub(crate) fn str_field<'a>(args: &'a serde_json::Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

pub fn register(reg: &Registry, client: &reqwest::Client) {
    reg.register(std::sync::Arc::new(Firecrawl::new(client.clone())));
}
