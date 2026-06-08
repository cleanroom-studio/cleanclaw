//! Provider trait — chat completions, both blocking and streaming.
//!
//! All async; backends return `Result<_, ProviderError>`.

use super::message::*;
use async_trait::async_trait;
use futures_util::Stream;
use std::pin::Pin;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(String),
    #[error("auth: {0}")]
    Auth(String),
    #[error("rate limit")]
    RateLimited,
    #[error("upstream: {0}")]
    Upstream(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("config: {0}")]
    Config(String),
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send + Sync>>;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Unique provider name (e.g. `"openai"`, `"anthropic"`).
    fn name(&self) -> &str;

    /// Blocking chat completion.
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Streaming chat completion. Yields `StreamEvent`s; final
    /// `StreamEvent::Done` carries the usage summary.
    async fn chat_stream(&self, req: &ChatRequest) -> Result<ProviderStream, ProviderError>;
}
