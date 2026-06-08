//! Auth crate — password hashing, API key issuance, web session cookies,
//! and an axum middleware that resolves an HTTP request to an `Identity`.

pub mod apikey;
pub mod identity;
pub mod password;
pub mod resolver;
pub mod rotation;
pub mod session;
pub mod users;

pub use identity::{ApiKeyType, AuthMethod, Identity, Role};
pub use resolver::{Resolver, SESSION_COOKIE_NAME, SESSION_TTL};
pub use users::{
    Account, Accounts, CreateInput, UserError, ROLE_APP_USER, ROLE_SUPER_ADMIN, ROLE_USER,
    STATUS_ACTIVE, STATUS_DISABLED,
};

// =====================================================================
// Password-reset token lifecycle. Mirrors the typical
//  password-reset flow: issue a
// single-use, time-bounded token, verify it, and consume it.
// =====================================================================

pub mod reset {
    use cleanclaw_core::{CleanClawError, Result};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Opaque single-use reset token. The plaintext is shown to the
    /// user once at issue time; the stored value is its SHA-256 hex
    /// digest keyed by the user's account id.
    #[derive(Debug, Clone)]
    pub struct ResetToken {
        pub plaintext: String,
        pub expires_at: Instant,
    }

    /// In-memory token store. Production deployments persist this in
    /// the user store; this is the offline-friendly shape used by
    /// `cleanclaw-cli reset-password` and by the integration tests.
    #[derive(Debug, Default)]
    pub struct ResetStore {
        ttl: Duration,
        inner: Mutex<HashMap<String, (String, Instant)>>,
    }

    impl ResetStore {
        pub fn new() -> Self {
            Self::with_ttl(Duration::from_secs(60 * 30))
        }

        pub fn with_ttl(ttl: Duration) -> Self {
            Self {
                ttl,
                inner: Mutex::new(HashMap::new()),
            }
        }

        /// Issue a fresh token for `account_id`. Returns the plaintext
        /// token the user must quote to confirm.
        pub fn issue(&self, account_id: &str) -> Result<ResetToken> {
            if account_id.is_empty() {
                return Err(CleanClawError::InvalidArgument(
                    "account_id required".into(),
                ));
            }
            let mut buf = [0u8; 24];
            use rand::RngCore;
            rand::thread_rng().fill_bytes(&mut buf);
            let plaintext = format!("rst_{}", hex::encode(buf));
            let digest = crate::apikey::sha256_hex(&plaintext);
            let expires_at = Instant::now() + self.ttl;
            self.inner
                .lock()
                .map_err(|e| CleanClawError::Internal(format!("reset lock: {e}")))?
                .insert(account_id.to_string(), (digest, expires_at));
            Ok(ResetToken {
                plaintext,
                expires_at,
            })
        }

        /// Verify `account_id` + `token`. Returns Ok(()) on a valid,
        /// unexpired match. The token is **consumed** on success.
        pub fn consume(&self, account_id: &str, token: &str) -> Result<()> {
            let mut guard = self
                .inner
                .lock()
                .map_err(|e| CleanClawError::Internal(format!("reset lock: {e}")))?;
            let (digest, expires_at) = guard
                .remove(account_id)
                .ok_or_else(|| CleanClawError::NotFound("no reset token".into()))?;
            if Instant::now() > expires_at {
                return Err(CleanClawError::InvalidArgument(
                    "reset token expired".into(),
                ));
            }
            let provided = crate::apikey::sha256_hex(token);
            if provided != digest {
                return Err(CleanClawError::Unauthorized);
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn issue_then_consume_ok() {
            let s = ResetStore::new();
            let tok = s.issue("acct_1").unwrap();
            assert!(tok.plaintext.starts_with("rst_"));
            s.consume("acct_1", &tok.plaintext).expect("consume ok");
        }

        #[test]
        fn consume_is_single_use() {
            let s = ResetStore::new();
            let tok = s.issue("acct_2").unwrap();
            s.consume("acct_2", &tok.plaintext).unwrap();
            let err = s.consume("acct_2", &tok.plaintext).unwrap_err();
            assert!(matches!(err, CleanClawError::NotFound(_)));
        }

        #[test]
        fn consume_rejects_wrong_token() {
            let s = ResetStore::new();
            let _ = s.issue("acct_3").unwrap();
            let err = s.consume("acct_3", "rst_garbage").unwrap_err();
            assert!(matches!(err, CleanClawError::Unauthorized));
        }

        #[test]
        fn consume_rejects_expired() {
            let s = ResetStore::with_ttl(Duration::from_millis(0));
            let tok = s.issue("acct_4").unwrap();
            std::thread::sleep(Duration::from_millis(5));
            let err = s.consume("acct_4", &tok.plaintext).unwrap_err();
            assert!(matches!(err, CleanClawError::InvalidArgument(_)));
        }

        #[test]
        fn issue_rejects_empty_account() {
            let s = ResetStore::new();
            let err = s.issue("").unwrap_err();
            assert!(matches!(err, CleanClawError::InvalidArgument(_)));
        }
    }
}
