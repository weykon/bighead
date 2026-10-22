//! Project-scoped file discovery without spawning a shell.

use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Read},
    path::Path,
};

pub(crate) fn inspect(root: &Path, input: &Value) -> Result<Value, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let requested = input.get("path").and_then(Value::as_str).unwrap_or(".");
    let path = root
        .join(requested)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("path must stay within the project root".into());
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(80)
        .clamp(1, 200) as usize;
    if path.is_file() {
        let offset = input
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(100_000) as usize;
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let byte_limit = 2 * 1024 * 1024;
        let byte_limited = file.metadata().map_err(|e| e.to_string())?.len() > byte_limit;
        let lines: Vec<_> = BufReader::new(file.take(byte_limit)).lines().skip(offset)
            .take(limit).enumerate().map(|(i, line)| {
                line.map(|line| json!({"line": offset + i + 1, "text": line.chars().take(500).collect::<String>()}))
            }).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        return Ok(
            json!({"path": requested, "lines": lines, "byte_limited": byte_limited,
            "next_offset": offset + lines.len(), "max_line_chars": 500}),
        );
    }
    let filter = input
        .get("name_contains")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut pending = VecDeque::from([path]);
    let mut files = Vec::new();
    let mut scanned = 0usize;
    while let Some(dir) = pending.pop_front() {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            scanned += 1;
            if scanned > 5000 || files.len() >= limit {
                return Ok(json!({"files": files, "truncated": true, "scanned": scanned - 1}));
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                if !matches!(
                    name.as_str(),
                    ".git" | "target" | "vendor" | "node_modules" | "big_head_worlds" | "dist"
                ) {
                    pending.push_back(entry.path());
                }
            } else if kind.is_file() && name.contains(filter) {
                files.push(
                    entry
                        .path()
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    files.sort();
    Ok(json!({"files": files, "truncated": false, "scanned": scanned}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bounded_lines_and_rejects_paths_outside_root() {
        let root = std::env::temp_dir().join(format!("inspect-project-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("sample.rs"), "one\ntwo\nthree\n").unwrap();
        let value = inspect(&root, &json!({"path":"sample.rs", "offset":1, "limit":1})).unwrap();
        assert_eq!(value["lines"][0]["text"], "two");
        assert_eq!(value["next_offset"], 2);
        assert!(inspect(&root, &json!({"path":".."})).is_err());
        let value = inspect(&root, &json!({"name_contains":"sample"})).unwrap();
        assert_eq!(value["files"], json!(["sample.rs"]));
        fs::remove_dir_all(root).unwrap();
    }
}
