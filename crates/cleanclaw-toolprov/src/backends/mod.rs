//! Catalog of every `Provider` implementation this crate ships.
//!
//! The crate historically stuffed these into `lib.rs` and
//! `extra_backends.rs`; this module re-homes them under a
//! category-per-directory tree that mirrors the four tool
//! surfaces the LLM sees:
//!
//!   * [`imagegen`]   — image generation
//!   * [`tts`]        — text-to-speech
//!   * [`webfetch`]   — URL fetch
//!   * [`websearch`]  — web search
//!
//! The "extras" sub-modules are opt-in third-party backends
//! (Fal / Replicate / ElevenLabs / Fish / MiniMax / Firecrawl /
//! Exa / SearXNG). They register on the same `Registry` as the
//! built-ins, but are kept in their own namespace so the canonical
//! 4-tool set stays small.
use crate::Registry;

// ----- built-in categories (originally in lib.rs) -----

pub mod imagegen;
pub mod tts;
pub mod webfetch;
pub mod websearch;

// ----- opt-in third-party backends (originally in extra_backends.rs) -----

pub mod extras_imagegen;
pub mod extras_tts;
pub mod extras_fetch;
pub mod extras_search;

/// Register every opt-in third-party backend on a `Registry`.
/// `register_builtin` calls this after registering the
/// always-on built-ins, so the registry ends up with all 12.
pub fn register_extras(reg: &Registry, client: &reqwest::Client) {
    extras_imagegen::register(reg, client);
    extras_tts::register(reg, client);
    extras_fetch::register(reg, client);
    extras_search::register(reg, client);
}
