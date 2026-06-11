//! Exa web search.
//!
//! `POST https://api.exa.ai/search` with bearer auth. The
//! response is a `results` array; we project the top 5 to text.
use async_trait::async_trait;
use serde_json::{json, Value};

use super::str_field;
use crate::{Provider, ProviderError, Request, Response};

const CATEGORY: &str = "web_search";

pub struct Exa {
    client: reqwest::Client,
}

impl Exa {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Provider for Exa {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "exa"
    }
    async fn execute(&self, req: Request) -> Result<Response, ProviderError> {
        let q = str_field(&req.args, "query");
        if q.is_empty() {
            return Err(ProviderError::InvalidArgs(
                "websearch: query required".into(),
            ));
        }
        if req.config.api_key.is_empty() {
            return Err(ProviderError::MissingApiKey("exa"));
        }
        let body = json!({
            "query": q,
            "numResults": 5,
            "useAutoprompt": false,
            "type": "neural",
        });
        let resp = self
            .client
            .post("https://api.exa.ai/search")
            .bearer_auth(&req.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!("exa {status}: {txt}")));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        let results = v
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = String::new();
        for (i, r) in results.iter().take(5).enumerate() {
            let title = r.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
            out.push_str(&format!("{}. {}\n   {}\n", i + 1, title, url));
        }
        if out.is_empty() {
            return Err(ProviderError::NoResults("exa"));
        }
        Ok(Response::from_text(out))
    }
}
