//! "Extras" text-to-speech backends.
//!
//! Providers:
//!
//!   * [`ElevenLabs`] — `https://api.elevenlabs.io/v1/text-to-speech/{voice_id}`.
//!   * [`Fish`]       — `https://api.fish.audio/v1/tts`.
//!   * [`MiniMax`]    — `https://api.minimaxi.com/v1/t2a_v2`.
use crate::Registry;

mod elevenlabs;
mod fish;
mod minimax;

pub use elevenlabs::ElevenLabs;
pub use fish::Fish;
pub use minimax::MiniMax;

pub(crate) fn str_field<'a>(args: &'a serde_json::Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

pub fn register(reg: &Registry, client: &reqwest::Client) {
    reg.register(std::sync::Arc::new(ElevenLabs::new(client.clone())));
    reg.register(std::sync::Arc::new(Fish::new(client.clone())));
    reg.register(std::sync::Arc::new(MiniMax::new(client.clone())));
}
