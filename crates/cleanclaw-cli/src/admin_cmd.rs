//! `cleanclaw admin …` — operator-only DB operations that bypass the
//! HTTP API.
//!
//! These commands
//! exist so a super-admin who's locked out of the dashboard can
//! recover their account from the shell.

use clap::Subcommand;
use cleanclaw_auth::users::{Accounts, CreateInput, ROLE_APP_USER, ROLE_SUPER_ADMIN, ROLE_USER, STATUS_ACTIVE};
use cleanclaw_auth::UserError;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_store::Store;
use std::sync::Arc;

use crate::agents_cmd::open_store;

#[derive(Subcommand)]
pub enum AdminCmd {
    /// Create a new user account.
    CreateUser {
        #[arg(long)]
        username: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long, default_value = "user")]
        role: String,
        /// -1 = unlimited, 0 = no self-creation, N>0 = up to N owned agents.
        #[arg(long)]
        agent_quota: Option<i64>,
    },
    /// Reset a user's password (operator recovery path).
    ResetPassword {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
    },
    /// Grant a different role to an existing user.
    GrantRole {
        #[arg(long)]
        username: String,
        #[arg(long)]
        role: String,
    },
    /// List all users in the local store.
    ListUsers {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        role: Option<String>,
    },
}

pub async fn run(cmd: AdminCmd) -> Result<()> {
    let store = open_store().await?;
    match cmd {
        AdminCmd::CreateUser {
            username,
            email,
            password: pw,
            display_name,
            role,
            agent_quota,
        } => {
            create_user(store, username, email, pw, display_name, role, agent_quota).await
        }
        AdminCmd::ResetPassword { username, password: pw } => {
            reset_password(store, username, pw).await
        }
        AdminCmd::GrantRole { username, role } => grant_role(store, username, role).await,
        AdminCmd::ListUsers { status, role } => list_users(store, status, role).await,
    }
}

async fn create_user(
    store: Arc<dyn Store>,
    username: String,
    email: String,
    pw: String,
    display_name: Option<String>,
    role: String,
    agent_quota: Option<i64>,
) -> Result<()> {
    if pw.len() < 8 {
        return Err(CleanClawError::InvalidArgument(
            "password must be at least 8 characters".into(),
        ));
    }
    let role_str = parse_role(&role)?;
    let accts = Accounts::new(store).map_err(ue)?;
    let display = display_name.unwrap_or_else(|| username.clone());
    let input = CreateInput {
        username,
        email,
        password: pw,
        display_name: display,
        role: role_str,
        agent_quota,
        avatar_url: String::new(),
        apikey_id: String::new(),
        external_id: String::new(),
    };
    let acct = accts.create(input).await.map_err(ue)?;
    println!("created user {} (id={})", acct.username, acct.id);
    Ok(())
}

async fn reset_password(
    store: Arc<dyn Store>,
    username: String,
    pw: String,
) -> Result<()> {
    if pw.len() < 8 {
        return Err(CleanClawError::InvalidArgument(
            "password must be at least 8 characters".into(),
        ));
    }
    let accts = Accounts::new(store).map_err(ue)?;
    let users = accts.list().await.map_err(ue)?;
    let user = users
        .into_iter()
        .find(|u| u.username == username)
        .ok_or_else(|| CleanClawError::NotFound(format!("user {username}")))?;
    accts.set_password(&user.id, &pw).await.map_err(ue)?;
    println!("reset password for {username}");
    Ok(())
}

async fn grant_role(
    store: Arc<dyn Store>,
    username: String,
    role: String,
) -> Result<()> {
    let role_str = parse_role(&role)?;
    let accts = Accounts::new(store).map_err(ue)?;
    let users = accts.list().await.map_err(ue)?;
    let user = users
        .into_iter()
        .find(|u| u.username == username)
        .ok_or_else(|| CleanClawError::NotFound(format!("user {username}")))?;
    // `update` takes explicit positional args; keep all the other
    // fields stable by reading them from the current row. Pass an
    // empty `display_name` so the update keeps the existing value.
    accts.update(
        &user.id,
        "",
        &role_str,
        "",
        Some(user.agent_quota),
    )
    .await
    .map_err(ue)?;
    println!("{username} → {role_str}");
    Ok(())
}

async fn list_users(
    store: Arc<dyn Store>,
    status: Option<String>,
    role: Option<String>,
) -> Result<()> {
    let accts = Accounts::new(store).map_err(ue)?;
    let users = accts.list().await.map_err(ue)?;
    let filtered: Vec<_> = users
        .into_iter()
        .filter(|u| status.as_deref().map(|s| s == u.status).unwrap_or(true))
        .filter(|u| role.as_deref().map(|r| r == u.role).unwrap_or(true))
        .collect();
    if filtered.is_empty() {
        println!("(no users match the filter)");
        return Ok(());
    }
    println!("{:<24} {:<32} {:<14} {}", "USERNAME", "EMAIL", "ROLE", "STATUS");
    for u in filtered {
        println!("{:<24} {:<32} {:<14} {}", u.username, u.email, u.role, u.status);
    }
    Ok(())
}

fn parse_role(s: &str) -> Result<String> {
    match s {
        "super_admin" | "super-admin" | "superadmin" => Ok(ROLE_SUPER_ADMIN.into()),
        "app_user" | "app-user" | "appuser" => Ok(ROLE_APP_USER.into()),
        "user" => Ok(ROLE_USER.into()),
        _ => Err(CleanClawError::InvalidArgument(format!("unknown role: {s}"))),
    }
}

/// Convert `UserError` → `CleanClawError`. Most variants carry a
/// payload; we map them to the closest existing variant.
fn ue(e: UserError) -> CleanClawError {
    use UserError::*;
    match e {
        InvalidCredentials => CleanClawError::Unauthorized,
        InvalidRole(r) => CleanClawError::InvalidArgument(format!("invalid role: {r}")),
        InvalidStatus(s) => CleanClawError::InvalidArgument(format!("invalid status: {s}")),
        LastSuperAdmin => CleanClawError::Conflict("last super admin".into()),
        Missing(f) => CleanClawError::InvalidArgument(format!("missing field: {f}")),
        Store(s) => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_role_known_values() {
        assert_eq!(parse_role("super_admin").unwrap(), "super_admin");
        assert_eq!(parse_role("app_user").unwrap(), "app_user");
        assert_eq!(parse_role("user").unwrap(), "user");
    }

    #[test]
    fn parse_role_accepts_dashed_aliases() {
        assert_eq!(parse_role("super-admin").unwrap(), "super_admin");
        assert_eq!(parse_role("app-user").unwrap(), "app_user");
    }

    #[test]
    fn parse_role_rejects_unknown() {
        assert!(parse_role("root").is_err());
    }

    #[test]
    fn ue_maps_variants() {
        assert!(matches!(ue(UserError::InvalidCredentials), CleanClawError::Unauthorized));
        assert!(matches!(ue(UserError::LastSuperAdmin), CleanClawError::Conflict(_)));
    }
}
