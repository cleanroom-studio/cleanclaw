//! Context builder — assembles the system prompt from SOUL.md,
//! IDENTITY.md, USER.md, MEMORY.md, and the runtime tool/skill catalog.
//!
//!

use cleanclaw_core::Result;
use cleanclaw_skills::{render_always_loaded, render_prompt, Skill};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityFiles {
    /// The agent's SOUL.md / IDENTITY.md / etc., read from the store.
    pub soul: String,
    pub identity: String,
    pub user: String,
    pub memory: String,
    pub agents: String,
    pub bootstrap: String,
    pub tools_md: String,
    pub heartbeat: String,
    pub agent_json: String,
}

impl IdentityFiles {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Read every agent file via the store. If `chatter_user_id` is
    /// non-empty, the owner-fallback overlay is applied — see
    /// `Store::get_workspace_file`.
    pub async fn load<S: IdentityFileStore>(
        store: &S,
        agent_id: &str,
        owner_user_id: &str,
        chatter_user_id: &str,
    ) -> Result<Self> {
        async fn fetch_one<S: IdentityFileStore>(
            store: &S,
            agent_id: &str,
            owner_user_id: &str,
            chatter_user_id: &str,
            filename: &'static str,
        ) -> Result<String> {
            store
                .read(agent_id, owner_user_id, chatter_user_id, filename)
                .await
                .map(|opt| opt.unwrap_or_default())
        }
        let soul = fetch_one(store, agent_id, owner_user_id, chatter_user_id, "SOUL.md").await?;
        let identity = fetch_one(
            store,
            agent_id,
            owner_user_id,
            chatter_user_id,
            "IDENTITY.md",
        )
        .await?;
        let user = fetch_one(store, agent_id, owner_user_id, chatter_user_id, "USER.md").await?;
        let memory =
            fetch_one(store, agent_id, owner_user_id, chatter_user_id, "MEMORY.md").await?;
        let agents =
            fetch_one(store, agent_id, owner_user_id, chatter_user_id, "AGENTS.md").await?;
        let bootstrap = fetch_one(
            store,
            agent_id,
            owner_user_id,
            chatter_user_id,
            "BOOTSTRAP.md",
        )
        .await?;
        let tools_md =
            fetch_one(store, agent_id, owner_user_id, chatter_user_id, "TOOLS.md").await?;
        let heartbeat = fetch_one(
            store,
            agent_id,
            owner_user_id,
            chatter_user_id,
            "HEARTBEAT.md",
        )
        .await?;
        let agent_json = fetch_one(
            store,
            agent_id,
            owner_user_id,
            chatter_user_id,
            "agent.json",
        )
        .await?;
        Ok(Self {
            soul,
            identity,
            user,
            memory,
            agents,
            bootstrap,
            tools_md,
            heartbeat,
            agent_json,
        })
    }
}

#[async_trait::async_trait]
pub trait IdentityFileStore: Send + Sync {
    /// Returns the file content if present, else `Ok(None)`.
    async fn read(
        &self,
        agent_id: &str,
        owner_user_id: &str,
        chatter_user_id: &str,
        filename: &str,
    ) -> Result<Option<String>>;
}

pub struct ContextBuilder {
    pub now_iso: String,
    pub timezone: String,
    pub env: HashMap<String, String>,
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self {
            now_iso: chrono::Utc::now().to_rfc3339(),
            timezone: "UTC".into(),
            env: HashMap::new(),
        }
    }

    /// Render the system prompt. Order matches the CleanClaw convention
    /// (date anchor → identity scaffolding → skills).
    pub fn build(&self, files: &IdentityFiles, skills: &[Skill], tools_section: &str) -> String {
        let mut out = String::new();

        // 1. Date / timezone anchor.
        out.push_str(&format!(
            "# Today\n\nUTC now: `{}`\nTimezone: `{}`\n",
            self.now_iso, self.timezone
        ));

        // 2. SOUL.md / IDENTITY.md.
        if !files.soul.is_empty() {
            out.push_str("\n# Soul\n\n");
            out.push_str(&files.soul);
            out.push('\n');
        }
        if !files.identity.is_empty() {
            out.push_str("\n# Identity\n\n");
            out.push_str(&files.identity);
            out.push('\n');
        }

        // 3. Agent files (BOOTSTRAP / AGENTS / TOOLS / HEARTBEAT).
        for (label, body) in [
            ("Bootstrap", &files.bootstrap),
            ("Agents", &files.agents),
            ("Tools", &files.tools_md),
            ("Heartbeat", &files.heartbeat),
        ] {
            if !body.is_empty() {
                out.push_str(&format!("\n# {label}\n\n"));
                out.push_str(body);
                out.push('\n');
            }
        }

        // 4. Per-chatter files (USER.md / MEMORY.md).
        if !files.user.is_empty() {
            out.push_str("\n# User\n\n");
            out.push_str(&files.user);
            out.push('\n');
        }
        if !files.memory.is_empty() {
            out.push_str("\n# Memory\n\n");
            out.push_str(&files.memory);
            out.push('\n');
        }

        // 5. Skills catalog.
        let skills_prompt = render_prompt(skills);
        if !skills_prompt.is_empty() {
            out.push_str(&skills_prompt);
        }
        let always = render_always_loaded(skills);
        if !always.is_empty() {
            out.push_str(&always);
        }

        // 6. Tools catalog (built by the runner).
        if !tools_section.is_empty() {
            out.push_str("\n# Tools\n\n");
            out.push_str(tools_section);
        }

        out
    }
}

// Re-export so the runner module can construct a `ContextBuilder`.
pub use std::sync::Arc as _Arc;

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_identity_renders_minimal_prompt() {
        let cb = ContextBuilder::new();
        let files = IdentityFiles::empty();
        let p = cb.build(&files, &[], "");
        assert!(p.contains("UTC now"));
        assert!(!p.contains("# Soul"));
    }

    #[test]
    fn full_identity_picks_up_every_section() {
        let cb = ContextBuilder::new();
        let mut files = IdentityFiles::empty();
        files.soul = "be kind".into();
        files.identity = "named alpha".into();
        files.user = "loves tea".into();
        files.memory = "met on Tuesday".into();
        files.bootstrap = "always greet".into();
        files.agents = "use 2-space".into();
        files.tools_md = "prefer ripgrep".into();
        files.heartbeat = "check-in hourly".into();
        let p = cb.build(&files, &[], "Tools: echo, current_time");
        assert!(p.contains("# Soul"));
        assert!(p.contains("# Identity"));
        assert!(p.contains("# User"));
        assert!(p.contains("# Memory"));
        assert!(p.contains("# Bootstrap"));
        assert!(p.contains("# Agents"));
        assert!(p.contains("# Tools"));
        assert!(p.contains("be kind"));
    }
}
