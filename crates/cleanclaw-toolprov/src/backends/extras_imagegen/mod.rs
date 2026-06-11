//! "Extras" image-generation backends that depend on a
//! third-party aggregator or hosted model marketplace. These are
//! **opt-in** — they live in their own namespace so the built-in
//! set in [`crate::backends::imagegen`] stays small.
//!
//! Providers:
//!
//!   * [`Fal`]       — `https://fal.run/<model>`. Auth: `Key <token>`.
//!   * [`Replicate`] — Replicate's predictions API with
//!     `Prefer: wait` for synchronous responses.
use crate::Registry;

mod fal;
mod replicate;

pub use fal::Fal;
pub use replicate::Replicate;

/// Small helper: pull a `&str` field out of a JSON args blob.
/// Used by every provider in this file; living in a shared
/// module keeps the per-provider files tight.
pub(crate) fn str_field<'a>(args: &'a serde_json::Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Register both providers on a registry. Called from
/// `register_extras`.
pub fn register(reg: &Registry, client: &reqwest::Client) {
    reg.register(std::sync::Arc::new(Fal::new(client.clone())));
    reg.register(std::sync::Arc::new(Replicate::new(client.clone())));
}
