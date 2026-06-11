//! Byte-level HTML extraction helpers shared by the
//! `credential_free` scrape providers (DuckDuckGo, Baidu). They
//! avoid pulling in a full HTML parser crate.

/// Find the first occurrence of `needle` in `haystack` starting
/// from `start`. Returns the byte offset relative to `start`, or
/// `None` if not found.
pub(crate) fn find_from(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode the small subset of HTML entities that turn up in
/// search-result titles (`&amp;` / `&lt;` / `&gt;` / `&quot;` /
/// `&#39;` / `&nbsp;` / numeric entities). Avoids pulling in a
/// full HTML-decoder crate.
pub(crate) fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(end) = bytes[i..].iter().position(|&b| b == b';') {
                let end_abs = i + end;
                if end_abs - i <= 8 {
                    let entity = &s[i..=end_abs];
                    let decoded: Option<String> = match entity {
                        "&amp;" => Some("&".to_string()),
                        "&lt;" => Some("<".to_string()),
                        "&gt;" => Some(">".to_string()),
                        "&quot;" => Some("\"".to_string()),
                        "&#39;" => Some("'".to_string()),
                        "&apos;" => Some("'".to_string()),
                        "&nbsp;" => Some(" ".to_string()),
                        _ if entity.starts_with("&#x") => {
                            u32::from_str_radix(&entity[3..entity.len() - 1], 16)
                                .ok()
                                .and_then(char::from_u32)
                                .map(|c| c.to_string())
                        }
                        _ if entity.starts_with("&#") => entity[2..entity.len() - 1]
                            .parse::<u32>()
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string()),
                        _ => None,
                    };
                    if let Some(d) = decoded {
                        out.push_str(&d);
                        i = end_abs + 1;
                        continue;
                    }
                }
            }
        }
        // SAFETY: `i` always lands on a UTF-8 char boundary
        // because we only advance past `;` (which is ASCII).
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Strip `<…>` tags from a string — used to remove inner `<em>`
/// / `<span>` markup from scraped search-result titles. Returns
/// an owned `String` to avoid lifetime gymnastics; the inputs
/// are small (one search-result title) so the alloc is cheap.
pub(crate) fn strip_tags(s: &str) -> String {
    if let Some(start) = s.find('<') {
        if let Some(end) = s[start..].find('>') {
            let mut out = String::with_capacity(s.len());
            out.push_str(&s[..start]);
            out.push_str(&strip_tags(&s[start + end + 1..]));
            return out;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- find_from ----

    #[test]
    fn find_from_found() {
        let h = b"hello world";
        let n = b"world";
        assert_eq!(find_from(h, n), Some(6));
    }

    #[test]
    fn find_from_not_found() {
        let h = b"hello";
        let n = b"xyz";
        assert_eq!(find_from(h, n), None);
    }

    #[test]
    fn find_from_needle_longer_than_haystack() {
        assert_eq!(find_from(b"ab", b"abc"), None);
    }

    #[test]
    fn find_from_empty_needle() {
        assert_eq!(find_from(b"abc", b""), None);
    }

    // ---- decode_html_entities ----

    #[test]
    fn decode_html_entities_basic() {
        assert_eq!(decode_html_entities("&amp;"), "&");
        assert_eq!(decode_html_entities("&lt;"), "<");
        assert_eq!(decode_html_entities("&gt;"), ">");
        assert_eq!(decode_html_entities("&quot;"), "\"");
        assert_eq!(decode_html_entities("&#39;"), "'");
        assert_eq!(decode_html_entities("&nbsp;"), " ");
    }

    #[test]
    fn decode_html_entities_numeric() {
        assert_eq!(decode_html_entities("&#38;"), "&");
        assert_eq!(decode_html_entities("&#x26;"), "&");
    }

    #[test]
    fn decode_html_entities_no_change() {
        assert_eq!(decode_html_entities("hello world"), "hello world");
        assert_eq!(decode_html_entities(""), "");
    }

    #[test]
    fn decode_html_entities_mixed() {
        assert_eq!(
            decode_html_entities("foo &amp; bar &lt; baz"),
            "foo & bar < baz"
        );
    }

    // ---- strip_tags ----

    #[test]
    fn strip_tags_no_tags() {
        assert_eq!(strip_tags("hello world"), "hello world");
    }

    #[test]
    fn strip_tags_simple() {
        assert_eq!(strip_tags("hello <em>world</em>"), "hello world");
    }

    #[test]
    fn strip_tags_nested() {
        assert_eq!(strip_tags("<b><i>bold</i></b>"), "bold");
    }

    #[test]
    fn strip_tags_empty() {
        assert_eq!(strip_tags(""), "");
    }
}
