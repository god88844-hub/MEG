use super::ROLE_USER_MULTIMODAL;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(super) const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageAttachment {
    pub name: String,
    pub mime: String,
    pub data_base64: String,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MultimodalEnvelope {
    pub(super) text: String,
    pub(super) images: Vec<ImageAttachment>,
}


pub(super) fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}


pub(super) fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}


/// Snapshot and validate an image file from disk ONCE at user-attach time.
pub fn load_image_attachment(path: &std::path::Path) -> anyhow::Result<ImageAttachment> {
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("Cannot read image attachment {}: {e}", path.display()))?;
    anyhow::ensure!(meta.is_file(), "Image attachment is not a regular file");
    anyhow::ensure!(
        meta.len() <= MAX_IMAGE_BYTES,
        "Image attachment exceeds 10 MiB limit: {}",
        path.display()
    );
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("Failed to read image bytes {}: {e}", path.display()))?;
    let mime = detect_image_mime(&bytes)
        .ok_or_else(|| anyhow::anyhow!("Unsupported or invalid image header: {}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    Ok(ImageAttachment {
        name,
        mime: mime.to_string(),
        data_base64: base64_encode(&bytes),
    })
}


/// Encode a trusted user message with pre-snapshotted image attachments.
pub fn encode_user_multimodal(text: &str, images: &[ImageAttachment]) -> crate::ChatMessage {
    if images.is_empty() {
        return crate::ChatMessage {
            role: "user".into(),
            content: text.to_string(),
        };
    }
    let env = MultimodalEnvelope {
        text: text.to_string(),
        images: images.to_vec(),
    };
    crate::ChatMessage {
        role: ROLE_USER_MULTIMODAL.into(),
        content: serde_json::to_string(&env).unwrap_or_else(|_| text.to_string()),
    }
}


pub(super) fn openai_multimodal_content(raw_envelope: &str) -> Value {
    let Ok(env) = serde_json::from_str::<MultimodalEnvelope>(raw_envelope) else {
        return Value::String(raw_envelope.to_string());
    };
    let mut parts = Vec::with_capacity(env.images.len() + 1);
    if !env.text.is_empty() {
        parts.push(json!({ "type": "text", "text": env.text }));
    }
    for img in env.images {
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", img.mime, img.data_base64) }
        }));
    }
    Value::Array(parts)
}


pub(super) fn anthropic_multimodal_content(raw_envelope: &str) -> Value {
    let Ok(env) = serde_json::from_str::<MultimodalEnvelope>(raw_envelope) else {
        return Value::String(raw_envelope.to_string());
    };
    let mut parts = Vec::with_capacity(env.images.len() + 1);
    if !env.text.is_empty() {
        parts.push(json!({ "type": "text", "text": env.text }));
    }
    for img in env.images {
        parts.push(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": img.mime, "data": img.data_base64 }
        }));
    }
    Value::Array(parts)
}

pub(super) fn openai_responses_multimodal_content(raw_envelope: &str) -> Value {
    let Ok(env) = serde_json::from_str::<MultimodalEnvelope>(raw_envelope) else {
        return Value::String(raw_envelope.to_string());
    };
    let mut parts = Vec::with_capacity(env.images.len() + 1);
    if !env.text.is_empty() {
        parts.push(json!({ "type": "input_text", "text": env.text }));
    }
    for img in env.images {
        parts.push(json!({
            "type": "input_image",
            "image_url": format!("data:{};base64,{}", img.mime, img.data_base64)
        }));
    }
    Value::Array(parts)
}


pub(super) fn gemini_multimodal_parts(raw_envelope: &str) -> Vec<Value> {
    let Ok(env) = serde_json::from_str::<MultimodalEnvelope>(raw_envelope) else {
        return vec![json!({ "text": raw_envelope })];
    };
    let mut parts = Vec::with_capacity(env.images.len() + 1);
    if !env.text.is_empty() {
        parts.push(json!({ "text": env.text }));
    }
    for img in env.images {
        parts.push(json!({
            "inlineData": {
                "mimeType": img.mime,
                "data": img.data_base64
            }
        }));
    }
    parts
}
