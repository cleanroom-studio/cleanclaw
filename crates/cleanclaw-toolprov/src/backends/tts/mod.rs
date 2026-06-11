//! Built-in text-to-speech backends.
//!
//! Two providers:
//!
//!   * [`OpenAI`] — `POST /v1/audio/speech` returning raw MP3 bytes.
//!   * [`None`]   — explicit no-op sentinel; the chain short-circuits
//!     on this so the model never sees a "none" provider in the
//!     tool description.
use crate::ProviderError;

/// Registry category key. The chain reads it back from
/// `Provider::category()` so there is no string duplication.
pub const CATEGORY: &str = "tts";

mod none;
mod openai;

pub use none::None;
pub use openai::OpenAI;

/// Parse the LLM-supplied args blob into a normalized
/// `(text, voice)` pair. Empty `text` is rejected because the
/// upstream charges per character.
pub(crate) fn parse_args(raw: &serde_json::Value) -> Result<(String, String), ProviderError> {
    let text = raw
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return Err(ProviderError::InvalidArgs("text is required".into()));
    }
    let voice = raw
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((text, voice))
}
