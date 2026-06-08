//! PII scrubbing.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use cleanclaw_provider::message::{ContentPart, Message};

static EMAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap());
static PHONE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:\+\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}").unwrap());
static CREDIT_CARD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b").unwrap());
static SSN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap());
static IP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap());
static API_KEY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\b(?:sk-[A-Za-z0-9_\-]{20,}|AIza[A-Za-z0-9_\-]{30,}|ghp_[A-Za-z0-9]{36,}|AKIA[A-Z0-9]{16}|xoxb-[A-Za-z0-9\-]+)\b",
    )
    .unwrap()
});
static JWT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_\-]+\.eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\b").unwrap()
});
static PRIVATE_KEY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----")
        .unwrap()
});
static PASSWORD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)("password"\s*:\s*)"[^"]*""#).unwrap());

/// Replace PII patterns with placeholders. Order matters: longer /
/// more specific patterns first so e.g. a JWT inside a private key
/// block doesn't get double-substituted.
pub fn scrub(text: &str) -> String {
    let mut s = PRIVATE_KEY_RE
        .replace_all(text, "[PRIVATE_KEY]")
        .into_owned();
    s = JWT_RE.replace_all(&s, "[TOKEN]").into_owned();
    s = API_KEY_RE.replace_all(&s, "[API_KEY]").into_owned();
    s = CREDIT_CARD_RE.replace_all(&s, "[CARD]").into_owned();
    s = SSN_RE.replace_all(&s, "[SSN]").into_owned();
    s = EMAIL_RE.replace_all(&s, "[EMAIL]").to_string();
    s = PHONE_RE.replace_all(&s, "[PHONE]").to_string();
    s = IP_RE.replace_all(&s, "[IP]").to_string();
    s = PASSWORD_RE
        .replace_all(&s, r#"$1"[REDACTED]""#)
        .into_owned();
    s
}

/// Whether the text contains any detectable PII patterns. Cheap proxy
/// for "did `scrub` change the text" — used by call sites that just
/// want a bool.
pub fn contains_pii(text: &str) -> bool {
    scrub(text) != text
}

/// Result of `scrub_messages` — redacted content parts preserved for
/// the caller. Mostly exposed for testing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScrubStats {
    pub messages: usize,
    pub redactions: usize,
}

/// Redact PII from a slice of provider messages. Returns a new Vec
/// (caller-friendly; original is untouched). Only `text` content parts
/// are redacted — image URLs are left alone (they're opaque tokens
/// the LLM needs).
pub fn scrub_messages(messages: &[Message]) -> (Vec<Message>, ScrubStats) {
    let mut out = Vec::with_capacity(messages.len());
    let mut redactions = 0usize;
    for m in messages {
        let mut nm = m.clone();
        let before_content = nm.content.clone();
        nm.content = scrub(&nm.content);
        if nm.content != before_content {
            redactions += 1;
        }
        for part in nm.content_parts.iter_mut() {
            if let ContentPart::Text { text } = part {
                let before = text.clone();
                *text = scrub(text);
                if *text != before {
                    redactions += 1;
                }
            }
        }
        out.push(nm);
    }
    (
        out,
        ScrubStats {
            messages: messages.len(),
            redactions,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_email() {
        assert_eq!(scrub("ping alice@example.com ok"), "ping [EMAIL] ok");
    }

    #[test]
    fn scrubs_phone() {
        assert_eq!(scrub("call 415-555-1212 today"), "call [PHONE] today");
    }

    #[test]
    fn scrubs_credit_card() {
        assert_eq!(scrub("card 4111 1111 1111 1111 done"), "card [CARD] done");
    }

    #[test]
    fn scrubs_ssn() {
        assert_eq!(scrub("ssn 123-45-6789 ok"), "ssn [SSN] ok");
    }

    #[test]
    fn scrubs_ip() {
        assert_eq!(scrub("from 10.0.0.1 ok"), "from [IP] ok");
    }

    #[test]
    fn scrubs_jwt_token() {
        let j = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1MSJ9.abcDefGhijKLMNopQRstuVWXyz012";
        assert_eq!(scrub(j), "[TOKEN]");
    }

    #[test]
    fn scrubs_api_key() {
        assert!(scrub("sk-abcdefghijklmnopqrstuv").contains("[API_KEY]"));
        assert!(scrub("AIzaSyA-aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789").contains("[API_KEY]"));
        assert!(scrub("ghp_abcdefghijklmnopqrstuvwxyz0123456789AB").contains("[API_KEY]"));
        assert!(scrub("AKIAIOSFODNN7EXAMPLE").contains("[API_KEY]"));
    }

    #[test]
    fn scrubs_private_key_block() {
        let pem =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
        let out = scrub(pem);
        assert!(out.contains("[PRIVATE_KEY]"));
        assert!(!out.contains("BEGIN"));
    }

    #[test]
    fn scrubs_password_field() {
        let s = r#"{"user":"a","password":"hunter2"}"#;
        let out = scrub(s);
        assert!(out.contains(r#""[REDACTED]""#));
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn scrub_idempotent() {
        // Scrubbing an already-scrubbed string is a no-op (no redactions).
        let once = scrub("email me at x@y.com");
        let twice = scrub(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn contains_pii_bool() {
        assert!(contains_pii("alice@example.com"));
        assert!(!contains_pii("no pii here"));
    }

    #[test]
    fn scrub_messages_redacts_text_parts() {
        let msgs = vec![
            Message::user("hi alice@x.com"),
            Message::assistant("ok"),
            Message::user(r#"{"password":"x"}"#),
        ];
        let (out, stats) = scrub_messages(&msgs);
        assert_eq!(out.len(), 3);
        assert!(out[0].content.contains("[EMAIL]"));
        assert_eq!(out[1].content, "ok");
        assert!(out[2].content.contains("[REDACTED]"));
        assert!(stats.redactions >= 2);
    }

    #[test]
    fn scrub_messages_leaves_image_parts_alone() {
        let mut m = Message::user("see image");
        m.content_parts = vec![ContentPart::ImageUrl {
            url: "https://x/y.png".into(),
        }];
        let (out, _) = scrub_messages(&[m]);
        match &out[0].content_parts[0] {
            ContentPart::ImageUrl { url } => assert_eq!(url, "https://x/y.png"),
            _ => panic!("expected ImageUrl"),
        }
    }
}
