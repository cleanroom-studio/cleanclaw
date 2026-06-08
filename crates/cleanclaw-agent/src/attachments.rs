//! Multimodal attachments — image / file uploads from the chat surface.
//!
//! The chat surface receives a chat row that may carry one or more photo
//! URLs; we resolve each URL to bytes, base64 it for the LLM, and
//! attach it to the inbound message as a `ContentPart::ImageBase64`
//! (OpenAI-style data URL).

use base64::Engine;
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_provider::{ContentPart, Message};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::warn;

const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024; // 8 MiB per attachment
const MAX_ATTACHMENTS_PER_MESSAGE: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub url: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// Original filename if we can recover it from the URL.
    pub filename: Option<String>,
}

/// In-memory attachment store. The chat handler populates this with
/// resolved attachments before the agent loop fires.
#[derive(Debug, Default, Clone)]
pub struct AttachmentStore {
    by_session: std::collections::HashMap<String, Vec<Attachment>>,
}

impl AttachmentStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a list of URLs for a session into `Attachment` records.
    /// Drops oversized / unsupported URLs with a warning. Returns
    /// the in-memory copies.
    pub async fn resolve(
        &self,
        session_key: &str,
        urls: &[String],
    ) -> Result<Vec<Attachment>> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for url in urls.iter().take(MAX_ATTACHMENTS_PER_MESSAGE) {
            if !seen.insert(url.clone()) {
                continue;
            }
            match fetch_one(url).await {
                Ok(a) => out.push(a),
                Err(e) => warn!(url, "attachment fetch failed: {e}"),
            }
        }
        // Note: we don't persist to by_session — call sites can call
        // attach() if they want the attachments to survive the turn.
        let _ = session_key;
        Ok(out)
    }

    pub fn attach(&mut self, session_key: &str, atts: Vec<Attachment>) {
        self.by_session.insert(session_key.to_string(), atts);
    }

    pub fn get(&self, session_key: &str) -> Option<&Vec<Attachment>> {
        self.by_session.get(session_key)
    }

    pub fn clear(&mut self, session_key: &str) {
        self.by_session.remove(session_key);
    }
}

async fn fetch_one(url: &str) -> Result<Attachment> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CleanClawError::InvalidArgument(format!(
            "attachment: only http(s) URLs are supported, got {url}"
        )));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| CleanClawError::Internal(format!("attachment client: {e}")))?;
    let resp = client.get(url).send().await.map_err(|e| {
        CleanClawError::Upstream(format!("attachment fetch {url}: {e}"))
    })?;
    if !resp.status().is_success() {
        return Err(CleanClawError::Upstream(format!(
            "attachment fetch {url}: HTTP {}",
            resp.status()
        )));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    if !is_supported_image(&content_type) {
        return Err(CleanClawError::InvalidArgument(format!(
            "attachment {url}: unsupported content-type {content_type}"
        )));
    }
    let bytes = resp.bytes().await.map_err(|e| {
        CleanClawError::Upstream(format!("attachment body: {e}"))
    })?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(CleanClawError::InvalidArgument(format!(
            "attachment {url}: {} bytes exceeds {MAX_ATTACHMENT_BYTES} limit",
            bytes.len()
        )));
    }
    let filename = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty() && !s.contains('?'))
        .map(|s| s.to_string());
    Ok(Attachment {
        url: url.to_string(),
        content_type,
        bytes: bytes.to_vec(),
        filename,
    })
}

pub fn is_supported_image(ct: &str) -> bool {
    matches!(
        ct,
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
    )
}

/// Convert resolved attachments into `ContentPart` values the LLM
/// provider can render.
pub fn to_content_parts(atts: &[Attachment]) -> Vec<ContentPart> {
    let engine = base64::engine::general_purpose::STANDARD;
    atts.iter()
        .map(|a| ContentPart::ImageBase64 {
            media_type: a.content_type.clone(),
            data: engine.encode(&a.bytes),
        })
        .collect()
}

/// Build a single user message from text + attachments.
pub fn user_message_with_attachments(text: &str, atts: &[Attachment]) -> Message {
    let mut m = Message::user(text);
    m.content_parts = to_content_parts(atts);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_image_formats() {
        assert!(is_supported_image("image/png"));
        assert!(is_supported_image("image/jpeg"));
        assert!(is_supported_image("image/webp"));
        assert!(!is_supported_image("application/pdf"));
        assert!(!is_supported_image("text/plain"));
    }

    #[test]
    fn content_parts_encode_bytes() {
        let a = Attachment {
            url: "https://x.test/i.png".into(),
            content_type: "image/png".into(),
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0],
            filename: Some("i.png".into()),
        };
        let parts = to_content_parts(&[a]);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            ContentPart::ImageBase64 { media_type, data } => {
                assert_eq!(media_type, "image/png");
                assert!(!data.is_empty());
            }
            _ => panic!(),
        }
    }
}
