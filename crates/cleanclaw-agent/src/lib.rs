//! Agent core — ReAct loop, context builder, tool registry, plus
//! higher-level features (memory, compaction, hooks, heartbeat,
//! tool-recovery, attachments, slash commands, goals, bundled skills).
//!
//! (22 files, ~10 800 LoC
//! including tests). We split the surface across focused modules:
//!
//! - `loop_runner`     — ReAct loop, Agent, AgentBuilder, AgentManager
//! - `context`         — System prompt assembly
//! - `tools`           — Built-in tool implementations
//! - `event_hub`       — Streaming event bus (cron / heartbeat / SSE)
//! - `events`          — Structured event types
//! - `memory`          — MEMORY.md read/parse/write
//! - `compact`         — Session history compaction
//! - `hooks`           — Lifecycle hook registry
//! - `heartbeat`       — Periodic self-check tick
//! - `tool_recovery`   — Turn-failure tracking
//! - `attachments`     — Multimodal image attachments
//! - `slash`           — Slash command dispatcher
//! - `goal`            — Long-running goal subsystem
//! - `skills_full`     — Multi-root skill loader
//! - `bundled_skills`  — Bundled-skill installer
//! - `subagent_runtime` — Sub-agent driver

pub mod attachments;
pub mod bundled_skills;
pub mod compact;
pub mod context;
pub mod event_hub;
pub mod events;
pub mod goal;
pub mod goal_hook;
pub mod heartbeat;
pub mod hooks;
pub mod loop_runner;
pub mod manager;
pub mod memory;
pub mod sdkbridge;
pub mod skills_full;
pub mod skills_learner;
pub mod slash;
pub mod slash_goal;
pub mod subagent_runtime;
pub mod tool_recovery;
pub mod tools;

pub use attachments::{Attachment, AttachmentStore};
pub use bundled_skills::{bundled_hash, install_bundled, BUNDLED_SKILL_NAMES};
pub use compact::{compact_in_place, estimate_tokens, save_compacted, should_compact};
pub use context::{ContextBuilder, IdentityFileStore, IdentityFiles};
pub use event_hub::{AgentEvent, EventEnvelope, EventHub, SharedEventHub, Usage};
pub use events::AgentEventType;
pub use goal::{goal_context_prompt, GoalManager, GoalStatus};
pub use heartbeat::{HeartbeatConfig, HeartbeatScheduler, HeartbeatTick};
pub use hooks::{Hook, HookPhase, HookRegistry};
pub use loop_runner::{Agent, AgentBuilder, AgentOutput, TurnInput};
pub use manager::AgentManager;
pub use memory::{
    append_memory, compact_memory as compact_memory_file, distill_session, read_memory,
    MemoryStoreAdapter, SimpleMessage,
};
pub use skills_full::{LoadedSkills, SharedSkillsLoader, SkillsConfig, SkillsLoader};
pub use slash::{
    apply_outcome as apply_slash_outcome, dispatch as dispatch_slash, SlashOutcome, SlashResult,
    SlashResultOutcome,
};
pub use tool_recovery::{recover_tool_calls_from_text, RecoveredCall, TurnFailKey, TurnFailures};
pub use tools::builtins;
pub use tools::{Tool, ToolContext, ToolRegistry};
