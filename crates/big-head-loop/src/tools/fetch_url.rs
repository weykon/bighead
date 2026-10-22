//! `fetch_url` tool —— read one URL selected from `search` results.
//!
//! `search` finds candidate URLs; this tool reads a specific URL body. It has
//! two backends:
//! - native HTTP + lightweight HTML extraction (always available)
//! - optional crawl4ai-compatible external command via `BIG_HEAD_CRAWL4AI_CMD`
//!
//! The crawl4ai command is invoked as: `<cmd> <url>` and may print either plain
//! readable text or a JSON object containing one of `markdown`, `text`,
//! `content`, or `html`.

use std::time::Duration;

use big_head_ai::{ToolCall, ToolCallResult};
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;

use crate::tools::html_utils::{extract_links, extract_title, html_to_readable_text};
use crate::tools::http_client::build_client;

const FETCH_USER_AGENT: &str = "big-head/0.1 (fetch_url; agent runtime)";
const DEFAULT_LIMIT_CHARS: usize = 8_000;
const MAX_LIMIT_CHARS: usize = 30_000;
const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_MAX_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 20;
const MAX_TIMEOUT_SECS: u64 = 60;
const MAX_LINKS: usize = 50;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FetchMode {
    Readable,
    Raw,
}

#[derive(Debug)]
struct FetchArgs {
    url: String,
    mode: FetchMode,
    limit: usize,
    max_bytes: usize,
    include_links: bool,
    prefer_crawl4ai: bool,
    timeout_secs: u64,
}

pub(crate) async fn dispatch_fetch_url(call: &ToolCall) -> ToolCallResult {
    match parse_args(call) {
        Ok(args) => match fetch_url(args).await {
            Ok(output) => ToolCallResult {
                tool_use_id: call.id.clone(),
                output,
                is_error: false,
            },
            Err(e) => ToolCallResult {
                tool_use_id: call.id.clone(),
                output: format!("fetch_url failed: {e}"),
                is_error: true,
            },
        },
        Err(e) => ToolCallResult {
            tool_use_id: call.id.clone(),
            output: e,
            is_error: true,
        },
    }
}

fn parse_args(call: &ToolCall) -> Result<FetchArgs, String> {
    let url = call
        .input
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing or empty 'url' field".to_string())?
        .to_string();

    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("fetch_url only supports http:// and https:// URLs".into());
    }

    let mode = call
        .input
        .get("mode")
        .cloned()
        .map(serde_json::from_value::<FetchMode>)
        .transpose()
        .map_err(|e| format!("Invalid 'mode' field: {e}. Expected readable or raw."))?
        .unwrap_or(FetchMode::Readable);

    let limit = call
        .input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, MAX_LIMIT_CHARS))
        .unwrap_or(DEFAULT_LIMIT_CHARS);

    let max_bytes = call
        .input
        .get("max_bytes")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, MAX_MAX_BYTES))
        .unwrap_or(DEFAULT_MAX_BYTES);

    let include_links = call
        .input
        .get("include_links")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let prefer_crawl4ai = call
        .input
        .get("prefer_crawl4ai")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let timeout_secs = call
        .input
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, MAX_TIMEOUT_SECS))
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    Ok(FetchArgs {
        url,
        mode,
        limit,
        max_bytes,
        include_links,
        prefer_crawl4ai,
        timeout_secs,
    })
}

async fn fetch_url(args: FetchArgs) -> Result<String, String> {
    if args.prefer_crawl4ai {
        if let Some(cmd) = std::env::var("BIG_HEAD_CRAWL4AI_CMD")
            .ok()
            .filter(|s| !s.trim().is_empty())
        {
            match fetch_with_crawl4ai(&cmd, &args).await {
                Ok(body) => return Ok(body),
                Err(e) => {
                    let native = fetch_native(&args).await?;
                    return Ok(add_backend_warning(native, &e));
                }
            }
        }
    }

    fetch_native(&args).await
}

async fn fetch_with_crawl4ai(cmd: &str, args: &FetchArgs) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(args.timeout_secs),
        Command::new(cmd).arg(&args.url).output(),
    )
    .await
    .map_err(|_| format!("crawl4ai command timed out after {}s", args.timeout_secs))?
    .map_err(|e| format!("run crawl4ai command '{cmd}': {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "crawl4ai command exited with {}: {}",
            output.status,
            stderr.chars().take(500).collect::<String>()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let extracted = extract_crawl4ai_text(&stdout);
    let (text, truncated) = truncate_chars(&extracted, args.limit);
    let envelope = json!({
        "success": true,
        "url": args.url,
        "final_url": args.url,
        "backend": "crawl4ai",
        "mode": mode_label(args.mode),
        "title": null,
        "content_type": "text/plain",
        "text": text,
        "links": [],
        "truncated": truncated,
        "bytes_downloaded": stdout.len()
    });
    serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())
}

async fn fetch_native(args: &FetchArgs) -> Result<String, String> {
    let client = build_client(FETCH_USER_AGENT)?;
    let response = client
        .get(&args.url)
        .send()
        .await
        .map_err(|e| format!("request {}: {e}", args.url))?;

    let status = response.status();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !status.is_success() {
        return Err(format!("HTTP {status} for {}", args.url));
    }

    if let Some(len) = response.content_length() {
        if len as usize > args.max_bytes {
            return Err(format!(
                "response too large: content-length={} bytes, max_bytes={}",
                len, args.max_bytes
            ));
        }
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read response body: {e}"))?;
    if bytes.len() > args.max_bytes {
        return Err(format!(
            "response too large: downloaded={} bytes, max_bytes={}",
            bytes.len(),
            args.max_bytes
        ));
    }

    let body = String::from_utf8_lossy(&bytes).to_string();
    let is_html = content_type.contains("text/html") || looks_like_html(&body);
    let is_json = content_type.contains("application/json") || body.trim_start().starts_with('{');

    let title = if is_html { extract_title(&body) } else { None };
    let raw_text = match args.mode {
        FetchMode::Readable if is_html => html_to_readable_text(&body),
        FetchMode::Readable if is_json => body.clone(),
        FetchMode::Readable => body.clone(),
        FetchMode::Raw => body.clone(),
    };
    let (text, truncated) = truncate_chars(&raw_text, args.limit);

    let links = if args.include_links && is_html {
        extract_links(&body, MAX_LINKS)
            .into_iter()
            .map(|(text, url)| json!({ "text": text, "url": url }))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let envelope = json!({
        "success": true,
        "url": args.url,
        "final_url": final_url,
        "backend": "native",
        "mode": mode_label(args.mode),
        "title": title,
        "content_type": content_type,
        "text": text,
        "links": links,
        "truncated": truncated,
        "bytes_downloaded": bytes.len()
    });

    serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())
}

fn extract_crawl4ai_text(stdout: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout) {
        for key in ["markdown", "text", "content", "html"] {
            if let Some(s) = v.get(key).and_then(|v| v.as_str()) {
                if key == "html" {
                    return html_to_readable_text(s);
                }
                return s.to_string();
            }
        }
    }
    stdout.to_string()
}

fn add_backend_warning(native_output: String, warning: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&native_output) else {
        return native_output;
    };
    if let Some(obj) = v.as_object_mut() {
        obj.insert("backend_warning".into(), json!(warning));
    }
    serde_json::to_string_pretty(&v).unwrap_or(native_output)
}

fn truncate_chars(text: &str, limit: usize) -> (String, bool) {
    let mut out = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        if count >= limit {
            return (out, true);
        }
        out.push(ch);
        count += 1;
    }
    (out, false)
}

fn looks_like_html(body: &str) -> bool {
    let prefix = body.chars().take(500).collect::<String>().to_lowercase();
    prefix.contains("<html") || prefix.contains("<!doctype html") || prefix.contains("<body")
}

fn mode_label(mode: FetchMode) -> &'static str {
    match mode {
        FetchMode::Readable => "readable",
        FetchMode::Raw => "raw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: "fetch_url".into(),
            input,
        }
    }

    #[test]
    fn parse_args_rejects_non_http_url() {
        let err = parse_args(&call(json!({"url": "file:///etc/passwd"}))).unwrap_err();
        assert!(err.contains("http:// and https://"));
    }

    #[test]
    fn parse_args_defaults() {
        let args = parse_args(&call(json!({"url": "https://example.com"}))).unwrap();
        assert_eq!(args.mode, FetchMode::Readable);
        assert_eq!(args.limit, DEFAULT_LIMIT_CHARS);
        assert!(args.include_links);
        assert!(args.prefer_crawl4ai);
    }

    #[test]
    fn crawl4ai_json_stdout_extracts_markdown() {
        let text = extract_crawl4ai_text(r##"{"markdown":"# Hello"}"##);
        assert_eq!(text, "# Hello");
    }

    #[test]
    fn truncate_reports_when_cut() {
        let (text, truncated) = truncate_chars("abcdef", 3);
        assert_eq!(text, "abc");
        assert!(truncated);
    }
}
