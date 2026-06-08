//! Goal identifier generation. Mirrors
//! .
//!
//! Random 16-byte hex with a `g-` prefix. The `g-` prefix is purely
//! human-readable (makes a "g-3a4f" id scan as a goal id, not a
//! message id, in logs and SQL clients) — uniqueness comes from
//! the random bytes.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;

/// NewID returns a fresh opaque identifier for a goal row. The
/// `g-` prefix is purely for human readability — uniqueness is
/// guaranteed by 12 random bytes (96 bits of entropy).
pub fn new_id() -> String {
    let mut buf = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut buf);
    format!("g-{}", URL_SAFE_NO_PAD.encode(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn new_id_has_g_prefix() {
        let id = new_id();
        assert!(id.starts_with("g-"));
    }

    #[test]
    fn new_id_is_unique() {
        let mut seen = HashSet::new();
        for _ in 0..200 {
            let id = new_id();
            assert!(seen.insert(id.clone()), "duplicate id: {id}");
        }
    }

    #[test]
    fn new_id_length_is_16_plus_prefix() {
        // 12 random bytes → 16 chars base64url. Plus "g-" → 18 chars.
        let id = new_id();
        assert_eq!(id.len(), 2 + 16);
    }
}
