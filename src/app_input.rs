use std::fs;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::{Arc, Mutex};

use rustyline::{
    Cmd, ConditionalEventHandler, DefaultEditor, Event, EventContext, EventHandler, KeyEvent,
    RepeatCount,
};

const PASTE_IMAGE_COMMAND: &str = "/paste-image";
const IMAGE_PASTE_FAILED_PLACEHOLDER: &str = "【图片粘贴失败】";

#[derive(Debug, Clone)]
pub struct ClipboardImageAttachment {
    pub id: usize,
    pub path: PathBuf,
}

pub type ClipboardImageStore = Arc<Mutex<Vec<ClipboardImageAttachment>>>;

#[derive(Clone)]
struct PasteImageHandler {
    attachments: ClipboardImageStore,
}

impl ConditionalEventHandler for PasteImageHandler {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, _: &EventContext) -> Option<Cmd> {
        let placeholder = match save_clipboard_image() {
            Ok(path) => {
                let mut attachments = self.attachments.lock().unwrap();
                let id = attachments.last().map_or(1, |attachment| attachment.id + 1);
                attachments.push(ClipboardImageAttachment { id, path });
                image_placeholder(id)
            }
            Err(_) => IMAGE_PASTE_FAILED_PLACEHOLDER.to_string(),
        };
        Some(Cmd::Insert(1, placeholder))
    }
}

pub fn new_clipboard_image_store() -> ClipboardImageStore {
    Arc::new(Mutex::new(Vec::new()))
}

pub fn bind_paste_image_keys(editor: &mut DefaultEditor, attachments: ClipboardImageStore) {
    editor.bind_sequence(
        KeyEvent::ctrl('V'),
        EventHandler::Conditional(Box::new(PasteImageHandler {
            attachments: attachments.clone(),
        })),
    );
    editor.bind_sequence(
        KeyEvent::alt('V'),
        EventHandler::Conditional(Box::new(PasteImageHandler { attachments })),
    );
}

pub fn paste_clipboard_image_placeholder(attachments: &ClipboardImageStore) -> String {
    match save_clipboard_image() {
        Ok(path) => {
            let mut attachments = attachments.lock().unwrap();
            let id = attachments.last().map_or(1, |attachment| attachment.id + 1);
            attachments.push(ClipboardImageAttachment { id, path });
            image_placeholder(id)
        }
        Err(_) => IMAGE_PASTE_FAILED_PLACEHOLDER.to_string(),
    }
}

pub fn line_to_spark(line: &str) -> Result<Option<String>, String> {
    let spark = line.trim();
    if spark.is_empty() {
        Ok(None)
    } else if spark == PASTE_IMAGE_COMMAND {
        spark_from_clipboard_image("").map(Some)
    } else if let Some(rest) = spark.strip_prefix(&format!("{PASTE_IMAGE_COMMAND} ")) {
        spark_from_clipboard_image(rest).map(Some)
    } else {
        Ok(Some(spark.to_string()))
    }
}

pub fn expand_image_placeholders(line: &str, attachments: &ClipboardImageStore) -> String {
    let mut images = attachments.lock().unwrap();
    if images.is_empty() {
        return line.to_string();
    }

    let mut text = line.to_string();
    let mut used_ids = Vec::new();
    let mut image_refs = Vec::new();

    for attachment in images.iter() {
        let placeholder = image_placeholder(attachment.id);
        if text.contains(&placeholder) {
            text = text.replace(&placeholder, &format!("图片#{}", attachment.id));
            used_ids.push(attachment.id);
            image_refs.push(format!("image:{}", attachment.path.display()));
        }
    }

    if !image_refs.is_empty() {
        text.push_str("\n\n");
        text.push_str(&image_refs.join("\n"));
        images.retain(|attachment| !used_ids.contains(&attachment.id));
    }

    text
}

fn image_placeholder(id: usize) -> String {
    format!("【图片#{id}】")
}

fn save_clipboard_image() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("big-head-clipboard-images");
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create temp image directory: {e}"))?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = dir.join(format!("clipboard-{}-{}.png", std::process::id(), ts));
    let bytes = clipboard_png_bytes()?;
    fs::write(&path, bytes).map_err(|e| format!("cannot save clipboard image: {e}"))?;
    Ok(path)
}

fn spark_from_clipboard_image(instruction: &str) -> Result<String, String> {
    let path = save_clipboard_image()?;

    let instruction = instruction.trim();
    let instruction = if instruction.is_empty() {
        "请查看这张剪贴板图片。"
    } else {
        instruction
    };

    Ok(format!("{instruction}\n\nimage:{}", path.display()))
}

#[cfg(target_os = "macos")]
fn clipboard_png_bytes() -> Result<Vec<u8>, String> {
    let output = Command::new("osascript")
        .args([
            "-e",
            "try",
            "-e",
            "set pngData to the clipboard as «class PNGf»",
            "-e",
            "return pngData",
            "-e",
            "on error errMsg",
            "-e",
            "error errMsg",
            "-e",
            "end try",
        ])
        .output()
        .map_err(|e| format!("cannot run osascript: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "clipboard does not contain a PNG image".into()
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let hex = parse_osascript_png_hex(&stdout)
        .ok_or("clipboard does not contain PNG image data returned by osascript")?;
    decode_hex_bytes(&hex)
}

#[cfg(not(target_os = "macos"))]
fn clipboard_png_bytes() -> Result<Vec<u8>, String> {
    Err("clipboard image paste is currently implemented for macOS only".into())
}

fn parse_osascript_png_hex(output: &str) -> Option<String> {
    let start = output.find("PNGf")? + "PNGf".len();
    let tail = &output[start..];
    let end = tail.find('»').unwrap_or(tail.len());
    let hex = tail[..end]
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>();
    (!hex.is_empty()).then_some(hex)
}

fn decode_hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 {
        return Err("clipboard PNG data has odd-length hex payload".into());
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|e| format!("clipboard PNG data is not valid hex: {e}"))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_clipboard_command_remains_plain_spark() {
        let spark = line_to_spark("/paste-image-later").unwrap().unwrap();
        assert_eq!(spark, "/paste-image-later");
    }

    #[test]
    fn empty_line_is_ignored() {
        assert!(line_to_spark("  ").unwrap().is_none());
    }

    #[test]
    fn parses_osascript_png_data() {
        let hex = parse_osascript_png_hex("«data PNGf89504E470D0A»\n").unwrap();
        assert_eq!(hex, "89504E470D0A");
    }

    #[test]
    fn decodes_hex_payload() {
        let bytes = decode_hex_bytes("89504E47").unwrap();
        assert_eq!(bytes, vec![0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn expands_image_placeholders_into_image_refs() {
        let attachments = Arc::new(Mutex::new(vec![ClipboardImageAttachment {
            id: 1,
            path: PathBuf::from("/tmp/a.png"),
        }]));

        let spark = expand_image_placeholders("看看【图片#1】", &attachments);

        assert_eq!(spark, "看看图片#1\n\nimage:/tmp/a.png");
        assert!(attachments.lock().unwrap().is_empty());
    }

    #[test]
    fn image_placeholder_text_is_stable() {
        assert_eq!(image_placeholder(3), "【图片#3】");
    }
}
