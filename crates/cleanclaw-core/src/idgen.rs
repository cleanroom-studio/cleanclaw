//! Lightweight random ID generator. Returns a short, unique ID with
//! a caller-chosen prefix.
//!
//! Used for `goal_*` rows whose primary key is a free-form string (we
//! don't have a strong-typed `GoalId` newtype because goals aren't
//! surfaced in URLs).

use rand::RngCore;

pub struct IdGen {
    rng: rand::rngs::ThreadRng,
}

impl IdGen {
    pub fn new() -> Self {
        Self {
            rng: rand::thread_rng(),
        }
    }
    pub fn next(&mut self, prefix: &str) -> String {
        let mut buf = [0u8; 8];
        self.rng.fill_bytes(&mut buf);
        // 16-char hex from 8 random bytes
        format!("{prefix}_{}", hex::encode(buf))
    }
}

impl Default for IdGen {
    fn default() -> Self {
        Self::new()
    }
}
