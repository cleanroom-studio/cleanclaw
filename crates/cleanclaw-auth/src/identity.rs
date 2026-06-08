//! Resolved caller for one HTTP request.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyType {
    Admin,
    User,
    Agent,
}

impl ApiKeyType {
    pub fn parse(s: &str) -> Self {
        match s {
            "admin" => Self::Admin,
            "user" => Self::User,
            _ => Self::Agent,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    SuperAdmin,
    Admin,
    User,
}

impl Role {
    pub fn parse(s: &str) -> Self {
        match s {
            "super_admin" => Self::SuperAdmin,
            "admin" => Self::Admin,
            _ => Self::User,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SuperAdmin => "super_admin",
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::SuperAdmin | Self::Admin)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub user_id: String,
    pub role: Role,
    pub method: AuthMethod,
    pub api_key_id: String,
    pub api_key_type: Option<ApiKeyType>,
    pub api_key_agents: Vec<String>,
}

impl Identity {
    /// Empty identity — only used for `Option<Identity>` in axum
    /// extensions when a handler runs unauthenticated (onboard, login,
    /// status). All other paths go through `Resolver::middleware`.
    pub fn anonymous() -> Self {
        Self {
            user_id: String::new(),
            role: Role::User,
            method: AuthMethod::Session,
            api_key_id: String::new(),
            api_key_type: None,
            api_key_agents: Vec::new(),
        }
    }

    /// May this caller hit platform-wide mutating endpoints
    /// (`/api/admin/*`)? Only super_admin sessions and type=admin apikeys.
    pub fn can_admin_platform(&self) -> bool {
        match self.method {
            AuthMethod::ApiKey => matches!(self.api_key_type, Some(ApiKeyType::Admin)),
            AuthMethod::Session => matches!(self.role, Role::SuperAdmin),
        }
    }

    pub fn is_super_admin(&self) -> bool {
        self.method == AuthMethod::Session && self.role == Role::SuperAdmin
    }

    /// May this caller create new agents? `type=agent` keys explicitly
    /// cannot — they're sandboxed to a fixed list.
    pub fn can_create_agent(&self) -> bool {
        if self.method == AuthMethod::ApiKey && self.api_key_type == Some(ApiKeyType::Agent) {
            return false;
        }
        true
    }

    /// May this caller talk to `agent_id`?
    pub fn can_access_agent(&self, agent_id: &str) -> bool {
        if self.method == AuthMethod::ApiKey {
            if self.api_key_type == Some(ApiKeyType::Admin) {
                return true;
            }
            return self.api_key_agents.iter().any(|a| a == agent_id);
        }
        if self.role == Role::SuperAdmin {
            return true;
        }
        // Session caller: handler is expected to verify ownership via
        // the agents table (cheap M:1). Allow here.
        true
    }
}
