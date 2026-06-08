//! Credential management. Mirrors
//! .
//!
//! Provides:
//!   * Per-user credential store at `~/.cleanclaw/users/{userID}/credentials.json`
//!   * AES-256-GCM encryption with a key derived from `userID` +
//!     optional passphrase (or hostname + home, when no passphrase)
//!   * `Set` / `Get` / `List` / `Delete` operations
//!   * `Discover` — scan env for known provider keys
//!   * `InjectEnv` — produce the env-var map the sandbox wants
//!
//! Plaintext credentials never touch disk. The on-disk format is
//! `nonce || ciphertext || tag` (12-byte nonce prefix, then the
//! GCM `Seal` output). For pre-multiuser installs (legacy key),
//! `load` falls back to the machine-derived key and re-encrypts
//! on the next save.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use ring::aead::{Aad, BoundKey, Nonce, NonceSequence, SealingKey, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("key derivation failed: {0}")]
    Key(String),
    #[error("decrypt failed")]
    Decrypt,
    #[error("not found: credential {0:?}")]
    NotFound(String),
    #[error("not found: key {key:?} in credential {name:?}")]
    KeyNotFound { name: String, key: String },
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("ring: {0}")]
    Ring(String),
}

/// One stored credential, with key-value pairs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CredentialEntry {
    pub name: String,
    /// "api_key" | "oauth" | "token"
    #[serde(default = "default_type")]
    pub r#type: String,
    /// "config" | "env" | "store"
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub keys: HashMap<String, String>,
}

fn default_type() -> String {
    "api_key".into()
}
fn default_source() -> String {
    "store".into()
}

/// One-shot nonce sequence for a single SealingKey/OpeningKey.
/// Each call to `next()` returns the same nonce — use one
/// per `seal`/`open` call.
struct OneNonce(Option<[u8; 12]>);

impl NonceSequence for OneNonce {
    fn advance(&mut self) -> Result<Nonce, ring::error::Unspecified> {
        self.0.take().map(Nonce::assume_unique_for_key).ok_or(ring::error::Unspecified)
    }
}

/// AES-256-GCM encrypted credential store, scoped to one user.
pub struct CredentialManager {
    user_id: String,
    master_key: [u8; 32],
    store_path: PathBuf,
    entries: RwLock<HashMap<String, CredentialEntry>>,
    needs_reencrypt: std::sync::atomic::AtomicBool,
}

impl CredentialManager {
    /// Construct a manager for `user_id`, deriving the master key
    /// from `passphrase` (or from the machine identity when
    /// `passphrase` is empty).
    pub fn for_user(user_id: &str, passphrase: &str) -> std::result::Result<Self, CredentialError> {
        if user_id.is_empty() {
            return Err(CredentialError::Invalid(
                "user_id is required".into(),
            ));
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let store_dir = PathBuf::from(&home).join(".cleanclaw").join("users").join(user_id);
        std::fs::create_dir_all(&store_dir)?;
        let master_key = derive_key_for_user(user_id, passphrase);
        let store_path = store_dir.join("credentials.json");
        let mut m = Self {
            user_id: user_id.to_string(),
            master_key,
            store_path,
            entries: RwLock::new(HashMap::new()),
            needs_reencrypt: std::sync::atomic::AtomicBool::new(false),
        };
        if let Err(e) = m.load() {
            // First-run or corrupt file — start fresh. The
            // caller can still call Set() to populate.
            tracing::debug!(?e, "credential load failed; starting fresh");
        }
        if m.needs_reencrypt.load(std::sync::atomic::Ordering::Acquire) {
            let _ = m.save();
            m.needs_reencrypt
                .store(false, std::sync::atomic::Ordering::Release);
        }
        Ok(m)
    }

    /// Set a `key=value` pair on the credential named `name`.
    /// Creates the credential if it doesn't exist.
    pub fn set(&self, name: &str, key: &str, value: &str) -> std::result::Result<(), CredentialError> {
        let mut entries = self.entries.write().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        let entry = entries.entry(name.to_string()).or_insert_with(|| CredentialEntry {
            name: name.to_string(),
            r#type: "api_key".into(),
            source: "store".into(),
            keys: HashMap::new(),
        });
        entry.keys.insert(key.to_string(), value.to_string());
        drop(entries);
        self.save()
    }

    pub fn get(&self, name: &str, key: &str) -> std::result::Result<String, CredentialError> {
        let entries = self.entries.read().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        let entry = entries
            .get(name)
            .ok_or_else(|| CredentialError::NotFound(name.to_string()))?;
        entry
            .keys
            .get(key)
            .cloned()
            .ok_or_else(|| CredentialError::KeyNotFound { name: name.into(), key: key.into() })
    }

    /// List credentials, masking each key value (`abcd...wxyz`).
    pub fn list(&self) -> std::result::Result<Vec<CredentialEntry>, CredentialError> {
        let entries = self.entries.read().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        Ok(entries
            .values()
            .map(|e| {
                let mut masked = e.clone();
                masked.keys = e
                    .keys
                    .iter()
                    .map(|(k, v)| {
                        let masked_v = if v.len() > 8 {
                            format!("{}...{}", &v[..4], &v[v.len() - 4..])
                        } else {
                            "****".into()
                        };
                        (k.clone(), masked_v)
                    })
                    .collect();
                masked
            })
            .collect())
    }

    pub fn delete(&self, name: &str) -> std::result::Result<(), CredentialError> {
        let mut entries = self.entries.write().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        if entries.remove(name).is_none() {
            return Err(CredentialError::NotFound(name.into()));
        }
        drop(entries);
        self.save()
    }

    /// Scan env for known provider keys. Doesn't persist — the
    /// caller is expected to merge into the store via `set` if
    /// they want to keep them.
    pub fn discover() -> Vec<CredentialEntry> {
        let mut discovered = Vec::new();
        for (provider, env_vars) in known_env_vars() {
            for env_var in env_vars {
                if let Ok(val) = std::env::var(env_var) {
                    if val.is_empty() {
                        continue;
                    }
                    let mut keys = HashMap::new();
                    keys.insert("apiKey".to_string(), val);
                    discovered.push(CredentialEntry {
                        name: provider.to_string(),
                        r#type: "api_key".into(),
                        source: "env".into(),
                        keys,
                    });
                }
            }
        }
        discovered
    }

    /// Build the env-var map for the sandbox. Includes both stored
    /// credentials (mapped back to their canonical env var) and
    /// env-discovered ones.
    pub fn inject_env(&self) -> std::result::Result<HashMap<String, String>, CredentialError> {
        let entries = self.entries.read().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        let mut env = HashMap::new();
        let known = known_env_vars();
        for (name, entry) in entries.iter() {
            if let Some(api_key) = entry.keys.get("apiKey") {
                if let Some(env_vars) = known.get(name.as_str()) {
                    if let Some(v) = env_vars.first() {
                        env.insert((*v).to_string(), api_key.clone());
                    }
                } else {
                    env.insert(format!("{}_API_KEY", name.to_uppercase()), api_key.clone());
                }
            }
        }
        // Pass through env-discovered keys too.
        for env_vars in known.values() {
            for env_var in env_vars {
                if let Ok(val) = std::env::var(env_var) {
                    if !val.is_empty() {
                        env.insert((*env_var).to_string(), val);
                    }
                }
            }
        }
        Ok(env)
    }

    fn save(&self) -> std::result::Result<(), CredentialError> {
        let entries = self.entries.read().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        let plain = serde_json::to_vec(&*entries)?;
        let ct = encrypt(&plain, &self.master_key)?;
        std::fs::write(&self.store_path, ct)?;
        Ok(())
    }

    fn load(&mut self) -> std::result::Result<(), CredentialError> {
        let bytes = std::fs::read(&self.store_path)?;
        let plain = match decrypt(&bytes, &self.master_key) {
            Ok(p) => p,
            Err(_) => {
                // Try the legacy machine-only key for upgrade
                // continuity, then mark for re-encrypt on the
                // next save.
                let legacy = legacy_derive_key();
                match decrypt(&bytes, &legacy) {
                    Ok(p) => {
                        self.needs_reencrypt
                            .store(true, std::sync::atomic::Ordering::Release);
                        p
                    }
                    Err(_) => return Err(CredentialError::Decrypt),
                }
            }
        };
        let map: HashMap<String, CredentialEntry> = serde_json::from_slice(&plain)?;
        let mut entries = self.entries.write().map_err(|e| CredentialError::Invalid(e.to_string()))?;
        *entries = map;
        Ok(())
    }
}

fn known_env_vars() -> HashMap<&'static str, Vec<&'static str>> {
    let mut m: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    m.insert("openai", vec!["OPENAI_API_KEY"]);
    m.insert("anthropic", vec!["ANTHROPIC_API_KEY"]);
    m.insert("openrouter", vec!["OPENROUTER_API_KEY"]);
    m.insert("google", vec!["GOOGLE_API_KEY", "GEMINI_API_KEY"]);
    m.insert("mistral", vec!["MISTRAL_API_KEY"]);
    m.insert("cohere", vec!["COHERE_API_KEY"]);
    m.insert("groq", vec!["GROQ_API_KEY"]);
    m.insert("together", vec!["TOGETHER_API_KEY"]);
    m.insert("deepseek", vec!["DEEPSEEK_API_KEY"]);
    m
}

fn derive_key_for_user(user_id: &str, passphrase: &str) -> [u8; 32] {
    let seed = if !passphrase.is_empty() {
        format!("cleanclaw:user:{user_id}:pp:{passphrase}")
    } else {
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".into());
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("cleanclaw:user:{user_id}:host:{hostname}:{home}")
    };
    let hash = Sha256::digest(seed.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    key
}

fn legacy_derive_key() -> [u8; 32] {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let hash = Sha256::digest(format!("cleanclaw:{hostname}:{home}").as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    key
}

fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> std::result::Result<Vec<u8>, CredentialError> {
    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| CredentialError::Ring("key".into()))?;
    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes).map_err(|_| CredentialError::Ring("nonce".into()))?;
    let mut sealing = SealingKey::new(unbound, OneNonce(Some(nonce_bytes)));
    let mut in_out = plaintext.to_vec();
    sealing
        .seal_in_place_append_tag(Aad::empty(), &mut in_out)
        .map_err(|_| CredentialError::Ring("seal".into()))?;
    let mut out = Vec::with_capacity(12 + in_out.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&in_out);
    Ok(out)
}

fn decrypt(ciphertext: &[u8], key: &[u8; 32]) -> std::result::Result<Vec<u8>, CredentialError> {
    if ciphertext.len() < 12 + 16 {
        return Err(CredentialError::Decrypt);
    }
    let (nonce_bytes, ct) = ciphertext.split_at(12);
    let mut nonce_arr = [0u8; 12];
    nonce_arr.copy_from_slice(nonce_bytes);
    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| CredentialError::Ring("key".into()))?;
    let mut opening = ring::aead::OpeningKey::new(unbound, OneNonce(Some(nonce_arr)));
    let mut in_out = ct.to_vec();
    let plain = opening
        .open_in_place(Aad::empty(), &mut in_out)
        .map_err(|_| CredentialError::Decrypt)?;
    Ok(plain.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // We can't easily test for_user() in parallel because it
    // touches $HOME. Serialize all credential tests through one
    // mutex.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_tmp_home<F: FnOnce(PathBuf)>(f: F) {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());
        f(dir.path().to_path_buf());
        if let Some(p) = prev {
            std::env::set_var("HOME", p);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn set_get_round_trip() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("openai", "apiKey", "sk-test-1234").unwrap();
            let got = cm.get("openai", "apiKey").unwrap();
            assert_eq!(got, "sk-test-1234");
        });
    }

    #[test]
    fn list_masks_values() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("openai", "apiKey", "sk-abcdefghijklmnop").unwrap();
            let listed = cm.list().unwrap();
            assert_eq!(listed.len(), 1);
            let v = listed[0].keys.get("apiKey").unwrap();
            assert!(v.contains("..."), "value should be masked: {v}");
            assert!(!v.contains("sk-abcdefghijklmnop"), "must not leak: {v}");
        });
    }

    #[test]
    fn delete_removes_credential() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("openai", "apiKey", "x").unwrap();
            cm.delete("openai").unwrap();
            assert!(cm.get("openai", "apiKey").is_err());
        });
    }

    #[test]
    fn encrypted_at_rest() {
        with_tmp_home(|home| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("openai", "apiKey", "sk-secret-99").unwrap();
            // The plaintext key must NOT appear on disk.
            let cred_path = home.join(".cleanclaw").join("users").join("u1").join("credentials.json");
            let bytes = std::fs::read(&cred_path).unwrap();
            let s = String::from_utf8_lossy(&bytes);
            assert!(!s.contains("sk-secret-99"), "credential must be encrypted: {s}");
        });
    }

    #[test]
    fn reload_recovers() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("openai", "apiKey", "sk-x").unwrap();
            drop(cm);
            // Re-open: the file should decrypt and entries restore.
            let cm2 = CredentialManager::for_user("u1", "pp").unwrap();
            assert_eq!(cm2.get("openai", "apiKey").unwrap(), "sk-x");
        });
    }

    #[test]
    fn wrong_passphrase_cannot_decrypt() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp-A").unwrap();
            cm.set("openai", "apiKey", "sk-x").unwrap();
            // A new manager with a different passphrase can't
            // see the same entries (decryption fails on load —
            // we silently fall back to empty rather than
            // crashing).
            let cm2 = CredentialManager::for_user("u1", "pp-B").unwrap();
            assert!(cm2.get("openai", "apiKey").is_err());
        });
    }

    #[test]
    fn inject_env_maps_known_providers() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("openai", "apiKey", "sk-1").unwrap();
            let env = cm.inject_env().unwrap();
            assert_eq!(env.get("OPENAI_API_KEY").map(|s| s.as_str()), Some("sk-1"));
        });
    }

    #[test]
    fn inject_env_uppercases_unknown_providers() {
        with_tmp_home(|_| {
            let cm = CredentialManager::for_user("u1", "pp").unwrap();
            cm.set("customthing", "apiKey", "k-1").unwrap();
            let env = cm.inject_env().unwrap();
            assert_eq!(env.get("CUSTOMTHING_API_KEY").map(|s| s.as_str()), Some("k-1"));
        });
    }

    #[test]
    fn discover_returns_set_env() {
        with_tmp_home(|_| {
            let prev = std::env::var("OPENAI_API_KEY").ok();
            std::env::set_var("OPENAI_API_KEY", "sk-discovered");
            let found = CredentialManager::discover();
            assert!(found.iter().any(|c| c.name == "openai" && c.keys.get("apiKey").map(|v| v.as_str()) == Some("sk-discovered")));
            if let Some(p) = prev {
                std::env::set_var("OPENAI_API_KEY", p);
            } else {
                std::env::remove_var("OPENAI_API_KEY");
            }
        });
    }

    #[test]
    fn empty_user_id_rejected() {
        assert!(CredentialManager::for_user("", "").is_err());
    }
}
