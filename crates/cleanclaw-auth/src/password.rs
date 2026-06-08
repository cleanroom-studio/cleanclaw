//! Argon2id password hashing. Encoded as a single string:
//!   `argon2id$v=19$m=...,t=...,p=...$<salt_b64>$<hash_b64>`

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use cleanclaw_core::{CleanClawError, Result};

pub fn hash_password(plain: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let hash = argon
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| CleanClawError::Internal(format!("hash password: {e}")))?;
    Ok(hash.to_string())
}

pub fn verify_password(plain: &str, encoded: &str) -> Result<bool> {
    let parsed = PasswordHash::new(encoded)
        .map_err(|e| CleanClawError::Internal(format!("parse hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let h = hash_password("hunter2").unwrap();
        assert!(verify_password("hunter2", &h).unwrap());
        assert!(!verify_password("hunter3", &h).unwrap());
    }
}
