//! URL normalization for provider API bases. Mirrors
//! .
//!
//! Different API types disagree on whether `/v1` is part of the
//! base or part of the path:
//!
//!   - OpenAI Chat Completions: runtime appends "/chat/completions",
//!     assuming /v1 is already in the base. A bare host hits 404.
//!   - Anthropic Messages: runtime appends "/v1/messages", assuming
//!     /v1 is NOT in the base. A trailing /v1 produces /v1/v1/messages.
//!
//! Both forms are common typos (people copy "https://api.openai.com"
//! off a doc page, or paste "https://api.anthropic.com/v1" by
//! analogy with OpenAI). We fold them into the canonical shape
//! here so the connection test, the runtime, and any other consumer
//! all hit the same URL.

/// Fold the user-typed `api_base` into the canonical form for
/// `api_type`. Returns the empty string unchanged.
//
/// Rules are intentionally conservative — we only touch the
/// trailing `/v1` segment, and only when the user gave us a bare
/// host (no custom path). Third-party gateways with their own
/// routing convention (e.g. "https://my-gateway.com/openai") are
/// left alone.
pub fn normalize_api_base(api_base: &str, api_type: &str) -> String {
    let base = api_base.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return String::new();
    }
    match api_type {
        "anthropic-messages" => base.trim_end_matches("/v1").to_string(),
        _ => {
            // If the path is non-empty, the user picked a custom
            // gateway layout — leave it alone.
            if let Some(idx) = base.find("://") {
                let after_scheme = &base[idx + 3..];
                if let Some(slash) = after_scheme.find('/') {
                    if !after_scheme[slash..].is_empty() {
                        return base;
                    }
                }
            }
            // Bare host — append /v1 for OpenAI-style callers.
            format!("{base}/v1")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_bare_host_gets_v1() {
        assert_eq!(
            normalize_api_base("https://api.openai.com", "openai-chat"),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn openai_with_v1_idempotent() {
        assert_eq!(
            normalize_api_base("https://api.openai.com/v1", "openai-chat"),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn openai_trailing_slash_normalized() {
        assert_eq!(
            normalize_api_base("https://api.openai.com/", "openai-chat"),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn anthropic_strips_trailing_v1() {
        assert_eq!(
            normalize_api_base("https://api.anthropic.com/v1", "anthropic-messages"),
            "https://api.anthropic.com"
        );
    }

    #[test]
    fn anthropic_bare_host_kept() {
        assert_eq!(
            normalize_api_base("https://api.anthropic.com", "anthropic-messages"),
            "https://api.anthropic.com"
        );
    }

    #[test]
    fn custom_gateway_path_preserved() {
        // A third-party gateway that mounts OpenAI under a custom
        // path is left alone — we can't guess where /v1 belongs.
        assert_eq!(
            normalize_api_base("https://my-gateway.com/openai", "openai-chat"),
            "https://my-gateway.com/openai"
        );
    }

    #[test]
    fn empty_string_returns_empty() {
        assert_eq!(normalize_api_base("", "openai-chat"), "");
        assert_eq!(normalize_api_base("   ", "openai-chat"), "");
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(
            normalize_api_base("  https://api.openai.com  ", "openai-chat"),
            "https://api.openai.com/v1"
        );
    }
}
