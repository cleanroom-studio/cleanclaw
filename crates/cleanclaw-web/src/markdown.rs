//! Server-side markdown rendering. Mirrors the `react-markdown` +
//! `remark-breaks` + `remark-gfm` setup in
//!  (which uses
//! `Markdown` from `react-markdown` with `remarkPlugins`).
//!
//! The Rust implementation is intentionally minimal — it supports:
//!
//! * Headings `#`–`######`
//! * Paragraphs
//! * Inline `code`, **bold**, *italic*
//! * Code fences (```)
//! * Unordered (`-`) and ordered (`1.`) lists
//! * Links `[text](url)` and bare URLs (auto-linked)
//! * Block quotes `>`
//! * Hard line breaks (a blank line separates paragraphs)
//!
//! The output is HTML-escaped before markdown rules apply, so XSS
//! is impossible. URLs are sanitized (only `http:` / `https:` /
//! `mailto:` / relative URLs are kept).

use std::fmt::Write;

/// Render a markdown string to safe HTML. `inline` returns the HTML
/// inside a `<p>` (single line, no block elements) for use inside
/// table cells; `render` returns a full document fragment.
pub fn render(md: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut code_buf: Vec<String> = Vec::new();
    let mut list_mode: Option<&'static str> = None; // "ul" | "ol"
    let mut para_buf: Vec<String> = Vec::new();

    let flush_para = |buf: &mut Vec<String>, out: &mut String| {
        if !buf.is_empty() {
            let joined = buf.join(" ");
            out.push_str("<p>");
            out.push_str(&inline(&joined));
            out.push_str("</p>\n");
            buf.clear();
        }
    };
    let close_list = |mode: &mut Option<&'static str>, out: &mut String| {
        if let Some(m) = mode.take() {
            out.push_str(&format!("</{m}>\n"));
        }
    };

    for line in md.lines() {
        // Code fence handling.
        if line.trim_start().starts_with("```") {
            flush_para(&mut para_buf, &mut out);
            close_list(&mut list_mode, &mut out);
            if in_code {
                let code = html_escape(&code_buf.join("\n"));
                let _ = writeln!(out, "<pre class=\"rounded-md bg-muted p-3 text-sm overflow-x-auto\"><code>{code}</code></pre>");
                code_buf.clear();
                in_code = false;
            } else {
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_buf.push(line.to_string());
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            flush_para(&mut para_buf, &mut out);
            close_list(&mut list_mode, &mut out);
            continue;
        }
        // Headings
        if let Some(h) = heading_level(line) {
            flush_para(&mut para_buf, &mut out);
            close_list(&mut list_mode, &mut out);
            let text = line.trim_start_matches('#').trim_start();
            out.push_str(&format!(
                "<h{h} class=\"font-semibold tracking-tight mt-4 mb-2\">{}</h{h}>\n",
                inline(text),
            ));
            continue;
        }
        // Block quote
        if let Some(rest) = line.strip_prefix('>') {
            flush_para(&mut para_buf, &mut out);
            close_list(&mut list_mode, &mut out);
            out.push_str(&format!(
                "<blockquote class=\"border-l-2 border-muted pl-3 text-muted-foreground\">{}</blockquote>\n",
                inline(rest.trim()),
            ));
            continue;
        }
        // List items
        if let Some(rest) = line.strip_prefix("- ") {
            flush_para(&mut para_buf, &mut out);
            if list_mode != Some("ul") {
                close_list(&mut list_mode, &mut out);
                out.push_str("<ul class=\"list-disc pl-5 space-y-1\">\n");
                list_mode = Some("ul");
            }
            out.push_str(&format!("<li>{}</li>\n", inline(rest)));
            continue;
        }
        if let Some(rest) = ordered_item(line) {
            flush_para(&mut para_buf, &mut out);
            if list_mode != Some("ol") {
                close_list(&mut list_mode, &mut out);
                out.push_str("<ol class=\"list-decimal pl-5 space-y-1\">\n");
                list_mode = Some("ol");
            }
            out.push_str(&format!("<li>{}</li>\n", inline(rest)));
            continue;
        }
        // Default: paragraph accumulator
        para_buf.push(line.to_string());
    }
    flush_para(&mut para_buf, &mut out);
    close_list(&mut list_mode, &mut out);
    if in_code && !code_buf.is_empty() {
        // Unterminated code fence — flush as plain `<pre>`.
        let code = html_escape(&code_buf.join("\n"));
        let _ = writeln!(out, "<pre class=\"rounded-md bg-muted p-3 text-sm overflow-x-auto\"><code>{code}</code></pre>");
    }
    out
}

/// Render a single line / inline snippet. Used by `render` for
/// inline contexts and exposed for callers that want to embed
/// already-trusted markdown in other layouts.
pub fn inline(s: &str) -> String {
    // Step 1: HTML-escape. From here on out we own the string and
    // emit only our own tags.
    let mut out = html_escape(s);
    // Step 2: code spans `\`...\``. Do this before bold/italic so
    // the asterisks inside code don't get mangled.
    out = apply_code_spans(&out);
    // Step 3: links `[text](url)`.
    out = apply_links(&out);
    // Step 4: bold `**...**`.
    out = apply_bold(&out);
    // Step 5: italic `*...*` / `_..._`.
    out = apply_italic(&out);
    // Step 6: bare URL autolinking.
    out = apply_autolink(&out);
    out
}

fn heading_level(s: &str) -> Option<u32> {
    let mut n: u32 = 0;
    for c in s.chars() {
        if c == '#' {
            n += 1;
        } else {
            break;
        }
    }
    if n == 0 || n > 6 {
        return None;
    }
    if s.chars().nth(n as usize) == Some(' ') {
        Some(n)
    } else {
        None
    }
}

fn ordered_item(s: &str) -> Option<&str> {
    let mut chars = s.chars();
    let mut digits = String::new();
    while let Some(c) = chars.clone().next() {
        if c.is_ascii_digit() {
            digits.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    let rest_start = digits.len();
    if s.as_bytes().get(rest_start) == Some(&b'.') && s.as_bytes().get(rest_start + 1) == Some(&b' ') {
        Some(&s[rest_start + 2..])
    } else {
        None
    }
}

fn apply_code_spans(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Find the closing backtick.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'`' {
                j += 1;
            }
            if j < bytes.len() {
                let inner = &s[i + 1..j];
                out.push_str("<code class=\"rounded bg-muted px-1 text-sm\">");
                out.push_str(inner);
                out.push_str("</code>");
                i = j + 1;
                continue;
            }
        }
        out.push(s.as_bytes()[i] as char);
        i += 1;
    }
    out
}

fn apply_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Find matching `]` then `(...)`.
            if let Some(close_bracket) = find_unescaped(bytes, i + 1, b']') {
                if bytes.get(close_bracket + 1) == Some(&b'(') {
                    if let Some(close_paren) = find_unescaped(bytes, close_bracket + 2, b')') {
                        let text = &s[i + 1..close_bracket];
                        let url = &s[close_bracket + 2..close_paren];
                        if is_safe_url(url) {
                            out.push_str("<a class=\"text-primary underline\" href=\"");
                            out.push_str(&html_escape_attr(url));
                            out.push_str("\">");
                            out.push_str(&inline(text));
                            out.push_str("</a>");
                        } else {
                            out.push_str(&html_escape(&s[i..close_paren + 1]));
                        }
                        i = close_paren + 1;
                        continue;
                    }
                }
            }
        }
        out.push(s.as_bytes()[i] as char);
        i += 1;
    }
    out
}

fn apply_bold(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(close) = find_double_marker(bytes, i + 2, b'*') {
                if close > i + 2 {
                    out.push_str("<strong>");
                    out.push_str(&s[i + 2..close]);
                    out.push_str("</strong>");
                    i = close + 2;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    if i < bytes.len() {
        out.push(bytes[i] as char);
    }
    out
}

fn apply_italic(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'*' || c == b'_' {
            if let Some(close) = find_single_marker(bytes, i + 1, c) {
                if close > i + 1 {
                    out.push_str("<em>");
                    out.push_str(&s[i + 1..close]);
                    out.push_str("</em>");
                    i = close + 1;
                    continue;
                }
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn apply_autolink(s: &str) -> String {
    // Find https?://... and wrap with <a>.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 7 < bytes.len() && &bytes[i..i + 7] == b"http://" {
            let end = scan_url_end(bytes, i + 7);
            if end > i + 7 {
                let url = &s[i..end];
                if is_safe_url(url) {
                    out.push_str("<a class=\"text-primary underline\" href=\"");
                    out.push_str(&html_escape_attr(url));
                    out.push_str("\">");
                    out.push_str(&html_escape(url));
                    out.push_str("</a>");
                    i = end;
                    continue;
                }
            }
        } else if i + 8 < bytes.len() && &bytes[i..i + 8] == b"https://" {
            let end = scan_url_end(bytes, i + 8);
            if end > i + 8 {
                let url = &s[i..end];
                if is_safe_url(url) {
                    out.push_str("<a class=\"text-primary underline\" href=\"");
                    out.push_str(&html_escape_attr(url));
                    out.push_str("\">");
                    out.push_str(&html_escape(url));
                    out.push_str("</a>");
                    i = end;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn scan_url_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\n' || b == b'\r' || b == b'<' || b == b'>' || b == b'"' || b == b'\'' {
            break;
        }
        i += 1;
    }
    // Trim trailing punctuation that's unlikely to be part of the URL.
    while i > start && matches!(bytes[i - 1], b'.' | b',' | b')' | b']' | b'!') {
        i -= 1;
    }
    i
}

fn find_unescaped(bytes: &[u8], start: usize, target: u8) -> Option<usize> {
    for i in start..bytes.len() {
        if bytes[i] == target {
            return Some(i);
        }
    }
    None
}

fn find_double_marker(bytes: &[u8], start: usize, marker: u8) -> Option<usize> {
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == marker && bytes[i + 1] == marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_marker(bytes: &[u8], start: usize, marker: u8) -> Option<usize> {
    for i in start..bytes.len() {
        if bytes[i] == marker {
            return Some(i);
        }
    }
    None
}

fn is_safe_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }
    if url.starts_with("//") || url.starts_with("javascript:") || url.starts_with("data:") || url.starts_with("vbscript:") {
        return false;
    }
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
        return true;
    }
    if url.starts_with('/') && !url.starts_with("//") {
        return true;
    }
    false
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

fn html_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_markdown_renders_empty() {
        assert_eq!(render(""), "");
    }

    #[test]
    fn paragraph_escapes_html() {
        let s = render("hello <script>alert(1)</script>");
        assert!(s.contains("&lt;script&gt;"));
        assert!(!s.contains("<script>"));
    }

    #[test]
    fn heading_levels() {
        let s = render("# H1\n## H2\n### H3");
        assert!(s.contains("<h1"));
        assert!(s.contains("<h2"));
        assert!(s.contains("<h3"));
    }

    #[test]
    fn bold_and_italic() {
        let s = render("**bold** and *italic*");
        assert!(s.contains("<strong>bold</strong>"));
        assert!(s.contains("<em>italic</em>"));
    }

    #[test]
    fn inline_code() {
        let s = render("use `let x = 1`;");
        assert!(s.contains("<code"));
        assert!(s.contains("let x = 1"));
    }

    #[test]
    fn code_fence() {
        let s = render("```\nlet x = 1;\nlet y = 2;\n```");
        assert!(s.contains("<pre"));
        assert!(s.contains("let x = 1"));
    }

    #[test]
    fn unordered_list() {
        let s = render("- a\n- b\n- c");
        assert!(s.contains("<ul"));
        assert!(s.contains("<li>a</li>"));
        assert!(s.contains("<li>c</li>"));
    }

    #[test]
    fn ordered_list() {
        let s = render("1. one\n2. two");
        assert!(s.contains("<ol"));
        assert!(s.contains("<li>one</li>"));
    }

    #[test]
    fn blockquote() {
        let s = render("> hello");
        assert!(s.contains("<blockquote"));
    }

    #[test]
    fn link() {
        let s = render("[click](https://example.com)");
        assert!(s.contains(r#"href="https://example.com""#));
    }

    #[test]
    fn link_rejects_javascript() {
        let s = render("[bad](javascript:alert(1))");
        assert!(!s.contains("href=\"javascript:"));
    }

    #[test]
    fn autolink_bare_url() {
        let s = render("see https://example.com for details");
        assert!(s.contains(r#"href="https://example.com""#));
    }

    #[test]
    fn autolink_trims_trailing_punct() {
        let s = render("see https://example.com.");
        assert!(s.contains("https://example.com"));
        assert!(!s.contains("https://example.com."));
    }

    #[test]
    fn inline_returns_safe_html() {
        let s = inline("**x** <b>y</b>");
        assert!(s.contains("<strong>x</strong>"));
        assert!(s.contains("&lt;b&gt;"));
    }

    #[test]
    fn nested_emphasis() {
        let s = render("**bold *italic* end**");
        assert!(s.contains("<strong>"));
        assert!(s.contains("<em>"));
    }
}
