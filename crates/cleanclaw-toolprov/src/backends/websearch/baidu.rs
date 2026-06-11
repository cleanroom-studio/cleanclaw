//! Baidu search backend.
//!
//! `credential_free`: no key, but Baidu sometimes serves a
//! captcha to non-CN IPs; the chain transparently falls through
//! to the next provider in that case.
use async_trait::async_trait;

use super::{decode_html_entities, find_from, parse_args, strip_tags, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

/// Baidu search — `credential_free` (no key, but Baidu
/// sometimes serves a captcha to non-CN IPs; the chain will
/// transparently fall through to the next provider in that
/// case). Endpoint: `https://www.baidu.com/s?wd=…`
pub struct Baidu {
    client: reqwest::Client,
}

impl Baidu {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Provider for Baidu {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "baidu"
    }
    fn credential_free(&self) -> bool {
        true
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let (query, n) = parse_args(&req.args)?;
        // Baidu's HTML search needs a referer + a modern UA
        // to avoid the anti-bot page; we use the desktop UA
        // the browser would send.
        let resp = self
            .client
            .get("https://www.baidu.com/s")
            .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .query(&[("wd", query.as_str())])
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!("baidu {status}: {txt}")));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        // Baidu's result entries: `<h3 class="t"><a href="…" …>title</a></h3>`.
        // The actual destination URL is in the surrounding
        // `<a>` whose `href` is a redirect — but the visible
        // text is what we want.
        let mut out = String::new();
        out.push_str(&format!("Search results for: {query}\n\n"));
        let bytes = html.as_bytes();
        let needle = b"<h3 class=\"t\"";
        let mut cursor = 0usize;
        let mut idx = 0;
        while idx < n {
            let Some(pos) = find_from(&bytes[cursor..], needle) else {
                break;
            };
            cursor += pos + needle.len();
            // Skip the rest of the <h3 …> opening tag.
            if let Some(gt) = bytes[cursor..].iter().position(|&b| b == b'>') {
                cursor += gt + 1;
            } else {
                break;
            }
            // The title sits inside the <a>…</a> immediately after.
            if let Some(a_end) = find_from(&bytes[cursor..], b"</a>") {
                let raw = std::str::from_utf8(&bytes[cursor..cursor + a_end])
                    .unwrap_or("")
                    .trim();
                let title = decode_html_entities(&strip_tags(raw));
                // The redirect URL is in the parent <a> tag's
                // `href`. Walk back to find it.
                let url_start = bytes[..cursor]
                    .windows(5)
                    .rposition(|w| w == b"href=\"")
                    .map(|p| {
                        // Find the end of that anchor tag.
                        let after = p + 6;
                        let _ = after;
                        p + 6
                    })
                    .unwrap_or(cursor);
                let url_end = bytes[url_start..]
                    .iter()
                    .position(|&b| b == b'"')
                    .unwrap_or(0);
                let url_raw =
                    std::str::from_utf8(&bytes[url_start..url_start + url_end]).unwrap_or("");
                let url = if url_raw.starts_with("http") {
                    url_raw.to_string()
                } else {
                    String::new()
                };
                idx += 1;
                out.push_str(&format!(
                    "{}. {}\n   {}\n\n",
                    idx,
                    if title.is_empty() {
                        "(no title)"
                    } else {
                        &title
                    },
                    if url.is_empty() { "(no url)" } else { &url },
                ));
                cursor += a_end + 4;
            } else {
                break;
            }
        }
        if idx == 0 {
            return Err(ProviderError::NoResults("baidu"));
        }
        Ok(Response::from_text(out))
    }
}
