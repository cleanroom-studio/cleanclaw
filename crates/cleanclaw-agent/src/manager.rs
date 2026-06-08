//! `manager.rs` is a thin alias — the real manager lives in
//! `loop_runner.rs` (paired with `AgentBuilder`). Kept as a separate
//! module so downstream code can `use cleanclaw_agent::AgentManager`.

pub use super::loop_runner::AgentManager;
