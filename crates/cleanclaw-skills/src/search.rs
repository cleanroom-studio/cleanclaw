//! Skills.sh search.
//!
//! The skills.sh registry exposes a JSON search API at
//! `https://skills.sh/api/search?q=<query>`. The response is a flat
//! list of `{ name, description, author, repo, tags[] }` rows.
//!
//! Offline-only: when the `http` feature isn't on, `search_registry`
//! returns `NotImplemented`. The CLI's `skill search` command picks
//! this up and prints a friendly message.

use cleanclaw_core::{CleanClawError, Result};
use serde::{Deserialize, Serialize};

/// One search hit. Subset of the skills.sh schema — fields the CLI
/// actually surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryHit {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryResponse {
    #[serde(default)]
    results: Vec<RegistryHit>,
}

/// URL template. Kept as a const so tests can pin the shape.
pub const SKILLS_SH_SEARCH: &str = "https://skills.sh/api/search";

/// Search the skills.sh registry for skills matching `query`. `limit`
/// defaults to 25 when zero or negative.
pub async fn search_registry(query: &str, limit: u32) -> Result<Vec<RegistryHit>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let limit = if limit == 0 { 25 } else { limit };
    let url = format!("{SKILLS_SH_SEARCH}?q={}&limit={}", urlencode(query), limit);
    fetch_and_parse(&url).await
}

async fn fetch_and_parse(url: &str) -> Result<Vec<RegistryHit>> {
    #[cfg(feature = "http")]
    {
        let client = reqwest::Client::builder()
            .user_agent("cleanclaw-cli")
            .build()
            .map_err(|e| CleanClawError::Internal(format!("client: {e}")))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| CleanClawError::Internal(format!("search: {e}")))?;
        if !resp.status().is_success() {
            return Err(CleanClawError::Internal(format!(
                "search returned {}",
                resp.status()
            )));
        }
        let body: RegistryResponse = resp
            .json()
            .await
            .map_err(|e| CleanClawError::Internal(format!("decode: {e}")))?;
        Ok(body.results)
    }
    #[cfg(not(feature = "http"))]
    {
        let _ = url;
        Err(CleanClawError::NotImplemented(
            "skill search requires the `http` feature on cleanclaw-skills".into(),
        ))
    }
}

/// URL-encode a query string the simple way (spaces → `+`, special
/// characters → `%XX`).
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Parse a raw `RegistryResponse` JSON value into a `Vec<RegistryHit>`.
/// Used by tests + by HTTP handlers that fetch the body themselves.
pub fn parse_response(raw: &serde_json::Value) -> Result<Vec<RegistryHit>> {
    let resp: RegistryResponse = serde_json::from_value(raw.clone())
        .map_err(|e| CleanClawError::Internal(format!("decode: {e}")))?;
    Ok(resp.results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_encode_basic() {
        assert_eq!(urlencode("hello world"), "hello+world");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("a?b"), "a%3Fb");
    }

    #[test]
    fn url_encode_empty() {
        assert_eq!(urlencode(""), "");
    }

    #[test]
    fn url_encode_unicode() {
        // Non-ASCII bytes each get a `%XX`. Two bytes for a 2-byte UTF-8
        // char. Each byte gets encoded independently.
        let s = urlencode("é");
        // "é" in UTF-8 is 0xC3 0xA9
        assert_eq!(s, "%C3%A9");
    }

    #[test]
    fn search_url_shape() {
        let url = format!(
            "{SKILLS_SH_SEARCH}?q={}&limit=10",
            urlencode("find skills")
        );
        assert_eq!(url, "https://skills.sh/api/search?q=find+skills&limit=10");
    }

    #[test]
    fn empty_query_returns_empty() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let res = rt.block_on(search_registry("", 10)).unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn empty_query_whitespace_returns_empty() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let res = rt.block_on(search_registry("   ", 10)).unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn parse_response_round_trip() {
        let raw = json!({
            "results": [
                {
                    "name": "data-analysis",
                    "description": "Pandas + numpy patterns",
                    "author": "alice",
                    "repo": "cleanroom-studio/cleanclaw",
                    "tags": ["data", "python"],
                },
                {
                    "name": "code-runner",
                    "description": "Use exec for code",
                }
            ]
        });
        let hits = parse_response(&raw).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "data-analysis");
        assert_eq!(hits[0].tags, vec!["data", "python"]);
        assert_eq!(hits[1].name, "code-runner");
        assert!(hits[1].author.is_empty());
        assert!(hits[1].tags.is_empty());
    }

    #[test]
    fn parse_response_missing_results_field() {
        let raw = json!({});
        let hits = parse_response(&raw).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn parse_response_malformed_errors() {
        let raw = json!({ "results": "not an array" });
        assert!(parse_response(&raw).is_err());
    }

    #[test]
    fn hit_serde_round_trip() {
        let h = RegistryHit {
            name: "x".into(),
            description: "y".into(),
            author: "a".into(),
            repo: "r".into(),
            tags: vec!["t".into()],
        };
        let s = serde_json::to_string(&h).unwrap();
        let back: RegistryHit = serde_json::from_str(&s).unwrap();
        assert_eq!(h, back);
    }
}
