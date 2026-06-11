//! Explicit "no image generation provider configured" sentinel.
//!
//! The chain handler in `agent/tools/image_gen.go` (Go) treats
//! this provider as a hard stop: the model never sees a "none"
//! entry in the tool description, but if it ends up in a chain
//! (e.g. because the only other provider was skipped for missing
//! config) we surface a clear "no results" so the agent can fall
//! back gracefully.
use async_trait::async_trait;

use super::CATEGORY;
use crate::{Provider, ProviderError, Request, Response};

/// `none` sentinel — chain handler in `agent/tools/image_gen.go`
/// (Go) short-circuits on this so the model never sees a "none"
/// provider. Mirrored on the Rust side.
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
        Err(ProviderError::NoResults("imagegen: none sentinel"))
    }
    fn credential_free(&self) -> bool {
        true
    }
}
