//! Slash command dispatcher.
//!
//! Recognized prefixes (case-insensitive):
//!   /new            — start a new session (clear history, return ack)
//!   /undo           — drop the last user+assistant pair
//!   /retry          — drop the last assistant turn, re-run with same user
//!   /compact        — force compaction
//!   /model <name>   — switch model mid-session (stored on the agent record)
//!   /personality <name> — set the prompt-mode override
//!
//! All other inputs are passed through as ordinary user text.

use chrono::Utc;
use cleanclaw_core::Result;
use cleanclaw_provider::Message;
use cleanclaw_store::models::SessionRecord;
use cleanclaw_store::Store;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashOutcome {
    /// Run the turn as normal — the input wasn't a slash command.
    NotASlash,
    /// A command was recognized. The agent loop should `continue` to
    /// the next user turn (the reply is in `reply`).
    Handled { reply: String },
    /// A command requires session manipulation before the next turn.
    ResetSession,
    /// Drop the last user/assistant exchange (used by /undo).
    DropLastExchange,
    /// Drop the last assistant turn (used by /retry). The user's
    /// preceding turn stays.
    DropLastAssistant,
}

pub struct SlashResult {
    pub outcome: SlashOutcome,
    pub reply: String,
}

pub fn dispatch(input: &str) -> SlashResult {
    let trimmed = input.trim();
    let mut parts = trimmed.split_whitespace();
    let head = match parts.next() {
        Some(s) => s,
        None => {
            return SlashResult {
                outcome: SlashOutcome::NotASlash,
                reply: String::new(),
            }
        }
    };
    if !head.starts_with('/') {
        return SlashResult {
            outcome: SlashOutcome::NotASlash,
            reply: String::new(),
        };
    }
    let cmd = head.to_ascii_lowercase();
    let rest: String = parts.collect::<Vec<&str>>().join(" ");
    match cmd.as_str() {
        "/new" => SlashResult {
            outcome: SlashOutcome::ResetSession,
            reply: "[slash] new session — previous history cleared.".into(),
        },
        "/undo" => SlashResult {
            outcome: SlashOutcome::DropLastExchange,
            reply: "[slash] undo — last user/assistant exchange removed.".into(),
        },
        "/retry" => SlashResult {
            outcome: SlashOutcome::DropLastAssistant,
            reply: "[slash] retry — last assistant turn removed; rerunning.".into(),
        },
        "/compact" => SlashResult {
            outcome: SlashOutcome::Handled {
                reply: "[slash] compact requested.".into(),
            },
            reply: "[slash] compact requested — running compaction on next turn.".into(),
        },
        "/model" => SlashResult {
            outcome: SlashOutcome::Handled {
                reply: format!("[slash] model switch requested: {rest}"),
            },
            reply: format!(
                "[slash] model switch requested: {rest}. (Note: runtime model swap persists on the agent record; the next turn will pick it up.)"
            ),
        },
        "/personality" => SlashResult {
            outcome: SlashOutcome::Handled {
                reply: format!("[slash] personality requested: {rest}"),
            },
            reply: format!("[slash] personality switch requested: {rest}"),
        },
        "/help" | "/?" => SlashResult {
            outcome: SlashOutcome::Handled {
                reply: SLASH_HELP.to_string(),
            },
            reply: SLASH_HELP.to_string(),
        },
        // The /goal slash has its own async dispatcher (it
        // touches the store for create/pause/resume/clear/show).
        // The synchronous `dispatch` returns a NotASlash sentinel
        // for it; the agent loop's pre-handler calls
        // `cleanclaw_agent::slash_goal::dispatch_goal` directly
        // when it sees a /goal prefix.
        "/goal" => SlashResult {
            outcome: SlashOutcome::NotASlash,
            reply: String::new(),
        },
        _ => SlashResult {
            outcome: SlashOutcome::NotASlash,
            reply: String::new(),
        },
    }
}

const SLASH_HELP: &str = "Slash commands:\n  /new — start a new session\n  /undo — drop the last user/assistant exchange\n  /retry — drop the last assistant turn, retry\n  /compact — force compaction\n  /model <name> — switch the model\n  /personality <agent|chatbot|customize> — switch prompt mode\n  /goal <objective> — set/refresh the long-running goal\n  /goal pause | resume | clear | show — manage the goal\n  /help — this message\n";

/// Apply the outcome to the persisted session row.
pub async fn apply_outcome(
    store: &Arc<dyn Store>,
    user_id: &str,
    agent_id: &str,
    session_key: &str,
    outcome: &SlashOutcome,
) -> Result<SlashResultOutcome> {
    let existing = store.get_session(user_id, agent_id, session_key).await.ok();
    let now = Utc::now();
    match outcome {
        SlashOutcome::NotASlash => Ok(SlashResultOutcome::Continue),
        SlashOutcome::Handled { reply } => {
            if let Some(rec) = &existing {
                let mut msgs: Vec<Message> = serde_json::from_value(rec.messages.clone()).unwrap_or_default();
                msgs.push(Message::system(reply));
                let updated = SessionRecord {
                    messages: serde_json::to_value(&msgs)?,
                    message_count: msgs.len() as i32,
                    updated_at: now,
                    ..rec.clone()
                };
                store.save_session(user_id, agent_id, session_key, &updated).await?;
            }
            Ok(SlashResultOutcome::Continue)
        }
        SlashOutcome::ResetSession => {
            // Save an empty session so the next turn starts clean.
            let rec = SessionRecord {
                user_id: user_id.to_string(),
                agent_id: agent_id.to_string(),
                session_key: session_key.to_string(),
                channel: existing.as_ref().map(|r| r.channel.clone()).unwrap_or_default(),
                account_id: existing.as_ref().map(|r| r.account_id.clone()).unwrap_or_default(),
                chat_id: existing.as_ref().map(|r| r.chat_id.clone()).unwrap_or_default(),
                project_id: existing.as_ref().map(|r| r.project_id.clone()).unwrap_or_default(),
                title: existing.as_ref().map(|r| r.title.clone()).unwrap_or_default(),
                messages: serde_json::json!([]),
                message_count: 0,
                updated_at: now,
                chatter_user_id: existing.as_ref().map(|r| r.chatter_user_id.clone()).unwrap_or_default(),
            };
            store.save_session(user_id, agent_id, session_key, &rec).await?;
            Ok(SlashResultOutcome::Continue)
        }
        SlashOutcome::DropLastExchange => {
            if let Some(rec) = &existing {
                let mut msgs: Vec<Message> = serde_json::from_value(rec.messages.clone()).unwrap_or_default();
                // Drop up to the last user+assistant pair.
                let mut dropped = 0;
                while let Some(m) = msgs.last() {
                    if matches!(m.role, cleanclaw_provider::Role::User | cleanclaw_provider::Role::Assistant) {
                        msgs.pop();
                        dropped += 1;
                        if dropped >= 2 {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                let updated = SessionRecord {
                    messages: serde_json::to_value(&msgs)?,
                    message_count: msgs.len() as i32,
                    updated_at: now,
                    ..rec.clone()
                };
                store.save_session(user_id, agent_id, session_key, &updated).await?;
            }
            Ok(SlashResultOutcome::Continue)
        }
        SlashOutcome::DropLastAssistant => {
            if let Some(rec) = &existing {
                let mut msgs: Vec<Message> = serde_json::from_value(rec.messages.clone()).unwrap_or_default();
                // Drop the last assistant turn.
                if let Some(pos) = msgs.iter().rposition(|m| matches!(m.role, cleanclaw_provider::Role::Assistant)) {
                    msgs.remove(pos);
                }
                let updated = SessionRecord {
                    messages: serde_json::to_value(&msgs)?,
                    message_count: msgs.len() as i32,
                    updated_at: now,
                    ..rec.clone()
                };
                store.save_session(user_id, agent_id, session_key, &updated).await?;
            }
            Ok(SlashResultOutcome::Continue)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashResultOutcome {
    Continue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_a_slash() {
        let r = dispatch("hello world");
        assert_eq!(r.outcome, SlashOutcome::NotASlash);
    }

    #[test]
    fn slash_new_resets() {
        let r = dispatch("/new");
        assert_eq!(r.outcome, SlashOutcome::ResetSession);
        assert!(r.reply.contains("new session"));
    }

    #[test]
    fn slash_undo_drops_pair() {
        let r = dispatch("/undo");
        assert_eq!(r.outcome, SlashOutcome::DropLastExchange);
    }

    #[test]
    fn slash_retry_drops_assistant() {
        let r = dispatch("/retry");
        assert_eq!(r.outcome, SlashOutcome::DropLastAssistant);
    }

    #[test]
    fn slash_compact_handled() {
        let r = dispatch("/compact");
        assert!(matches!(r.outcome, SlashOutcome::Handled { .. }));
    }

    #[test]
    fn slash_model_with_arg() {
        let r = dispatch("/model openai/gpt-4o");
        assert!(matches!(r.outcome, SlashOutcome::Handled { .. }));
        assert!(r.reply.contains("openai/gpt-4o"));
    }

    #[test]
    fn slash_help() {
        let r = dispatch("/help");
        assert!(r.reply.contains("Slash commands"));
    }
}
