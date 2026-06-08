//! API key issuance + SHA-256 verification.
//!
//! Format: `fk_<random 32 chars base32>`. Only the SHA-256 hash is
//! persisted; the prefix (first 8 chars after `fk_`) is stored plain
//! for UI display.

use base64::Engine;
use cleanclaw_core::{ApiKeyId, CleanClawError, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const KEY_PREFIX_LEN: usize = 8;
const KEY_BODY_LEN: usize = 32; // bytes
const PREFIX: &str = "fk_";

/// Generate a fresh API key. Returns the full key string (caller
/// surfaces it to the user once) and the SHA-256 hash to persist.
pub fn generate() -> (String, String, String) {
    let mut body = [0u8; KEY_BODY_LEN];
    rand::thread_rng().fill_bytes(&mut body);
    // URL-safe base64 w/o padding → 43 chars from 32 bytes.
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body);
    let key = format!("{PREFIX}{b64}");
    let _id = ApiKeyId::generate();
    let prefix = key[..KEY_PREFIX_LEN + PREFIX.len()].to_string();
    let hash = sha256_hex(&key);
    (key, hash, prefix)
}

pub fn sha256_hex(s: &str) -> String {
    let digest = Sha256::digest(s.as_bytes());
    hex::encode(digest)
}

pub fn key_id_from_token(token: &str) -> Result<&str> {
    token
        .strip_prefix(PREFIX)
        .ok_or_else(|| CleanClawError::InvalidArgument("api key must start with fk_".into()))
        .map(|_| token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_is_unique_and_verifies() {
        let (k1, h1, p1) = generate();
        let (k2, h2, p2) = generate();
        assert_ne!(k1, k2);
        assert_ne!(h1, h2);
        assert!(k1.starts_with(PREFIX));
        assert_eq!(p1.len(), KEY_PREFIX_LEN + PREFIX.len());
        assert_eq!(p2.len(), KEY_PREFIX_LEN + PREFIX.len());
        // Round-trip: hashing the same token yields the same hash.
        assert_eq!(sha256_hex(&k1), h1);
        assert_eq!(sha256_hex(&k2), h2);
    }
}
