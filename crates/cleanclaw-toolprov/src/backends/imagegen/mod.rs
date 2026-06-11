//! Built-in image-generation backends.
//!
//! Two providers:
//!
//!   * [`OpenAI`] — DALL·E 3 / gpt-image-1 via `/v1/images/generations`.
//!   * [`None`]   — explicit no-op sentinel; the chain short-circuits
//!     on this so the model never sees a "none" provider in the
//!     tool description.
//!
//! The `CATEGORY` constant (`"image_gen"`) is the key used by the
//! `Registry` to look up these providers. The chain reads it back
//! from `Provider::category()` so there is no string duplication.
use crate::ProviderError;

// Re-export the category name for callers that want to compose a
// `Chain` by category without depending on the private string.
pub const CATEGORY: &str = "image_gen";

mod none;
mod openai;

pub use none::None;
pub use openai::OpenAI;

/// Parse the LLM-supplied args blob into a normalized
/// `(prompt, size, n)` triple. `n` is clamped to `1..=4` so a
/// confused model can never request 50 images and burn the
/// upstream quota.
pub(crate) fn parse_args(
    raw: &serde_json::Value,
) -> Result<(String, String, usize), ProviderError> {
    let prompt = raw
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if prompt.is_empty() {
        return Err(ProviderError::InvalidArgs("prompt is required".into()));
    }
    let size = raw
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let n = raw.get("n").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let n = n.clamp(1, 4);
    Ok((prompt, size, n))
}

/// Render the upstream `data[].url` shape into the markdown
/// payload the model sees.
pub(crate) fn render_urls(prompt: &str, urls: &[String]) -> String {
    if urls.is_empty() {
        return String::new();
    }
    let mut s = format!("Generated {} image(s) for: {prompt}\n\n", urls.len());
    for (i, u) in urls.iter().enumerate() {
        s.push_str(&format!("{}. ![image {}]({})\n", i + 1, i + 1, u));
    }
    s
}

/// Render the upstream `data[].b64_json` shape (gpt-image-1)
/// into inline data-URI markdown the model can paste back.
pub(crate) fn render_b64(prompt: &str, b64s: &[String]) -> String {
    if b64s.is_empty() {
        return String::new();
    }
    let mut s = format!("Generated {} image(s) for: {prompt}\n\n", b64s.len());
    for (i, b) in b64s.iter().enumerate() {
        s.push_str(&format!(
            "{}. ![image {}](data:image/png;base64,{})\n",
            i + 1,
            i + 1,
            b
        ));
    }
    s
}
