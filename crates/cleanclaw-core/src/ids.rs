use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_newtype {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            #[inline]
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            #[inline]
            pub fn as_str(&self) -> &str {
                &self.0
            }
            #[inline]
            pub fn prefix() -> &'static str {
                $prefix
            }
            pub fn generate() -> Self {
                Self(format!("{}{}", $prefix, ulid::Ulid::new()))
            }
            pub fn is_valid(s: &str) -> bool {
                s.starts_with($prefix) && s.len() > $prefix.len()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

id_newtype!(UserId, "u_");
id_newtype!(AgentId, "agt_");
id_newtype!(SessionKey, "sk_");
id_newtype!(ApiKeyId, "fk_");
id_newtype!(CronJobId, "cj_");
id_newtype!(ChannelId, "ch_");
id_newtype!(ProjectId, "p_");
id_newtype!(HookId, "hk_");
id_newtype!(PluginId, "plg_");
id_newtype!(MessageId, "m_");
id_newtype!(ChatId, "c_");
