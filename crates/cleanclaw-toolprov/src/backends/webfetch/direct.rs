//! Direct HTTP GET fetcher. The always-on fallback: no API key,
//! no proxy, just `reqwest` with a `cleanclaw/1.0` User-Agent.
//! Body is truncated so a single 5 MB page can't blow the LLM
//! context.
use async_trait::async_trait;

use super::{parse_args, CATEGORY};
use crate::{Provider, ProviderError, Request, Response};

/// Always-on direct fetcher; opts into CredentialFree so the
/// dashboard can pick it without an API key.
pub struct Direct {
    client: reqwest::Client,
}

impl Direct {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Provider for Direct {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "direct"
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let url = parse_args(&req.args)?;
        let resp = self
            .client
            .get(&url)
            .header("user-agent", "cleanclaw/1.0")
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ProviderError::Upstream(format!("{}", resp.status())));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        // Truncate so the LLM context doesn't blow up on a 5 MB
        // page; the rest can be re-fetched on demand.
        let max = 16 * 1024;
        let truncated = if text.len() > max {
            format!(
                "{}\n\n[truncated; original {} bytes]",
                &text[..max],
                text.len()
            )
        } else {
            text
        };
        Ok(Response::from_text(truncated))
    }
    fn credential_free(&self) -> bool {
        true
    }
}
