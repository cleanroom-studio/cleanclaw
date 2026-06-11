//! Explicit "no web search provider configured" sentinel.
use async_trait::async_trait;

use super::CATEGORY;
use crate::{Provider, ProviderError, Request, Response};

pub struct None;

#[async_trait]
impl Provider for None {
    fn category(&self) -> &'static str {
        CATEGORY
    }
    fn name(&self) -> &'static str {
        "none"
    }
    async fn execute(&self, _req: Request) -> Result<Response, ProviderError> {
        Err(ProviderError::NoResults("websearch: none sentinel"))
    }
    fn credential_free(&self) -> bool {
        true
    }
}
