//! Tool-call turn-failure tracking and recovery.
//!
//! Tracks
//! (tool_name, args_hash) → previous-error-summary for tool calls that
//! already failed earlier in the current turn. Tool implementations
//! consult `prior_failure` to short-circuit a guaranteed-fail retry.
//! The hash keying matches the agent loop's loop-detection hash so
//! both layers agree on what "the same call" means.
//!
//! Also: XML tool-call recovery. When a non-Anthropic model
//! (DeepSeek, MiMo, …) emits tool calls as raw XML in the assistant
//! `content` rather than the structured `tool_calls` array, the
//! loop's downstream dispatch would miss them. The
//! `recover_tool_calls_from_text` helper scans the message text for
//! the same XML patterns the Go `tool_recovery.go` recognizes and
//! synthesizes `ToolCall` records that the loop can dispatch like
//! any other call.

use cleanclaw_provider::ToolCall;
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// =====================================================================
// Per-turn failure tracker
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnFailKey {
    pub tool: String,
    pub args_hash: [u8; 32],
}

pub struct TurnFailures {
    inner: Mutex<HashMap<TurnFailKey, String>>,
}

impl TurnFailures {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Reset the per-turn map. Call once at the start of every
    /// `run_turn`.
    pub fn reset(&self) {
        self.inner.lock().unwrap().clear();
    }

    /// Record that the given (tool, args) call failed. Returns the
    /// summary (truncated to 200 chars so a runaway tool can't
    /// bloat the map).
    pub fn record(&self, tool: &str, args: &serde_json::Value, error: &str) {
        let summary = truncate(error, 200);
        let key = TurnFailKey {
            tool: tool.to_string(),
            args_hash: hash_args(args),
        };
        self.inner.lock().unwrap().insert(key, summary);
    }

    /// Returns the prior error summary for this exact (tool, args)
    /// pair, or `None` if it hasn't failed yet in this turn.
    pub fn prior_failure(&self, tool: &str, args: &serde_json::Value) -> Option<String> {
        let key = TurnFailKey {
            tool: tool.to_string(),
            args_hash: hash_args(args),
        };
        self.inner.lock().unwrap().get(&key).cloned()
    }
}

impl Default for TurnFailures {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Truncate at a char boundary.
        let mut out = s[..max].to_string();
        while !out.is_char_boundary(out.len()) {
            out.pop();
        }
        out.push('…');
        out
    }
}

fn hash_args(args: &serde_json::Value) -> [u8; 32] {
    let mut h = Sha256::new();
    let s = serde_json::to_string(args).unwrap_or_default();
    h.update(s.as_bytes());
    h.finalize().into()
}

// =====================================================================
// XML tool-call recovery
// =====================================================================

/// One recovered tool call. Wraps the provider `ToolCall` with the
/// extracted text so the agent loop can log where the call came from.
#[derive(Debug, Clone)]
pub struct RecoveredCall {
    pub call: ToolCall,
    /// Text region the call was extracted from (used for debugging).
    pub source_text: String,
}

fn function_calls_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // Anthropic-style: <function_calls>...<invoke name="x">...</invoke>...</function_calls>
        Regex::new(r"(?s)<function_calls>.*?</function_calls>")
            .expect("static regex")
    })
}

fn invoke_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"(?s)<invoke\s+name="(?P<n>[^"]+)"\s*>(?P<a>.*?)</invoke>"#)
            .expect("static regex")
    })
}

fn parameter_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"(?s)<parameter\s+name="(?P<k>[^"]+)"\s*>(?P<v>.*?)</parameter>"#)
            .expect("static regex")
    })
}

fn short_form_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // Rust's `regex` crate doesn't support back-references, so
        // we don't try to match the closing tag name. The first
        // child element wins; deeper nesting is rare enough that
        // we don't bother with it. (Long-form `<parameter>` parsing
        // handles structured cases anyway.)
        Regex::new(r"(?s)<(?P<k>[A-Za-z_][A-Za-z0-9_-]*)>(?P<v>.*?)</[A-Za-z_][A-Za-z0-9_-]*>")
            .expect("static regex")
    })
}

fn dsml_strip(s: &str) -> &str {
    // Some providers wrap the XML block in DSML fence tokens.
    s.trim_start_matches("｜｜DSML｜｜function_calls｜｜")
        .trim_end_matches("｜｜/DSML｜｜")
        .trim()
}

/// Extract leaked tool calls from the assistant content. Returns
/// the synthesized `ToolCall`s and the (possibly trimmed) source
/// text the loop should keep around. Empty `tool_calls` field on
/// the input message signals nothing was structured — we recover
/// from raw XML in that case.
pub fn recover_tool_calls_from_text(
    content: &str,
    existing_calls: &[ToolCall],
) -> Vec<RecoveredCall> {
    if !existing_calls.is_empty() {
        // The provider already gave us structured calls; don't double up.
        return Vec::new();
    }
    let trimmed = dsml_strip(content);
    let mut out = Vec::new();
    for block in function_calls_re().find_iter(trimmed) {
        let block_text = block.as_str();
        for cap in invoke_re().captures_iter(block_text) {
            let name = match cap.name("n") {
                Some(m) => m.as_str().to_string(),
                None => continue,
            };
            let raw_args = cap.name("a").map(|m| m.as_str()).unwrap_or("");
            let arguments = parse_invocation_args(raw_args);
            // Synthesize a stable id. The agent loop dispatches by
            // name+args, not id, so a deterministic id is enough.
            let id = format!("xml_{}_{}", name, out.len());
            out.push(RecoveredCall {
                call: ToolCall {
                    id,
                    name,
                    arguments,
                },
                source_text: block_text.to_string(),
            });
        }
    }
    out
}

/// Parse the inner XML of an `<invoke>` block into a JSON object.
/// Recognises `<parameter name="x">value</parameter>` and `<x>val</x>`
/// shorthand; falls back to `{}` when the body is empty.
fn parse_invocation_args(body: &str) -> Value {
    let mut out = Map::new();
    for cap in parameter_re().captures_iter(body) {
        if let (Some(k), Some(v)) = (cap.name("k"), cap.name("v")) {
            out.insert(
                k.as_str().to_string(),
                Value::String(v.as_str().trim().to_string()),
            );
        }
    }
    if !out.is_empty() {
        return Value::Object(out);
    }
    // Short form: <k>v</k>. We pick the first child element name.
    if let Some(cap) = short_form_re().captures(body) {
        if let (Some(k), Some(v)) = (cap.name("k"), cap.name("v")) {
            let mut m = Map::new();
            m.insert(
                k.as_str().to_string(),
                Value::String(v.as_str().trim().to_string()),
            );
            return Value::Object(m);
        }
    }
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(name: &str) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }
    }

    #[test]
    fn empty_content_returns_empty() {
        let out = recover_tool_calls_from_text("", &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn existing_structured_calls_block_recovery() {
        let raw = "<function_calls><invoke name=\"foo\"><x>1</x></invoke></function_calls>";
        let out = recover_tool_calls_from_text(raw, &[tc("foo")]);
        assert!(out.is_empty(), "should not double-recover when structured calls present");
    }

    #[test]
    fn recovers_anthropic_style_long_form() {
        let raw = r#"<function_calls>
            <invoke name="read_file">
                <parameter name="path">/etc/hostname</parameter>
            </invoke>
        </function_calls>"#;
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "read_file");
        assert_eq!(
            out[0].call.arguments,
            serde_json::json!({ "path": "/etc/hostname" })
        );
        assert!(out[0].call.id.starts_with("xml_read_file_"));
    }

    #[test]
    fn recovers_short_form() {
        let raw = r#"<function_calls><invoke name="echo"><text>hi</text></invoke></function_calls>"#;
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "echo");
        assert_eq!(out[0].call.arguments, serde_json::json!({ "text": "hi" }));
    }

    #[test]
    fn recovers_multiple_invokes() {
        let raw = r#"<function_calls>
            <invoke name="a"><x>1</x></invoke>
            <invoke name="b"><y>2</y></invoke>
        </function_calls>"#;
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].call.name, "a");
        assert_eq!(out[1].call.name, "b");
    }

    #[test]
    fn strips_dsml_fence() {
        let raw = "｜｜DSML｜｜function_calls｜｜<function_calls><invoke name=\"x\"><y>1</y></invoke></function_calls>｜｜/DSML｜｜";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "x");
    }

    #[test]
    fn no_function_calls_block_returns_empty() {
        let raw = "Hello, this is just plain text without any tool calls.";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn malformed_xml_returns_empty() {
        // Unclosed <invoke> should yield no calls.
        let raw = "<function_calls><invoke name=\"x\"></function_calls>";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn empty_invoke_body_yields_empty_args() {
        let raw = "<function_calls><invoke name=\"ping\"></invoke></function_calls>";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "ping");
        assert_eq!(out[0].call.arguments, serde_json::json!({}));
    }

    #[test]
    fn parse_invocation_args_long_form_multiple_params() {
        let body = r#"<parameter name="a">1</parameter>
            <parameter name="b">hello</parameter>
            <parameter name="c">true</parameter>"#;
        let v = parse_invocation_args(body);
        assert_eq!(v, serde_json::json!({"a":"1","b":"hello","c":"true"}));
    }

    #[test]
    fn turn_failures_record_then_query() {
        let tf = TurnFailures::new();
        let args = serde_json::json!({"path": "/x"});
        assert!(tf.prior_failure("read", &args).is_none());
        tf.record("read", &args, "no such file");
        assert_eq!(tf.prior_failure("read", &args), Some("no such file".into()));
        // Different args = no hit
        assert!(tf.prior_failure("read", &serde_json::json!({"path": "/y"})).is_none());
        // Different tool = no hit
        assert!(tf.prior_failure("write", &args).is_none());
        tf.reset();
        assert!(tf.prior_failure("read", &args).is_none());
    }

    #[test]
    fn hash_args_stable() {
        let a = hash_args(&serde_json::json!({"x": 1}));
        let b = hash_args(&serde_json::json!({"x": 1}));
        let c = hash_args(&serde_json::json!({"x": 2}));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn truncate_preserves_short_strings() {
        assert_eq!(truncate("hi", 100), "hi");
    }

    #[test]
    fn truncate_truncates_long_strings() {
        let s = "x".repeat(500);
        let t = truncate(&s, 50);
        assert!(t.chars().count() <= 51);
        assert!(t.ends_with('…'));
    }
}
