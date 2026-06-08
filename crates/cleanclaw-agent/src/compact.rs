//! Session history compaction.
//!
//! When the session's `messages` JSON blob grows past a threshold, the agent
//! loop asks the LLM to summarize the older turns into a single
//! "summary" message and truncates the in-memory working set. The
//! `session_messages` archive is left untouched — compaction is
//! purely a working-set optimization.
//!
//! For the first cut we implement the deterministic version:
//!   - keep the system prompt + the most recent N turns
//!   - drop everything in the middle, replacing it with a single
//!     "earlier conversation was compacted; this is a summary" message
//!
//! The LLM-driven summarization is a follow-up.

use chrono::Utc;
use cleanclaw_core::Result;
use cleanclaw_provider::Message;
use cleanclaw_store::models::SessionRecord;
use cleanclaw_store::Store;
use std::sync::Arc;

pub const DEFAULT_KEEP_RECENT: usize = 20;
pub const DEFAULT_TRIGGER_TOKENS: usize = 32_000; // rough char-budget proxy

/// Heuristic token estimate (chars / 4 — close enough for "should we
/// compact?").
pub fn estimate_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| {
            m.content.len()
                + m.content_parts
                    .iter()
                    .map(|p| match p {
                        cleanclaw_provider::ContentPart::Text { text } => text.len(),
                        cleanclaw_provider::ContentPart::ImageUrl { .. } => 1024,
                        cleanclaw_provider::ContentPart::ImageBase64 { data, .. } => data.len() / 4,
                    })
                    .sum::<usize>()
        })
        .sum::<usize>()
        / 4
}

/// Should this session compact? Returns `Some(keep_recent)` if yes.
pub fn should_compact(messages: &[Message], threshold: usize) -> Option<usize> {
    if estimate_tokens(messages) >= threshold {
        Some(DEFAULT_KEEP_RECENT)
    } else {
        None
    }
}

/// In-place compact: replace the older half of the working set
/// with a single "summary" message. Returns the new message list.
pub fn compact_in_place(messages: Vec<Message>, keep_recent: usize) -> Vec<Message> {
    if messages.len() <= keep_recent {
        return messages;
    }
    let split = messages.len() - keep_recent;
    let older = &messages[..split];
    let recent = &messages[split..];

    let summary_text = build_summary(older);
    let mut out: Vec<Message> = Vec::with_capacity(recent.len() + 1);
    out.push(Message::system(format!(
        "[Compaction summary — earlier conversation was {older_len} turns ago. This is a brief recap:]\n\n{summary_text}",
        older_len = older.len()
    )));
    out.extend(recent.iter().cloned());
    out
}

fn build_summary(older: &[Message]) -> String {
    // Heuristic: concatenate first 200 chars of each user turn.
    let mut out = String::new();
    let mut count = 0;
    for m in older {
        if m.role == cleanclaw_provider::Role::User {
            let snippet: String = m.content.chars().take(200).collect();
            if !snippet.is_empty() {
                out.push_str(&format!("• {snippet}\n"));
                count += 1;
                if count >= 10 {
                    break;
                }
            }
        }
    }
    if out.is_empty() {
        out.push_str("(no user turns in the compacted range)\n");
    }
    out
}

/// Persist the compacted messages back to the `sessions.messages`
/// JSONB column. Does not touch `session_messages` (the archive is
/// append-only).
pub async fn save_compacted(
    store: &Arc<dyn Store>,
    user_id: &str,
    agent_id: &str,
    session_key: &str,
    messages: &[Message],
) -> Result<()> {
    let rec = SessionRecord {
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
        session_key: session_key.to_string(),
        channel: String::new(),
        account_id: String::new(),
        chat_id: String::new(),
        project_id: String::new(),
        title: String::new(),
        messages: serde_json::to_value(messages)?,
        message_count: messages.len() as i32,
        updated_at: Utc::now(),
        chatter_user_id: String::new(),
    };
    store
        .save_session(user_id, agent_id, session_key, &rec)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_grows_with_messages() {
        let m1 = Message::user("hello");
        let m2 = Message::assistant("hi back");
        assert!(estimate_tokens(&[m1, m2]) > 0);
    }

    #[test]
    fn no_compact_when_short() {
        let msgs = vec![Message::user("hi")];
        assert!(should_compact(&msgs, 1024).is_none());
    }

    #[test]
    fn compact_replaces_older_with_summary() {
        let mut msgs: Vec<Message> = (0..30)
            .map(|i| Message::user(format!("turn {i}")))
            .collect();
        let compacted = compact_in_place(msgs.clone(), 5);
        assert_eq!(compacted.len(), 6); // 1 summary + 5 recent
        msgs = compacted;
        // The first message is a system "summary" message.
        assert!(matches!(msgs[0].role, cleanclaw_provider::Role::System));
        assert!(msgs[0].content.contains("Compaction summary"));
    }

    #[test]
    fn summary_picks_user_turns() {
        let mut msgs = vec![];
        for i in 0..15 {
            msgs.push(Message::user(format!("question {i}")));
            msgs.push(Message::assistant(format!("answer {i}")));
        }
        let compacted = compact_in_place(msgs, 4);
        let summary = &compacted[0].content;
        // Should contain at most 10 user-turn bullets, not 15.
        assert!(summary.matches('•').count() <= 10);
    }
}
