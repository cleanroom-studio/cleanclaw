//! Anthropic XML tool-call fallback. Mirrors
//! .
//!
//! Some non-Anthropic models served through the Anthropic-compatible
//! endpoint (notably MiMo via xiaomimimo's anthropic-compat endpoint
//! and DeepSeek-derivatives that use the `｜｜DSML｜｜` fullwidth-pipe
//! marker style) emit Claude-style tool-call XML as plain text
//! instead of returning a structured `content_block` of type
//! `tool_use`. The model has clearly seen Claude's training format
//! `<function_calls><invoke name="X"><parameter name="P">v</parameter>
//! </invoke></function_calls>` (or a close cousin) and reproduces it
//! verbatim, but the upstream gateway never converts it back to a
//! `tool_use` block, so it leaks into the assistant's text content.
//!
//! When detected, we strip the XML from the text and synthesize
//! `ToolCall` entries the agent loop can dispatch normally. If no
//! XML pattern is found, returns the input text unchanged and a
//! nil slice.
//!
//! The tag prefix is optional and tolerant: Claude uses `antml:`,
//! and DeepSeek-style models wrap tags in fullwidth pipes like
//! `｜｜DSML｜｜` (note: U+FF5C, not ASCII `|`). The outer wrapper
//! tag is either `function_calls` (Claude) or `tool_calls`
//! (DSML/OpenAI-ish).

use crate::message::ToolCall;
use serde_json::Value;

/// Scan `text` for embedded Claude-style tool-call XML. Returns
/// the cleaned text and a list of synthesized `ToolCall`s.
pub fn extract_leaked_tool_calls(text: &str) -> (String, Vec<ToolCall>) {
    // Find every outer block. Both `function_calls` and `tool_calls`
    // are valid; the inner `invoke` tag prefix may carry `antml:` or
    // fullwidth-pipe wrappers. We accept all of them.
    let mut out_calls: Vec<ToolCall> = Vec::new();
    let mut cleaned = String::with_capacity(text.len());

    let mut cursor = 0;
    while cursor < text.len() {
        // Find the next outer block start: either `<function_calls>`
        // or `<tool_calls>`. The tag may be prefixed by the
        // `antml:` namespace or wrapped in `｜｜DSML｜｜` markers —
        // strip those first.
        let remaining = &text[cursor..];
        let (block_rel, _) = match find_block_start(remaining) {
            Some(b) => b,
            None => {
                cleaned.push_str(remaining);
                break;
            }
        };
        let block_start_abs = cursor + block_rel;
        // Copy the prefix that came before the block.
        cleaned.push_str(&text[cursor..block_start_abs]);
        // Find the matching close tag (the inner content can't
        // contain another `<function_calls>` — they're not nested).
        let after_open = block_start_abs + open_tag_len(&text[block_start_abs..]);
        let close_rel = match find_outer_close(&text[after_open..]) {
            Some(c) => c,
            None => {
                // Unclosed block — bail and keep the rest verbatim.
                cleaned.push_str(&text[block_start_abs..]);
                break;
            }
        };
        let block_end_abs = after_open + close_rel;
        let inner = &text[after_open..block_end_abs];
        // Parse each `<invoke>` inside.
        for call in parse_invokes(inner) {
            out_calls.push(call);
        }
        cursor = block_end_abs + close_tag_len(&text[block_end_abs..]);
    }

    (cleaned, out_calls)
}

/// Length of the opening wrapper tag (e.g. `<function_calls>` or
/// `<tool_calls>`), including any namespace / wrapper prefixes
/// (`antml:`, `｜｜DSML｜｜`).
fn open_tag_len(s: &str) -> usize {
    // The caller already knows the start position. We measure from
    // there: skip optional `<antml:` or `｜｜DSML｜｜`, then read
    // the wrapper tag name + `>`.
    let bytes = s.as_bytes();
    let mut i = 0;
    // Skip leading whitespace.
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    // Skip `｜｜DSML｜｜` or `antml:` prefix.
    if bytes[i..].starts_with("｜｜DSML｜｜".as_bytes()) {
        i += "｜｜DSML｜｜".len();
    }
    if bytes[i..].starts_with(b"antml:") {
        i += "antml:".len();
    }
    // Skip the wrapper name: `function_calls` or `tool_calls`.
    while i < bytes.len() && bytes[i] != b'>' {
        i += 1;
    }
    if i < bytes.len() {
        i += 1; // consume '>'
    }
    i
}

/// Find the absolute index of the next outer-block open tag, and
/// return the length of any wrapper prefix (so the caller can skip
/// past it).
fn find_block_start(s: &str) -> Option<(usize, ())> {
    // Look for `<function_calls` or `<tool_calls`. The tag may be
    // prefixed by `<antml:` or wrapped in `｜｜DSML｜｜` markers.
    let positions = [
        s.find("<function_calls"),
        s.find("<tool_calls"),
        s.find("<antml:function_calls"),
        s.find("<antml:tool_calls"),
        s.find("｜｜DSML｜｜<function_calls"),
        s.find("｜｜DSML｜｜<tool_calls"),
    ];
    positions.iter().filter_map(|&p| p).min().map(|p| (p, ()))
}

/// Find the relative index of the matching close tag (`</function_calls>`
/// or `</tool_calls>`) in `s`. Returns the offset of the `<` of the
/// close tag.
fn find_outer_close(s: &str) -> Option<usize> {
    let positions = [
        s.find("</function_calls>"),
        s.find("</tool_calls>"),
        s.find("</antml:function_calls>"),
        s.find("</antml:tool_calls>"),
    ];
    positions.iter().filter_map(|&p| p).min()
}

fn close_tag_len(s: &str) -> usize {
    if s.starts_with("</antml:") {
        "</antml:function_calls>"
            .len()
            .min("</antml:tool_calls>".len())
            .max(s.find('>').map(|i| i + 1).unwrap_or(s.len()))
    } else {
        s.find('>').map(|i| i + 1).unwrap_or(s.len())
    }
}

/// Parse every `<invoke>` (or `<antml:invoke>`) inside the inner
/// content of a `<function_calls>` / `<tool_calls>` block.
fn parse_invokes(inner: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find the next `<invoke` or `<antml:invoke`.
        let rest = &inner[i..];
        let next = match [
            rest.find("<invoke "),
            rest.find("<invoke\t"),
            rest.find("<invoke>"),
            rest.find("<antml:invoke "),
        ]
        .iter()
        .filter_map(|&p| p)
        .min()
        {
            Some(p) => p,
            None => break,
        };
        let invoke_abs = i + next;
        // The open tag's attribute list (e.g. `<invoke name="foo">`)
        // lives between the `<invoke` and the first `>`. We pass it
        // through to the per-invoke parser so the function name can
        // be extracted.
        let tag_text_end = match inner[invoke_abs..].find('>') {
            Some(p) => invoke_abs + p,
            None => break,
        };
        let tag_text = &inner[invoke_abs..=tag_text_end];
        let after_invoke_open = tag_text_end + 1;
        // Find the matching close tag for this invoke.
        let close_rel = match find_invoke_close(&inner[after_invoke_open..]) {
            Some(c) => c,
            None => break,
        };
        let invoke_end_abs = after_invoke_open + close_rel;
        let body = &inner[after_invoke_open..invoke_end_abs];
        if let Some(call) = parse_one_invoke(body, tag_text) {
            calls.push(call);
        }
        // Skip past `</invoke>`. Use a length that matches what
        // find_invoke_close returned.
        let close_tag =
            if body.is_empty() || !inner[after_invoke_open..].contains("</antml:invoke>") {
                "</invoke>".len()
            } else {
                "</antml:invoke>".len()
            };
        i = invoke_end_abs + close_tag;
    }
    calls
}

fn find_invoke_close(s: &str) -> Option<usize> {
    s.find("</invoke>").or_else(|| s.find("</antml:invoke>"))
}

/// Parse one invoke body. The `tag_text` is the full open tag
/// (e.g. `<invoke name="get_weather">` or `<antml:invoke name="x">`)
/// — we use it to extract the function name attribute. The body
/// contains zero or more `<parameter name="X">value</parameter>`
/// children.
fn parse_one_invoke(body: &str, tag_text: &str) -> Option<ToolCall> {
    // The function name comes from the `name` attribute on the
    // `<invoke>` tag itself.
    let name = read_attr(tag_text, 0, "name")?;
    let mut args_map: serde_json::Map<String, Value> = serde_json::Map::new();

    // Each `<parameter name="X">value</parameter>` (or
    // `<antml:parameter name="X">value</antml:parameter>`) child.
    let mut cursor = 0;
    let bytes = body.as_bytes();
    while cursor < bytes.len() {
        let rest = &body[cursor..];
        // Match either `<parameter` (no namespace) or
        // `<antml:parameter` (Claude's namespaced variant).
        let (tag_open, tag_open_len) = match [
            ("<parameter", "<parameter".len()),
            ("<antml:parameter", "<antml:parameter".len()),
        ]
        .iter()
        .filter_map(|(needle, len)| rest.find(needle).map(|p| (p, *len)))
        .min_by_key(|&(p, _)| p)
        {
            Some(t) => t,
            None => break,
        };
        let after_open = cursor + tag_open + tag_open_len;
        // Skip whitespace and read attribute(s).
        let after_ws = skip_ws(body, after_open);
        let pname = match read_attr(body, after_ws, "name") {
            Some(n) => n,
            None => break,
        };
        // Skip to the closing `>` of the open tag.
        let after_open_tag = body[after_ws..]
            .find('>')
            .map(|i| after_ws + i + 1)
            .unwrap_or(after_ws);
        // Find `</parameter>` or `</antml:parameter>`.
        let is_antml = body[after_open_tag..].contains("</antml:parameter>");
        let close_rel = body[after_open_tag..]
            .find("</parameter>")
            .or_else(|| body[after_open_tag..].find("</antml:parameter>"))
            .unwrap_or(0);
        let value_text = &body[after_open_tag..after_open_tag + close_rel];
        // Strip CDATA wrappers if present.
        let cleaned_value = value_text
            .trim()
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>")
            .trim();
        // Try to parse as JSON; fall back to raw string.
        let value = serde_json::from_str(cleaned_value)
            .unwrap_or_else(|_| Value::String(cleaned_value.to_string()));
        args_map.insert(pname, value);
        let close_len = if is_antml {
            "</antml:parameter>".len()
        } else {
            "</parameter>".len()
        };
        cursor = after_open_tag + close_rel + close_len;
    }

    Some(ToolCall {
        id: format!("xml_{}", uuid::Uuid::new_v4().simple()),
        name,
        arguments: Value::Object(args_map),
    })
}

fn skip_ws(s: &str, mut i: usize) -> usize {
    let bytes = s.as_bytes();
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    i
}

fn read_attr(s: &str, _i: usize, attr: &str) -> Option<String> {
    // Scan the entire string for `attr="..."` rather than assuming
    // it starts at position `i`. The caller passes `i` as a hint
    // for where attributes begin (after the tag name), but the
    // simple `find_attr` approach is more robust when the tag
    // string is the full open tag.
    let needle = format!("{attr}=\"");
    let start = s.find(&needle)?;
    let after = &s[start + needle.len()..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// Public test helper: extract calls from a text blob and return
/// the synthesized names. Used by integration tests that don't
/// want to depend on a `ToolCall` builder.
pub fn extract_call_names(text: &str) -> Vec<String> {
    extract_leaked_tool_calls(text)
        .1
        .into_iter()
        .map(|c| c.name)
        .collect()
}

/// Convenience: extract leaked tool calls and append them to an
/// existing `Vec<ToolCall>` (typically the runtime's accumulator),
/// returning the cleaned text. This is the one-stop helper the
/// `cleanclaw-agent::loop_runner` calls after each streamed
/// `ContentDelta` to make sure leaked XML gets dispatched rather
/// than rendered to the user.
pub fn strip_into(text: &str, sink: &mut Vec<ToolCall>) -> String {
    let (cleaned, mut calls) = extract_leaked_tool_calls(text);
    sink.append(&mut calls);
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_xml_returns_input_unchanged() {
        let (text, calls) = extract_leaked_tool_calls("plain text, no XML");
        assert_eq!(text, "plain text, no XML");
        assert!(calls.is_empty());
    }

    #[test]
    fn function_calls_block_parses() {
        let input = r#"hi
<function_calls>
<invoke name="get_weather">
<parameter name="city">Tokyo</parameter>
</invoke>
</function_calls>
bye"#;
        let (cleaned, calls) = extract_leaked_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "Tokyo");
        assert!(cleaned.contains("hi"));
        assert!(cleaned.contains("bye"));
        assert!(!cleaned.contains("<function_calls>"));
    }

    #[test]
    fn tool_calls_block_parses() {
        // OpenAI-ish wrapper.
        let input = "<tool_calls><invoke name=\"f\"><parameter name=\"x\">1</parameter></invoke></tool_calls>";
        let (cleaned, calls) = extract_leaked_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "f");
        assert!(cleaned.is_empty());
    }

    #[test]
    fn antml_namespaced_block_parses() {
        let input = "<antml:function_calls><antml:invoke name=\"g\"><antml:parameter name=\"k\">v</antml:parameter></antml:invoke></antml:function_calls>";
        let (_, calls) = extract_leaked_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "g");
        assert_eq!(calls[0].arguments["k"], "v");
    }

    #[test]
    fn dsml_fullwidth_pipe_wrapper_parses() {
        // DeepSeek-style wrapping: fullwidth pipes around the
        // open + close tag.
        let input = "｜｜DSML｜｜<function_calls><invoke name=\"h\"><parameter name=\"q\">42</parameter></invoke></function_calls>";
        let (cleaned, calls) = extract_leaked_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "h");
        assert!(cleaned.is_empty() || !cleaned.contains("function_calls"));
    }

    #[test]
    fn cdata_wrapped_value_is_stripped() {
        let input = r#"<function_calls><invoke name="x"><parameter name="data"><![CDATA[{"a":1}]]></parameter></invoke></function_calls>"#;
        let (_, calls) = extract_leaked_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["data"], json!({"a": 1}));
    }

    #[test]
    fn multiple_invokes_in_one_block() {
        let input = r#"<function_calls>
<invoke name="a"><parameter name="x">1</parameter></invoke>
<invoke name="b"><parameter name="y">2</parameter></invoke>
</function_calls>"#;
        let (_, calls) = extract_leaked_tool_calls(input);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "a");
        assert_eq!(calls[1].name, "b");
    }

    #[test]
    fn extract_call_names_helper() {
        let names =
            extract_call_names("<function_calls><invoke name=\"foo\"></invoke></function_calls>");
        assert_eq!(names, vec!["foo".to_string()]);
    }
}
