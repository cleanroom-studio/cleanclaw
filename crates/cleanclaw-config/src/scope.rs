//! Scope tags for the `configs` table.
//!
//! Three layers: system (shared) > user (per-account) > agent (per-agent).
//! Child scopes shadow parent scopes by (kind, name).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    System,
    User,
    Agent,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::System => "system",
            Scope::User => "user",
            Scope::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Scope::System),
            "user" => Some(Scope::User),
            "agent" => Some(Scope::Agent),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for s in [Scope::System, Scope::User, Scope::Agent] {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
    }
}
