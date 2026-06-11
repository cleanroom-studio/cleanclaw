//! DuckDuckGo HTML "lite" search backend.
//!
//! `credential_free`: no key, but DDG throttles anonymous scrapers
//! aggressively. The chain transparently falls through to the next
//! provider if this one returns 403/429/empty.
use async_trait::async_trait;

use super::{decode_html_entities, find_from, parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

/// DuckDuckGo HTML search — `credential_free` (no key).
/// Scrapes `https://html.duckduckgo.com/html/?q=...` (the
/// "lite" endpoint) and returns the top `n` results. Used as
/// the default primary so the dashboard works out-of-the-box
/// even when no paid search API is configured.
pub struct DuckDuckGo {
    client: reqwest::Client,
}

impl DuckDuckGo {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Provider for DuckDuckGo {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "duckduckgo"
    }
    fn credential_free(&self) -> bool {
        true
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let (query, n) = parse_args(&req.args)?;
        // DDG's "lite" HTML endpoint requires a UA and the
        // POST form. We send as POST so the q= is in the body,
        // matching what a browser would do.
        let resp = self
            .client
            .post("https://html.duckduckgo.com/html/")
            .header("User-Agent", "Mozilla/5.0 (compatible; CleanClaw/0.1)")
            .form(&[("q", query.as_str())])
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!(
                "duckduckgo {status}: {txt}"
            )));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        // Cheap HTML extraction: `<a class="result__a" href="…" rel="noopener">title</a>`
        // followed by `.result__snippet`. The class names are
        // stable across DDG's HTML lite endpoint.
        let mut out = String::new();
        out.push_str(&format!("Search results for: {query}\n\n"));
        let mut idx = 0;
        let bytes = html.as_bytes();
        let needle = b"class=\"result__a\"";
        let mut cursor = 0usize;
        while idx < n && cursor < bytes.len() {
            if let Some(pos) = find_from(&bytes[cursor..], needle) {
                cursor += pos + needle.len();
                // Walk to the closing `>` of the opening tag.
                if let Some(end_tag) = bytes[cursor..].iter().position(|&b| b == b'>') {
                    cursor += end_tag + 1;
                }
                // Read up to `</a>` for the title.
                if let Some(end_a) = find_from(&bytes[cursor..], b"</a>") {
                    let title = decode_html_entities(
                        std::str::from_utf8(&bytes[cursor..cursor + end_a])
                            .unwrap_or("")
                            .trim(),
                    );
                    // URL: walk back from the title's anchor
                    // to find `href="…"` on the same line.
                    let url_start = bytes[..cursor + end_a]
                        .windows(5)
                        .rposition(|w| w == b"href=\"")
                        .map(|p| p + 6)
                        .unwrap_or(cursor);
                    let url_end = bytes[url_start..]
                        .iter()
                        .position(|&b| b == b'"')
                        .unwrap_or(0);
                    let url = decode_html_entities(
                        std::str::from_utf8(&bytes[url_start..url_start + url_end])
                            .unwrap_or("")
                            .trim(),
                    );
                    // Snippet: optional `<a class="result__snippet"…>…</a>`.
                    let snip: Option<String> = (|| -> Option<String> {
                        let sn_start =
                            find_from(&bytes[cursor + end_a..], b"class=\"result__snippet\"")?;
                        let abs = cursor + end_a + sn_start;
                        let end_s = bytes[abs..].iter().position(|&b| b == b'>')?;
                        let s = abs + end_s + 1;
                        let e = find_from(&bytes[s..], b"</a>")?;
                        Some(decode_html_entities(
                            std::str::from_utf8(&bytes[s..s + e]).unwrap_or("").trim(),
                        ))
                    })();
                    idx += 1;
                    out.push_str(&format!(
                        "{}. {}\n   {}\n{}\n\n",
                        idx,
                        if title.is_empty() {
                            "(no title)"
                        } else {
                            &title
                        },
                        if url.is_empty() { "(no url)" } else { &url },
                        snip.unwrap_or_default(),
                    ));
                    cursor += end_a + 4;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if idx == 0 {
            return Err(ProviderError::NoResults("duckduckgo"));
        }
        Ok(Response::from_text(out))
    }
}
