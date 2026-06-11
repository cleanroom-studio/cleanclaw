//! Tool-call turn-failure tracking and recovery.
//!
//! Tracks (tool_name, args_hash) → previous-error-summary for
//! tool calls that already failed earlier in the current turn.
//! Tool implementations consult `prior_failure` to short-circuit
//! a guaranteed-fail retry. The hash keying matches the agent
//! loop's loop-detection hash so both layers agree on what
//! "the same call" means.
//!
//! Also: XML tool-call recovery. When a non-Anthropic model
//! (DeepSeek, MiMo, …) emits tool calls as raw XML in the
//! assistant `content` rather than the structured `tool_calls`
//! array, the loop's downstream dispatch would miss them. The
//! `recover_tool_calls_from_text` helper scans the message text
//! for the same XML patterns the Go `tool_recovery.go` recognises
//! and synthesizes `ToolCall` records that the loop can dispatch
//! like any other call.
//!
//! # Two halves, one file
//!
//! The module is deliberately two-faced:
//!
//! * `TurnFailures` (top half) is a *runtime* concern — it
//!  protects the user from a model that retries the same
//!  broken call twice in a row.
//! * `recover_tool_calls_from_text` (bottom half) is a *wire*
//!  concern — it bridges a class of providers (DeepSeek, MiMo,
//!  Qwen tool-mode, …) that emit calls in a different shape
//!  than Anthropic's structured `tool_calls`.
//!
//! They share no code, but they share a *philosophy*: the agent
//! loop should not silently drop work because the wire format
//! was non-standard or the model was stubborn.
//!
//! # Threading
//!
//! `TurnFailures` uses a `std::sync::Mutex` rather than
//! `tokio::sync::Mutex` because the critical section is two
//! `HashMap` operations. We never hold the lock across an
//! `.await` point.
//!
//! The regexes are wrapped in `OnceLock<Regex>` so they compile
//! once per process. Compilation is expensive (each `Regex` is
//! a small DFA) and the patterns are static; this is the
//! idiomatic `lazy_static`-replacement in stable Rust.
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
/// Composite key for the failure tracker.
///
/// `tool` is the tool name as the model emitted it (case-
/// sensitive, no normalisation). `args_hash` is the SHA-256 of
/// the canonical JSON serialisation of the arguments — the
/// same hash `cleanclaw_provider` uses for its own loop-
/// detection, so the two layers agree on what "the same call"
/// means. Hashing the arguments (rather than comparing them
/// structurally) keeps the map O(1) and avoids the cost of a
/// deep `Value` equality check on every retry.
///
/// `Clone` + `Eq` + `Hash` + `PartialEq` are all derived so the
/// type can be used directly as a `HashMap` key without a
/// custom wrapper.
pub struct TurnFailKey {
    /// The tool name as emitted by the model. Stored verbatim
    /// (no lowercasing / no whitespace trimming) so a
    /// misspelled retry produces a different key and is *not*
    /// short-circuited.
    pub tool: String,
    /// SHA-256 of the canonical JSON form of the arguments.
    /// 32 bytes fixed-width, so the key is the same size
    /// regardless of how many arguments the tool takes.
    pub args_hash: [u8; 32],
}

/// Per-turn failure tracker. One instance is owned by each
/// `Agent` and is reset at the start of every `run_turn`.
///
/// Shared across iterations of the same turn via `Arc`
/// (`Agent::turn_failures`). Iterations of one turn may
/// read or write the map concurrently; `std::sync::Mutex`
/// is sufficient because the critical section is tiny.
pub struct TurnFailures {
    inner: Mutex<HashMap<TurnFailKey, String>>,
}

impl TurnFailures {
    /// Build an empty tracker. `AgentBuilder::build` calls
    /// this once per agent; subsequent calls to `reset` reuse
    /// the same allocation.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Reset the per-turn map.
    ///
    /// Called once at the start of every `run_turn` so a
    /// fresh turn doesn't inherit prior turns' failure
    /// record. We `clear()` rather than drop-and-replace to
    /// keep the `HashMap`'s allocated buckets — agents have
    /// many turns and most failures repeat across turns, so
    /// keeping the capacity saves a re-allocation each time.
    /// Reset the per-turn map. Call once at the start of every
    /// `run_turn`.
    pub fn reset(&self) {
        self.inner.lock().unwrap().clear();
    }

    /// Record that the given (tool, args) call failed.
    ///
    /// Stores a *truncated* (≤200 char) summary of the error
    /// in the map. Truncation matters because a runaway tool
    /// could otherwise dump megabytes of stack trace into the
    /// value, bloating the map and slowing subsequent lookups.
    ///
    /// Returns nothing — the value is only useful to the
    /// next caller via `prior_failure`. We don't return
    /// `Result` because the operation can't fail.
    pub fn record(&self, tool: &str, args: &serde_json::Value, error: &str) {
        let summary = truncate(error, 200);
        let key = TurnFailKey {
            tool: tool.to_string(),
            args_hash: hash_args(args),
        };
        self.inner.lock().unwrap().insert(key, summary);
    }

    /// Look up the prior error summary for an exact
    /// (tool, args) pair, or `None` if this call hasn't
    /// failed earlier in the current turn.
    ///
    /// Tool implementations call this on entry:
    /// ```ignore
    /// if let Some(prev) = ctx.turn_failures.prior_failure(self.name(), &args) {
    ///     return Err(format!("already failed in this turn: {prev}"));
    /// }
    /// ```
    /// This is a *cooperative* short-circuit — a tool that
    /// doesn't consult the tracker still gets called. We
    /// avoid a hard policy so that tools with idempotent
    /// retry semantics (e.g. a "read a fixed URL") can
    /// still serve the call even if it once failed.
    pub fn prior_failure(&self, tool: &str, args: &serde_json::Value) -> Option<String> {
        let key = TurnFailKey {
            tool: tool.to_string(),
            args_hash: hash_args(args),
        };
        self.inner.lock().unwrap().get(&key).cloned()
    }
}

impl Default for TurnFailures {
    /// Default — used when the tracker is held inside other
    /// `derive(Default)` structs.
    fn default() -> Self {
        Self::new()
    }
}

/// UTF-8-safe truncation.
///
/// Walks the byte slice back to the nearest `char_boundary`
/// if the cut point falls mid-codepoint, and appends `…`
/// (U+2026) so readers can see the string was clipped.
///
/// `max` is in *bytes* — the same unit the std `str::len`
/// uses — so the budget is constant across scripts. A
/// 200-byte budget holds ~50 CJK characters or ~200 ASCII
/// characters, which is plenty for an error summary.
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

/// Hash a `serde_json::Value` into a 32-byte key.
///
/// We serialise the value to a canonical JSON string and
/// SHA-256 the bytes. The serialisation is `serde_json`'s
/// default (compact, no whitespace); for the purposes of
/// "is this the same call?" two semantically-equal but
/// textually-different serialisations would be treated as
/// different. In practice every call site that reaches
/// this function has come through the provider's tool-call
/// serialiser, so the form is stable.
///
/// We swallow the `serde_json::Error` by mapping it to an
/// empty string. The only way `to_string` fails is a
/// `MapKey` borrow issue, which Rust prevents at compile
/// time, so this is effectively unreachable — but we keep
/// the `unwrap_or_default` to satisfy the type system and
/// because a `Value` that fails to serialise is the same
/// as no call for hashing purposes.
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
/// One recovered tool call.
///
/// Wraps the provider `ToolCall` with the source text the
/// call was extracted from. The source text is kept for
/// debugging — when a recovery "works" (i.e. produces a
/// call) we want the operator to be able to inspect what
/// the model actually emitted. The agent loop logs the
/// length and the recovered tool name; the full text is
/// not persisted.
pub struct RecoveredCall {
    /// The synthesised tool call. Goes into the loop's normal
    /// dispatch path; the loop never needs to know the call
    /// came from XML recovery rather than a structured
    /// `tool_calls` array.
    pub call: ToolCall,
    /// Source text the call was extracted from. Used for
    /// debugging only — the loop does not act on it.
    pub source_text: String,
}

/// Regex matching an entire `<function_calls>...</function_calls>`
/// block.
///
/// `(?s)` enables dotall so `.` matches newlines. We pull the
/// whole block out first, then run the inner `invoke_re` on
/// the substring — this keeps the per-block regexes simple
/// and avoids catastrophic backtracking if the outer pattern
/// is satisfied by text that has no `<invoke>` children.
///
/// Wrapped in `OnceLock<Regex>` so the compilation cost
/// (`Regex::new` builds a small DFA) is paid once per process.
fn function_calls_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // Anthropic-style: <function_calls>...<invoke name="x">...</invoke>...</function_calls>
        Regex::new(r"(?s)<function_calls>.*?</function_calls>").expect("static regex")
    })
}

/// Regex matching a single `<invoke name="...">...</invoke>`
/// block.
///
/// Captures two named groups: `n` (the tool name, double-
/// quoted) and `a` (the body, which may contain either
/// `<parameter>` children or a single short-form child).
/// The `?` quantifier on `a` is lazy so we don't run past
/// the `</invoke>` close tag.
fn invoke_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"(?s)<invoke\s+name="(?P<n>[^"]+)"\s*>(?P<a>.*?)</invoke>"#)
            .expect("static regex")
    })
}

/// Regex matching a single `<parameter name="k">v</parameter>`
/// child of an `<invoke>` block (Anthropic's long form).
fn parameter_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"(?s)<parameter\s+name="(?P<k>[^"]+)"\s*>(?P<v>.*?)</parameter>"#)
            .expect("static regex")
    })
}

/// Regex matching a single short-form `<k>v</k>` child.
///
/// The name and value are captured; the closing tag name is
/// *not* matched against the opening tag name because Rust's
/// `regex` crate doesn't support back-references. In practice
/// the closing tag almost always matches the opener (it's the
/// same word), and the first matching close is what we want.
/// If a model writes mismatched tags we silently recover
/// whatever the first match produces.
///
/// This regex is also lenient about the tag-name character
/// class (`[A-Za-z_][A-Za-z0-9_-]*`): tool / parameter names
/// in JSON-schema land are usually ASCII identifiers, and we
/// don't want to over-engineer for Unicode tool names here.
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

/// Strip the optional `｜｜DSML｜｜` fence tokens some
/// providers wrap their XML in.
///
/// `trim_start_matches` / `trim_end_matches` are used because
/// the fence may appear zero or one times (it never wraps
/// mid-content), and we want to be lenient if the provider
/// omits either side. The final `trim()` cleans leading /
/// trailing whitespace introduced by the strip.
fn dsml_strip(s: &str) -> &str {
    // Some providers wrap the XML block in DSML fence tokens.
    s.trim_start_matches("｜｜DSML｜｜function_calls｜｜")
        .trim_end_matches("｜｜/DSML｜｜")
        .trim()
}

/// Extract leaked tool calls from the assistant content.
///
/// Returns the synthesised `ToolCall`s along with the (raw)
/// source text each call was extracted from (used for
/// debug logging).
///
/// Short-circuits to `Vec::new()` if the input already
/// carries structured `tool_calls` — we never want to
/// double-dispatch a call the model emitted twice (once in
/// `tool_calls` and once in `content`), which is a real
/// failure mode for some "hybrid" providers.
///
/// Otherwise the function is pure: no I/O, no state, no
/// allocations beyond the output `Vec`. Safe to call from
/// any context, including inside an `async` lock-free path.
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

/// Parse the inner XML of an `<invoke>` block into a JSON
/// object.
///
/// Recognises two syntaxes:
/// * **Long form**: `<parameter name="k">v</parameter>`
///   (multiple keys per block, Anthropic style).
/// * **Short form**: `<k>v</k>` (single key, MiMo / Qwen style).
///
/// Falls back to `{}` (empty object) when the body is
/// empty or unparseable. We do *not* error: a malformed
/// XML body just means "no parameters", and the tool's
/// own validation will catch any missing-required-field
/// error.
///
/// All values are emitted as `Value::String` regardless of
/// the XML body content. JSON type coercion (e.g. turning
/// `"true"` into a JSON boolean) is *not* done here — the
/// tool's `parameters` schema validation is the right place
/// to do that, and doing it twice would cause subtle
/// disagreements (e.g. `"1"` vs `1`).
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
    //! Unit tests for both halves of this module: the
    //! turn-failure tracker and the XML recovery parser.
    //!
    //! Tests are split into two natural groups:
    //! 1. XML recovery (`recover_tool_calls_from_text` and
    //!    `parse_invocation_args`).
    //! 2. Failure tracker (`TurnFailures::record` /
    //!    `prior_failure` / `reset`, plus the helper
    //!    functions `hash_args` and `truncate`).
    use super::*;

    /// Helper to build a minimal `ToolCall` for tests that
    /// pass `existing_calls` into `recover_tool_calls_from_text`.
    fn tc(name: &str) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }
    }

    #[test]
    /// Empty input → no calls.
    fn empty_content_returns_empty() {
        let out = recover_tool_calls_from_text("", &[]);
        assert!(out.is_empty());
    }

    #[test]
    /// When the model already gave us structured calls, the
    /// recovery function must not double up. This is the
    /// most important safety pin: a regression here would
    /// dispatch the same call twice on hybrid providers.
    fn existing_structured_calls_block_recovery() {
        let raw = "<function_calls><invoke name=\"foo\"><x>1</x></invoke></function_calls>";
        let out = recover_tool_calls_from_text(raw, &[tc("foo")]);
        assert!(
            out.is_empty(),
            "should not double-recover when structured calls present"
        );
    }

    #[test]
    /// Anthropic long-form recovery:
    /// `<function_calls><invoke name="..."><parameter name="...">...</parameter></invoke></function_calls>`.
    /// Verifies the synthesised `id` carries the tool name
    /// and a deterministic prefix (so log filtering is easy).
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
    /// Short-form recovery: `<invoke name="echo"><text>hi</text></invoke>`.
    fn recovers_short_form() {
        let raw =
            r#"<function_calls><invoke name="echo"><text>hi</text></invoke></function_calls>"#;
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "echo");
        assert_eq!(out[0].call.arguments, serde_json::json!({ "text": "hi" }));
    }

    #[test]
    /// Multiple `<invoke>` blocks in one `<function_calls>`
    /// parent — must produce one call per block, in order.
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
    /// Strip the DSML fence that some providers wrap the
    /// XML in. The full-width `｜｜` characters are
    /// deliberate (they're not the half-width `||`); we
    /// match the exact bytes.
    fn strips_dsml_fence() {
        let raw = "｜｜DSML｜｜function_calls｜｜<function_calls><invoke name=\"x\"><y>1</y></invoke></function_calls>｜｜/DSML｜｜";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "x");
    }

    #[test]
    /// No `<function_calls>` block in the input → empty
    /// result. The function must not be eager.
    fn no_function_calls_block_returns_empty() {
        let raw = "Hello, this is just plain text without any tool calls.";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert!(out.is_empty());
    }

    #[test]
    /// Malformed XML (unclosed `<invoke>`) → empty result.
    /// We don't error out; the loop's downstream XML
    /// recovery is best-effort.
    fn malformed_xml_returns_empty() {
        // Unclosed <invoke> should yield no calls.
        let raw = "<function_calls><invoke name=\"x\"></function_calls>";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert!(out.is_empty());
    }

    #[test]
    /// Empty `<invoke>` body → empty args object. Useful for
    /// tools that take no parameters.
    fn empty_invoke_body_yields_empty_args() {
        let raw = "<function_calls><invoke name=\"ping\"></invoke></function_calls>";
        let out = recover_tool_calls_from_text(raw, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].call.name, "ping");
        assert_eq!(out[0].call.arguments, serde_json::json!({}));
    }

    #[test]
    /// Long-form with multiple `<parameter>` children. All
    /// values are emitted as strings (no type coercion — see
    /// the doc on `parse_invocation_args`).
    fn parse_invocation_args_long_form_multiple_params() {
        let body = r#"<parameter name="a">1</parameter>
            <parameter name="b">hello</parameter>
            <parameter name="c">true</parameter>"#;
        let v = parse_invocation_args(body);
        assert_eq!(v, serde_json::json!({"a":"1","b":"hello","c":"true"}));
    }

    #[test]
    /// End-to-end tracker pin:
    /// * empty → None
    /// * record + query → Some(summary)
    /// * different args → None
    /// * different tool → None
    /// * reset → None again
    fn turn_failures_record_then_query() {
        let tf = TurnFailures::new();
        let args = serde_json::json!({"path": "/x"});
        assert!(tf.prior_failure("read", &args).is_none());
        tf.record("read", &args, "no such file");
        assert_eq!(tf.prior_failure("read", &args), Some("no such file".into()));
        // Different args = no hit
        assert!(tf
            .prior_failure("read", &serde_json::json!({"path": "/y"}))
            .is_none());
        // Different tool = no hit
        assert!(tf.prior_failure("write", &args).is_none());
        tf.reset();
        assert!(tf.prior_failure("read", &args).is_none());
    }

    #[test]
    /// `hash_args` must be deterministic: same input → same
    /// hash, different input → different hash.
    fn hash_args_stable() {
        let a = hash_args(&serde_json::json!({"x": 1}));
        let b = hash_args(&serde_json::json!({"x": 1}));
        let c = hash_args(&serde_json::json!({"x": 2}));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    /// `truncate` is a no-op when the input is short.
    fn truncate_preserves_short_strings() {
        assert_eq!(truncate("hi", 100), "hi");
    }

    #[test]
    /// `truncate` clips long strings to ≤max bytes plus the
    /// trailing ellipsis. The test uses a 500-char ASCII
    /// string and a 50-byte cap, so the result is at most
    /// 51 bytes (50 chars + the `…`).
    fn truncate_truncates_long_strings() {
        let s = "x".repeat(500);
        let t = truncate(&s, 50);
        assert!(t.chars().count() <= 51);
        assert!(t.ends_with('…'));
    }
}
