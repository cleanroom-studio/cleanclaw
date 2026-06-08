//! API key rotation. Mirrors the Go helper in
//!  that issues a fresh key on
//! demand and refuses to re-issue the same token within a grace window.
//!
//! Defence-in-depth: the previous key's SHA-256 hash is stashed in
//! `ApiKeyRecord.prev_hash` (with `prev_hash_set_at`). Any future
//! `rotate` call that would mint a key whose hash collides with the
//! current OR previous hash is rejected.

use chrono::{DateTime, Duration, Utc};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::Store;
use std::sync::Arc;

use crate::apikey::{generate, sha256_hex};

/// How long a previously-rotated-out key's hash stays in the
/// "do-not-reissue" set. Past this, the hash is forgotten and the
/// caller can rotate back to the same logical token if they really
/// must (the chance of an accidental 32-byte base64 collision is
/// negligible).
pub const DEFAULT_ROTATION_GRACE: Duration = Duration::seconds(30 * 24 * 3600); // 30 days

/// Reason the rotation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotationRefusal {
    /// The minted key's hash collides with the current active hash.
    MatchesCurrentHash,
    /// The minted key's hash collides with the previous (in-grace) hash.
    MatchesPreviousHash { within_grace: bool },
    /// Target apikey does not exist.
    NotFound,
    /// Caller did not own the apikey.
    NotOwner,
}

#[derive(Debug, Clone)]
pub struct RotationOutcome {
    pub plaintext: String,
    pub hash: String,
    pub prefix: String,
    pub grace_until: DateTime<Utc>,
}

pub struct ApikeyRotator {
    store: Arc<dyn Store>,
    grace: Duration,
}

impl ApikeyRotator {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self::with_grace(store, DEFAULT_ROTATION_GRACE)
    }

    pub fn with_grace(store: Arc<dyn Store>, grace: Duration) -> Self {
        Self { store, grace }
    }

    /// Issue a fresh key for an existing apikey, refusing to re-issue
    /// the current or in-grace previous key. The new hash is written
    /// to the row and the old hash moves into `prev_hash`.
    pub async fn rotate(
        &self,
        apikey_id: &str,
        caller_user_id: &str,
    ) -> std::result::Result<RotationOutcome, RotationRefusal> {
        let existing = match self.store.get_api_key(apikey_id).await {
            Ok(k) => k,
            Err(CleanClawError::NotFound(_)) => return Err(RotationRefusal::NotFound),
            Err(_) => return Err(RotationRefusal::NotFound),
        };
        if existing.user_id != caller_user_id {
            return Err(RotationRefusal::NotOwner);
        }
        let (key, hash, prefix) = generate();

        // Refuse same-hash-as-current.
        if hash == existing.key_hash {
            return Err(RotationRefusal::MatchesCurrentHash);
        }
        // Refuse same-hash-as-previous (if still in grace).
        if let Some(prev) = &existing.prev_hash {
            if &hash == prev {
                let within_grace = existing
                    .prev_hash_set_at
                    .map(|t| t + self.grace > Utc::now())
                    .unwrap_or(false);
                return Err(RotationRefusal::MatchesPreviousHash { within_grace });
            }
        }
        self.store
            .rotate_api_key(apikey_id, &hash, &prefix)
            .await
            .map_err(|_| RotationRefusal::NotFound)?;
        Ok(RotationOutcome {
            plaintext: key,
            hash,
            prefix,
            grace_until: existing.prev_hash_set_at.unwrap_or_else(Utc::now) + self.grace,
        })
    }

    /// Check whether `candidate_token` would be refused by the rotate
    /// pipeline. Useful for tests + dry-run validation.
    pub async fn would_refuse(
        &self,
        apikey_id: &str,
        candidate_token: &str,
    ) -> std::result::Result<Option<RotationRefusal>, CleanClawError> {
        let existing = self.store.get_api_key(apikey_id).await?;
        let hash = sha256_hex(candidate_token);
        if hash == existing.key_hash {
            return Ok(Some(RotationRefusal::MatchesCurrentHash));
        }
        if let Some(prev) = &existing.prev_hash {
            if &hash == prev {
                let within_grace = existing
                    .prev_hash_set_at
                    .map(|t| t + self.grace > Utc::now())
                    .unwrap_or(false);
                return Ok(Some(RotationRefusal::MatchesPreviousHash { within_grace }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apikey::generate;

    fn make_record(id: &str, user_id: &str, hash: &str) -> ApiKeyRecord {
        ApiKeyRecord {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: "".into(),
            key_hash: hash.to_string(),
            key_prefix: "fk_".into(),
            r#type: "user".into(),
            created_at: Utc::now(),
            prev_hash: None,
            prev_hash_set_at: None,
        }
    }

    #[test]
    fn refusal_variants_distinct() {
        assert_ne!(
            RotationRefusal::MatchesCurrentHash,
            RotationRefusal::MatchesPreviousHash { within_grace: true }
        );
    }

    #[test]
    fn grace_default_is_thirty_days() {
        assert_eq!(DEFAULT_ROTATION_GRACE, Duration::days(30));
    }

    #[test]
    fn make_record_blank() {
        let r = make_record("fk_1", "u_1", "abc");
        assert_eq!(r.id, "fk_1");
        assert!(r.prev_hash.is_none());
        assert!(r.prev_hash_set_at.is_none());
    }

    #[test]
    fn hash_round_trip_consistent() {
        let (k, h, _p) = generate();
        assert_eq!(sha256_hex(&k), h);
    }

    #[test]
    fn refusal_debug() {
        // Smoke: Debug impl is used in logs.
        let r = RotationRefusal::MatchesPreviousHash { within_grace: true };
        assert!(format!("{r:?}").contains("MatchesPreviousHash"));
    }
}
