//! Terminal renderer — subscribes to [`StateHook`] and renders activities
//! with ANSI colors and Unicode symbols.

use crate::{AgentActivity, StateHook, WaveContentKind};
use std::io::Write; // for flush()
use tokio::sync::broadcast;

/// Renders agent activities to the terminal with ANSI colors.
///
/// Spawn via [`TerminalRenderer::start()`] — runs a `tokio::spawn`'d
/// subscriber that formats and prints each activity.
pub struct TerminalRenderer {
    hook: StateHook,
}

impl TerminalRenderer {
    pub fn new(hook: StateHook) -> Self {
        Self { hook }
    }

    /// Start the renderer. Returns a join handle for the spawned task.
    ///
    /// The task runs until all [`StateHook`] senders are dropped.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // 禁用鼠标滚轮事件，避免滚轮信号进入 stdin 输入框
            // 发送 DEC private mode off (mouse wheel mode off)
            print!("\x1b[?1000l");
            std::io::stdout().flush().ok();

            let mut rx = self.hook.subscribe();
            loop {
                match rx.recv().await {
                    Ok(activity) => Self::render(&activity),
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("\x1b[33m[state] skipped {} activities\x1b[0m", n);
                    }
                }
            }
        })
    }

    fn render(activity: &AgentActivity) {
        match activity {
            AgentActivity::Idle => {}

            AgentActivity::Reasoning { wave } => {
                println!("\x1b[36;1m  🧠 Wave {} ── reasoning\x1b[0m", wave);
            }

            AgentActivity::WaveContent {
                wave,
                kind,
                content,
                truncated,
            } => {
                let (icon, label, color) = match kind {
                    WaveContentKind::Thinking => ("💭", "returned thinking", "\x1b[35m"),
                    WaveContentKind::Text => ("💬", "intermediate response", "\x1b[36m"),
                };
                let suffix = if *truncated {
                    " (display truncated)"
                } else {
                    ""
                };
                println!("{color}  {icon} Wave {wave} {label}{suffix}:\x1b[0m");
                render_indented(content, color);
                if *truncated {
                    println!("\x1b[33m     [... full content remains in agent history]\x1b[0m");
                }
            }

            AgentActivity::WaveDecision {
                wave,
                stop_reason,
                tools,
            } => {
                let stop = stop_reason.as_deref().unwrap_or("unknown");
                let tools = if tools.is_empty() {
                    "none".to_string()
                } else {
                    tools.join(", ")
                };
                println!(
                    "\x1b[36m  🧭 Wave {} decision: stop={} · tools={}\x1b[0m",
                    wave, stop, tools
                );
            }

            AgentActivity::TokenBreakdown {
                wave,
                system_prompt_tokens,
                initial_context_tokens,
                conversation_tokens,
                tool_definitions_tokens,
                total_estimated,
                message_count,
                context_usage_pct,
            } => {
                println!(
                    "\x1b[36m  📊 Wave {} Token Breakdown ({} messages, {:.1}% used):\x1b[0m",
                    wave, message_count, context_usage_pct
                );
                println!(
                    "\x1b[36m     System Prompt:    {:>7} tokens\x1b[0m",
                    system_prompt_tokens
                );
                println!(
                    "\x1b[36m     Initial Context:  {:>7} tokens  (Spark + World)\x1b[0m",
                    initial_context_tokens
                );
                println!(
                    "\x1b[36m     Conversation:     {:>7} tokens  (History)\x1b[0m",
                    conversation_tokens
                );
                println!(
                    "\x1b[36m     Tool Definitions: {:>7} tokens\x1b[0m",
                    tool_definitions_tokens
                );
                println!(
                    "\x1b[36;1m     Total:            {:>7} tokens\x1b[0m",
                    total_estimated
                );
            }

            AgentActivity::ApiRequest { provider, model } => {
                println!("\x1b[34m  📡 API → {} ({})\x1b[0m", provider, model);
            }

            AgentActivity::ApiRetry {
                provider,
                model,
                attempt,
                max_attempts,
                delay_ms,
                error,
            } => {
                let secs = *delay_ms as f64 / 1000.0;
                let err = truncate_utf8(error, 96);
                println!(
                    "\x1b[33m  🔁 API Reconnecting... {}/{} → {} ({}) in {:.1}s ── {}\x1b[0m",
                    attempt, max_attempts, provider, model, secs, err
                );
            }

            AgentActivity::ApiResponse {
                input_tokens,
                output_tokens,
                duration_ms,
            } => {
                let secs = *duration_ms as f64 / 1000.0;
                println!(
                    "\x1b[34m  📡 API ← {}→{} tokens · {:.1}s\x1b[0m",
                    input_tokens, output_tokens, secs
                );
            }

            AgentActivity::OutputWarning {
                output_tokens,
                max_tokens,
                stop_reason,
                level,
            } => {
                let stop = stop_reason.as_deref().unwrap_or("unknown");
                let (icon, color) = match level.as_str() {
                    "truncated" => ("🚫", "\x1b[31;1m"),
                    _ => ("⚠️ ", "\x1b[33m"),
                };
                println!(
                    "{}  {} Output limit: {}/{} tokens, stop_reason={} ── response may be incomplete\x1b[0m",
                    color, icon, output_tokens, max_tokens, stop
                );
            }

            AgentActivity::ToolExecution { trace_id, tx_id, tool_call_id, tool, status, elapsed_ms } => {
                println!("Tool {status}: {tool} trace={trace_id} tx={tx_id} call={tool_call_id} elapsed_ms={elapsed_ms:?}");
            }
            AgentActivity::ToolCall {
                wave,
                tool,
                input_summary,
            } => {
                let summary = truncate_utf8(input_summary, 80);
                println!("\x1b[33m  🔧 Wave {}: {} ({})\x1b[0m", wave, tool, summary);
            }

            AgentActivity::ToolResult {
                tool,
                is_error,
                output_summary,
            } => {
                let icon = if *is_error { "✗" } else { "✓" };
                let color = if *is_error { "\x1b[31m" } else { "\x1b[32m" };
                let summary = truncate_utf8(output_summary, 120);
                println!("{}    {} {} ── {}\x1b[0m", color, icon, tool, summary);
            }

            AgentActivity::ActionApplied { action } => {
                println!("\x1b[35m  ⚡ {}\x1b[0m", action);
            }

            AgentActivity::BudgetWarning { used, limit, level } => {
                let pct = (*used as f64 / *limit as f64 * 100.0) as u32;
                let (icon, color) = match level.as_str() {
                    "exceeded" => ("🚫", "\x1b[31;1m"),
                    _ => ("⚠️ ", "\x1b[33m"),
                };
                println!(
                    "{}  {} Budget: {}/{} ({}%)\x1b[0m",
                    color, icon, used, limit, pct
                );
            }

            AgentActivity::ShardProgress {
                paradigm,
                completed,
                total,
            } => {
                println!(
                    "\x1b[35m  📦 Shard {}: {}/{}\x1b[0m",
                    paradigm, completed, total
                );
            }

            AgentActivity::Done {
                iterations,
                actions,
            } => {
                println!(
                    "\x1b[32;1m  ✅ Done: {} waves, {} actions\x1b[0m",
                    iterations, actions
                );
            }

            AgentActivity::Error { source, message } => {
                println!("\x1b[31;1m  💥 {}: {}\x1b[0m", source, message);
            }

            AgentActivity::WebSearch { query, provider } => {
                let q = truncate_utf8(query, 60);
                println!("\x1b[36m  🔍 WebSearch [{}] ── \"{}\"\x1b[0m", provider, q);
            }

            AgentActivity::WebSearchDone {
                query,
                provider,
                result_count,
                duration_ms,
            } => {
                let q = truncate_utf8(query, 60);
                let secs = *duration_ms as f64 / 1000.0;
                println!(
                    "\x1b[36m  🔍 WebSearch [{}] ← {} results · {:.2}s · \"{}\"\x1b[0m",
                    provider, result_count, secs, q
                );
            }

            AgentActivity::WikiSearch { query, lang } => {
                let q = truncate_utf8(query, 60);
                println!("\x1b[36;1m  📚 WikiSearch [{}] ── \"{}\"\x1b[0m", lang, q);
            }

            AgentActivity::WikiSearchDone {
                query,
                lang,
                result_count,
                duration_ms,
            } => {
                let q = truncate_utf8(query, 60);
                let secs = *duration_ms as f64 / 1000.0;
                println!(
                    "\x1b[36;1m  📚 WikiSearch [{}] ← {} results · {:.2}s · \"{}\"\x1b[0m",
                    lang, result_count, secs, q
                );
            }
        }
    }
}

fn render_indented(content: &str, color: &str) {
    for line in content.lines() {
        let line = sanitize_terminal_line(line);
        println!("{color}     {line}\x1b[0m");
    }
}

fn sanitize_terminal_line(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len());
    for ch in line.chars() {
        match ch {
            '\t' => sanitized.push_str("    "),
            ch if ch.is_control() => {}
            ch => sanitized.push(ch),
        }
    }
    sanitized
}

/// Truncate a string to at most `max_chars` characters, appending "..." if truncated.
/// Safe for multi-byte UTF-8 (Chinese, emoji, etc.).
fn truncate_utf8(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        if max_chars <= 3 {
            ".".repeat(max_chars)
        } else {
            let truncated: String = s.chars().take(max_chars - 3).collect();
            format!("{}...", truncated)
        }
    } else {
        s.to_string()
    }
}
