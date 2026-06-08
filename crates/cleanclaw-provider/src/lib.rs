//! LLM provider layer.
//!
//! We define a `Provider`
//! trait, implement it for OpenAI and Anthropic, and provide a factory
//! that picks the right implementation from the runtime `Config`.

pub mod anthropic;
pub mod anthropic_xml_fallback;
pub mod credentials;
pub mod factory;
pub mod message;
pub mod openai;
pub mod provider;
pub mod url;

pub use factory::build_provider;
pub use message::*;
pub use provider::{Provider, ProviderError, ProviderStream};
