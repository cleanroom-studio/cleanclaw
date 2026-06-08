//! Provider factory — pick the right implementation from a runtime
//! `Config` + provider key.

use super::anthropic::{AnthropicConfig, AnthropicProvider};
use super::openai::{OpenAIConfig, OpenAIProvider};
use super::provider::{Provider, ProviderError};
use cleanclaw_config::ProviderConfig;
use std::collections::HashMap;
use std::sync::Arc;

pub fn build_provider(
    name: &str,
    cfg: &ProviderConfig,
) -> Result<Arc<dyn Provider>, ProviderError> {
    let api_key = resolve_api_key(&cfg.api_key, &cfg.api_type)
        .ok_or_else(|| ProviderError::Config(format!("provider {name} missing api key")))?;

    // Pick adapter from the explicit `apiType`, falling back to the
    // provider key for "openai" / "anthropic".
    let api_type = if !cfg.api_type.is_empty() {
        cfg.api_type.to_ascii_lowercase()
    } else {
        name.to_ascii_lowercase()
    };

    let base = if cfg.api_base.is_empty() {
        default_base(&api_type).to_string()
    } else {
        cfg.api_base.clone()
    };

    let provider: Arc<dyn Provider> = match api_type.as_str() {
        "openai" | "openai-compat" | "openai_compat" | "openrouter" | "v1" | "v1/chat" => {
            Arc::new(OpenAIProvider::new(OpenAIConfig {
                api_key,
                api_base: base,
            }))
        }
        "anthropic" | "anthropic-messages" | "claude" => {
            Arc::new(AnthropicProvider::new(AnthropicConfig {
                api_key,
                api_base: base,
                version: "2023-06-01".into(),
            }))
        }
        other => {
            // Default to OpenAI-compat for unknown types — works for the
            // majority of providers (Together, Groq, Mistral, OpenRouter,
            // vLLM, …) which all implement /v1/chat/completions.
            Arc::new(OpenAIProvider::new(OpenAIConfig {
                api_key,
                api_base: base,
            }))
            ._alias_ok(other)
            .clone()
        }
    };
    Ok(provider)
}

fn default_base(api_type: &str) -> &'static str {
    match api_type {
        "anthropic" | "anthropic-messages" | "claude" => "https://api.anthropic.com",
        _ => "https://api.openai.com/v1",
    }
}

/// Resolve the API key. Supports either a literal value (`apiKey: sk-…`)
/// or an env-var reference (`apiKey: $OPENAI_API_KEY`).
fn resolve_api_key(api_key: &str, _api_type: &str) -> Option<String> {
    if let Some(rest) = api_key.strip_prefix('$') {
        std::env::var(rest).ok()
    } else {
        Some(api_key.to_string())
    }
}

// Build a per-request env-passed view of providers.
pub fn build_all(
    providers: &HashMap<String, ProviderConfig>,
) -> Result<HashMap<String, Arc<dyn Provider>>, ProviderError> {
    let mut out = HashMap::new();
    for (name, cfg) in providers {
        out.insert(name.clone(), build_provider(name, cfg)?);
    }
    Ok(out)
}

// ---- helper trait so the wildcard arm above is a no-op ---------------------
trait _Alias {
    fn _alias_ok(self, _other: &str) -> Self;
}
impl<T> _Alias for Arc<T> {
    fn _alias_ok(self, _other: &str) -> Self {
        self
    }
}
