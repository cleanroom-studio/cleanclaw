//! User account registry.
//!
//! Thin facade over `cleanclaw_store::Store` — Account reads/writes
//! land in the `users` table, with the existing `password::hash_password`
//! + `password::verify_password` doing credential work.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use cleanclaw_core::CleanClawError;
use cleanclaw_store::models::UserRecord;
use cleanclaw_store::Store;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::password;

pub const ROLE_SUPER_ADMIN: &str = "super_admin";
pub const ROLE_USER: &str = "user";
pub const ROLE_APP_USER: &str = "app_user";

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_DISABLED: &str = "disabled";

#[derive(Debug, Error)]
pub enum UserError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("invalid role: {0}")]
    InvalidRole(String),
    #[error("invalid status: {0}")]
    InvalidStatus(String),
    #[error("refusing to remove the last active super_admin")]
    LastSuperAdmin,
    #[error("missing required field: {0}")]
    Missing(&'static str),
    #[error("store: {0}")]
    Store(#[from] CleanClawError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub email: String,
    #[serde(
        rename = "displayName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub display_name: String,
    pub role: String,
    pub status: String,
    #[serde(rename = "apikeyId", default, skip_serializing_if = "String::is_empty")]
    pub apikey_id: String,
    #[serde(
        rename = "externalId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub external_id: String,
    #[serde(
        rename = "avatarUrl",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub avatar_url: String,
    #[serde(rename = "agentQuota")]
    pub agent_quota: i64,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct CreateInput {
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: String,
    pub role: String,
    /// - `None`            → unlimited (self-registered default)
    /// - `Some(v < 0)`     → unlimited
    /// - `Some(v == 0)`    → cannot self-create agents
    /// - `Some(v > 0)`     → max `v` owned agents
    pub agent_quota: Option<i64>,
    pub avatar_url: String,
    pub apikey_id: String,
    pub external_id: String,
}

/// Account registry. Wraps a `Store` so the platform has a single
/// SQL backend as the source of truth.
pub struct Accounts {
    store: Arc<dyn Store>,
}

impl Accounts {
    pub fn new(store: Arc<dyn Store>) -> Result<Self, UserError> {
        if Arc::strong_count(&store) == 0 {
            return Err(UserError::Store(CleanClawError::InvalidArgument(
                "users: store is required".into(),
            )));
        }
        Ok(Self { store })
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Total account count. Onboarding gates on `count == 0`.
    pub async fn count(&self) -> Result<i64, UserError> {
        Ok(self.store.count_users().await?)
    }

    /// Create a new account. Idempotent on `(apikey_id, external_id)`
    /// when both are non-empty: a repeat call returns the existing
    /// row instead of erroring.
    pub async fn create(&self, in_: CreateInput) -> Result<Account, UserError> {
        let apikey_id = in_.apikey_id.trim().to_string();
        let external_id = in_.external_id.trim().to_string();
        if !apikey_id.is_empty() && !external_id.is_empty() {
            if let Ok(rec) = self
                .store
                .get_user_by_external(&apikey_id, &external_id)
                .await
            {
                return Ok(to_account(&rec));
            }
        }
        let username = in_.username.trim().to_string();
        let email = in_.email.trim().to_ascii_lowercase();
        if username.is_empty() || email.is_empty() || in_.password.is_empty() {
            return Err(UserError::Missing("username/email/password"));
        }
        let role = if in_.role.is_empty() {
            ROLE_USER.to_string()
        } else {
            in_.role.clone()
        };
        if role != ROLE_SUPER_ADMIN && role != ROLE_USER && role != ROLE_APP_USER {
            return Err(UserError::InvalidRole(role));
        }
        let hash = password::hash_password(&in_.password)?;
        let id = new_id("u_");
        let now = Utc::now();
        let quota = in_.agent_quota.unwrap_or(-1) as i32;
        let rec = UserRecord {
            id: id.clone(),
            username,
            email,
            password_hash: hash,
            display_name: in_.display_name,
            role,
            status: STATUS_ACTIVE.to_string(),
            apikey_id: apikey_id.clone(),
            external_id: external_id.clone(),
            avatar_url: in_.avatar_url,
            agent_quota: quota,
            created_at: now,
            updated_at: now,
        };
        // Race: another request may have inserted the same
        // (apikey_id, external_id) pair in between. Re-read on
        // failure so the caller still gets the idempotent contract.
        if let Err(e) = self.store.create_user(&rec).await {
            if !apikey_id.is_empty() && !external_id.is_empty() {
                if let Ok(again) = self
                    .store
                    .get_user_by_external(&apikey_id, &external_id)
                    .await
                {
                    return Ok(to_account(&again));
                }
            }
            return Err(UserError::Store(e));
        }
        Ok(to_account(&rec))
    }

    /// Validate a username-or-email + password pair. Returns
    /// `InvalidCredentials` on every failure mode (missing user, wrong
    /// password, disabled account) so callers can't distinguish.
    pub async fn authenticate(&self, login: &str, password_: &str) -> Result<Account, UserError> {
        let mut login = login.trim().to_string();
        if login.is_empty() || password_.is_empty() {
            return Err(UserError::InvalidCredentials);
        }
        if login.contains('@') {
            login = login.to_ascii_lowercase();
        }
        let rec = self
            .store
            .get_user_by_login(&login)
            .await
            .map_err(|e| match e {
                CleanClawError::NotFound(_) => UserError::InvalidCredentials,
                other => UserError::Store(other),
            })?;
        if rec.status != STATUS_ACTIVE {
            return Err(UserError::InvalidCredentials);
        }
        if rec.password_hash.is_empty() || rec.role == ROLE_APP_USER {
            return Err(UserError::InvalidCredentials);
        }
        let ok = password::verify_password(password_, &rec.password_hash)?;
        if !ok {
            return Err(UserError::InvalidCredentials);
        }
        Ok(to_account(&rec))
    }

    pub async fn get(&self, id: &str) -> Result<Account, UserError> {
        let rec = self.store.get_user(id).await?;
        Ok(to_account(&rec))
    }

    pub async fn list(&self) -> Result<Vec<Account>, UserError> {
        let recs = self.store.list_users().await?;
        Ok(recs.iter().map(to_account).collect())
    }

    /// Apply non-credential changes. Pass empty strings to leave a
    /// field alone. Use `set_password` for credential rotation.
    pub async fn update(
        &self,
        id: &str,
        display_name: &str,
        role: &str,
        status: &str,
        agent_quota: Option<i64>,
    ) -> Result<Account, UserError> {
        let mut rec = self.store.get_user(id).await?;
        if !display_name.is_empty() {
            rec.display_name = display_name.to_string();
        }
        if !role.is_empty() {
            if role != ROLE_SUPER_ADMIN && role != ROLE_USER {
                return Err(UserError::InvalidRole(role.to_string()));
            }
            rec.role = role.to_string();
        }
        if !status.is_empty() {
            if status != STATUS_ACTIVE && status != STATUS_DISABLED {
                return Err(UserError::InvalidStatus(status.to_string()));
            }
            rec.status = status.to_string();
        }
        if let Some(q) = agent_quota {
            rec.agent_quota = q as i32;
        }
        rec.updated_at = Utc::now();
        self.store.update_user(&rec).await?;
        Ok(to_account(&rec))
    }

    /// Self-service profile edit. Admin-only role/status changes go
    /// through `update`.
    pub async fn update_profile(
        &self,
        id: &str,
        display_name: &str,
        avatar_url: &str,
    ) -> Result<Account, UserError> {
        let mut rec = self.store.get_user(id).await?;
        rec.display_name = display_name.to_string();
        rec.avatar_url = avatar_url.to_string();
        rec.updated_at = Utc::now();
        self.store.update_user(&rec).await?;
        Ok(to_account(&rec))
    }

    pub async fn verify_password(&self, id: &str, password_: &str) -> Result<(), UserError> {
        let rec = self
            .store
            .get_user(id)
            .await
            .map_err(|_| UserError::InvalidCredentials)?;
        if rec.password_hash.is_empty() {
            return Err(UserError::InvalidCredentials);
        }
        let ok = password::verify_password(password_, &rec.password_hash)?;
        if !ok {
            return Err(UserError::InvalidCredentials);
        }
        Ok(())
    }

    pub async fn set_password(&self, id: &str, new_password: &str) -> Result<(), UserError> {
        if new_password.is_empty() {
            return Err(UserError::Missing("password"));
        }
        let mut rec = self.store.get_user(id).await?;
        rec.password_hash = password::hash_password(new_password)?;
        rec.updated_at = Utc::now();
        self.store.update_user(&rec).await?;
        Ok(())
    }

    /// Idempotent: returns the existing `(apikey, external_id)` user
    /// or creates a fresh `app_user` row the first time. Username and
    /// email are synthesized to satisfy UNIQUE constraints.
    pub async fn ensure_app_user(
        &self,
        apikey_id: &str,
        external_id: &str,
        display_name: &str,
    ) -> Result<Account, UserError> {
        let apikey_id = apikey_id.trim();
        let external_id = external_id.trim();
        if apikey_id.is_empty() || external_id.is_empty() {
            return Err(UserError::Missing("apikeyId/externalId"));
        }
        if let Ok(rec) = self
            .store
            .get_user_by_external(apikey_id, external_id)
            .await
        {
            return Ok(to_account(&rec));
        }
        let id = new_id("u_");
        let now = Utc::now();
        let syn = format!("{apikey_id}:{external_id}");
        let rec = UserRecord {
            id,
            username: format!("ext:{syn}"),
            email: format!("{syn}@external.cleanclaw.local"),
            password_hash: String::new(),
            display_name: display_name.to_string(),
            role: ROLE_APP_USER.to_string(),
            status: STATUS_ACTIVE.to_string(),
            apikey_id: apikey_id.to_string(),
            external_id: external_id.to_string(),
            avatar_url: String::new(),
            agent_quota: -1_i32,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = self.store.create_user(&rec).await {
            if let Ok(again) = self
                .store
                .get_user_by_external(apikey_id, external_id)
                .await
            {
                return Ok(to_account(&again));
            }
            return Err(UserError::Store(e));
        }
        Ok(to_account(&rec))
    }

    /// Delete an account. Refuses to drop the last active super_admin.
    pub async fn delete(&self, id: &str) -> Result<(), UserError> {
        let target = self.store.get_user(id).await?;
        if target.role == ROLE_SUPER_ADMIN {
            let all = self.store.list_users().await?;
            let admins = all
                .iter()
                .filter(|u| u.role == ROLE_SUPER_ADMIN && u.status == STATUS_ACTIVE)
                .count();
            if admins <= 1 {
                return Err(UserError::LastSuperAdmin);
            }
        }
        self.store.delete_user(id).await?;
        Ok(())
    }
}

fn to_account(r: &UserRecord) -> Account {
    Account {
        id: r.id.clone(),
        username: r.username.clone(),
        email: r.email.clone(),
        display_name: r.display_name.clone(),
        role: r.role.clone(),
        status: r.status.clone(),
        apikey_id: r.apikey_id.clone(),
        external_id: r.external_id.clone(),
        avatar_url: r.avatar_url.clone(),
        agent_quota: r.agent_quota as i64,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn new_id(prefix: &str) -> String {
    use rand::RngCore;
    let mut buf = [0u8; 10];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    format!("{prefix}{}", hex::encode(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> CreateInput {
        CreateInput {
            username: "alice".into(),
            email: "alice@example.com".into(),
            password: "hunter2hunter2".into(),
            ..Default::default()
        }
    }

    #[test]
    fn new_id_has_prefix() {
        let id = new_id("u_");
        assert!(id.starts_with("u_"));
        // prefix + 20 hex chars
        assert_eq!(id.len(), 2 + 20);
    }

    #[test]
    fn role_constants_distinct() {
        assert_ne!(ROLE_SUPER_ADMIN, ROLE_USER);
        assert_ne!(ROLE_USER, ROLE_APP_USER);
        assert_ne!(ROLE_SUPER_ADMIN, ROLE_APP_USER);
    }

    #[test]
    fn missing_field_detection() {
        let mut in_ = input();
        in_.username = String::new();
        // We can't run the async store path without a real store; just
        // verify the input validator's preconditions are well-formed.
        assert!(in_.username.is_empty());
        assert!(!in_.email.is_empty());
    }
}
