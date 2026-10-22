//! Image attachment extraction for spark text.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use base64::Engine;
use big_head_ai::{ContentBlock, ImageSource};

const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageInputWarning {
    pub reference: String,
    pub message: String,
}

pub(crate) fn image_blocks_from_text(text: &str) -> (Vec<ContentBlock>, Vec<ImageInputWarning>) {
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = BTreeSet::new();

    for reference in extract_image_references(text) {
        if !seen.insert(reference.clone()) {
            continue;
        }

        match image_block_from_reference(&reference) {
            Ok(block) => blocks.push(block),
            Err(message) => warnings.push(ImageInputWarning { reference, message }),
        }
    }

    (blocks, warnings)
}

fn image_block_from_reference(reference: &str) -> Result<ContentBlock, String> {
    if is_http_url(reference) {
        if !looks_like_supported_image(reference) {
            return Err("URL does not look like a supported image".into());
        }
        return Ok(ContentBlock::Image {
            source: ImageSource::Url {
                url: reference.to_string(),
            },
            detail: None,
        });
    }

    let path = PathBuf::from(reference);
    let metadata = std::fs::metadata(&path).map_err(|e| format!("read metadata: {e}"))?;
    if !metadata.is_file() {
        return Err("path is not a file".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "image is too large: {} bytes, max {}",
            metadata.len(),
            MAX_IMAGE_BYTES
        ));
    }

    let media_type = media_type_for_path(&path)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("read image: {e}"))?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);

    Ok(ContentBlock::Image {
        source: ImageSource::Base64 { media_type, data },
        detail: None,
    })
}

fn extract_image_references(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(normalize_candidate)
        .filter(|candidate| is_http_url(candidate) || looks_like_supported_image(candidate))
        .collect()
}

fn normalize_candidate(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '，'
                    | '。'
                    | '、'
            )
        })
        .trim_end_matches(|c: char| matches!(c, ',' | ';' | ':' | '!' | '?' | '，' | '。'));

    let without_prefix = trimmed
        .strip_prefix("image:")
        .or_else(|| trimmed.strip_prefix("img:"))
        .or_else(|| trimmed.strip_prefix("图片:"))
        .unwrap_or(trimmed);

    if without_prefix.is_empty() {
        None
    } else {
        Some(without_prefix.to_string())
    }
}

fn media_type_for_path(path: &Path) -> Result<String, String> {
    let media_type = mime_guess::from_path(path)
        .first()
        .map(|mime| mime.essence_str().to_string())
        .ok_or_else(|| "unsupported image media type".to_string())?;

    if is_supported_media_type(&media_type) {
        Ok(media_type)
    } else {
        Err(format!("unsupported image media type: {media_type}"))
    }
}

fn looks_like_supported_image(reference: &str) -> bool {
    let lower = reference
        .split(['?', '#'])
        .next()
        .unwrap_or(reference)
        .to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn is_supported_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn extracts_url_image_blocks() {
        let (blocks, warnings) =
            image_blocks_from_text("看看 https://example.com/cat.png?size=large 这张图");
        assert!(warnings.is_empty());
        assert_eq!(blocks.len(), 1);
        let ContentBlock::Image {
            source: ImageSource::Url { url },
            ..
        } = &blocks[0]
        else {
            panic!("expected url image block");
        };
        assert_eq!(url, "https://example.com/cat.png?size=large");
    }

    #[test]
    fn encodes_local_image_blocks() {
        let path = std::env::temp_dir().join(format!("big-head-image-{}.png", Uuid::new_v4()));
        std::fs::write(&path, [0x89, b'P', b'N', b'G']).unwrap();

        let (blocks, warnings) = image_blocks_from_text(&format!("看看 {}", path.display()));
        let _ = std::fs::remove_file(&path);

        assert!(warnings.is_empty());
        assert_eq!(blocks.len(), 1);
        let ContentBlock::Image {
            source: ImageSource::Base64 { media_type, data },
            ..
        } = &blocks[0]
        else {
            panic!("expected base64 image block");
        };
        assert_eq!(media_type, "image/png");
        assert!(!data.is_empty());
    }

    #[test]
    fn reports_unreadable_local_images() {
        let missing = std::env::temp_dir().join(format!("missing-{}.png", Uuid::new_v4()));
        let (blocks, warnings) = image_blocks_from_text(&format!("图片:{}", missing.display()));

        assert!(blocks.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("read metadata"));
    }
}
