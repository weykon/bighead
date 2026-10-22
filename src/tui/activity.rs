use big_head_state::{AgentActivity, WaveContentKind};

const WAVE_CONTENT_PREVIEW_CHARS: usize = 1_200;
const STATUS_PREVIEW_CHARS: usize = 180;

#[derive(Debug, Clone)]
pub(super) struct ActivityState {
    status: String,
    current_wave: Option<usize>,
    current_api: Option<String>,
    last_api_response: Option<String>,
    last_token_breakdown: Option<TokenBreakdownView>,
    last_budget: Option<String>,
    last_done: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActivityLine {
    pub(super) text: String,
    pub(super) kind: ActivityLineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActivityLineKind {
    Normal,
    Info,
    Api,
    Token,
    ToolCall,
    ToolResult,
    Action,
    Search,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TokenBreakdownView {
    pub(super) wave: usize,
    pub(super) system_prompt_tokens: usize,
    pub(super) initial_context_tokens: usize,
    pub(super) conversation_tokens: usize,
    pub(super) tool_definitions_tokens: usize,
    pub(super) total_estimated: usize,
    pub(super) message_count: usize,
    pub(super) context_usage_pct: String,
}

impl Default for ActivityState {
    fn default() -> Self {
        Self {
            status: "Idle".into(),
            current_wave: None,
            current_api: None,
            last_api_response: None,
            last_token_breakdown: None,
            last_budget: None,
            last_done: None,
        }
    }
}

impl ActivityState {
    pub(super) fn status(&self) -> &str {
        &self.status
    }

    pub(super) fn apply(&mut self, activity: AgentActivity) -> Option<ActivityLine> {
        let line = match activity {
            AgentActivity::ToolExecution { trace_id, tx_id, tool_call_id, tool, status, elapsed_ms } => {
                let text = format!("{tool}: {status} · {}ms · trace={trace_id} tx={tx_id} call={tool_call_id}", elapsed_ms.unwrap_or(0));
                self.status = format!("{tool}: {status}");
                Some(if status == "failed" { ActivityLine::error(text) } else { ActivityLine::info(text) })
            }
            AgentActivity::Idle => {
                self.status = "Idle".into();
                None
            }
            AgentActivity::Reasoning { wave } => {
                self.current_wave = Some(wave);
                self.status = format!("Wave {wave}: reasoning");
                Some(ActivityLine::token(format!("Wave {wave}: reasoning")))
            }
            AgentActivity::WaveContent {
                wave,
                kind,
                content,
                truncated,
            } => {
                let label = wave_content_label(kind);
                let suffix = if truncated {
                    " [display truncated]"
                } else {
                    ""
                };
                let text = format!(
                    "Wave {wave} {label}: {}{suffix}",
                    preview(&content, WAVE_CONTENT_PREVIEW_CHARS)
                );
                self.status = format!("Wave {wave}: {label}");
                Some(ActivityLine::info(text))
            }
            AgentActivity::WaveDecision {
                wave,
                stop_reason,
                tools,
            } => {
                let text = wave_decision_text(wave, stop_reason.as_deref(), &tools);
                self.status = text.clone();
                Some(ActivityLine::token(text))
            }
            AgentActivity::ApiRequest { provider, model } => {
                let text = format!("API -> {provider} ({model})");
                self.current_api = Some(text.clone());
                self.status = text.clone();
                Some(ActivityLine::api(text))
            }
            AgentActivity::ApiRetry {
                provider,
                model,
                attempt,
                max_attempts,
                delay_ms,
                error,
            } => {
                let text = format!(
                    "Reconnecting... {attempt}/{max_attempts} API -> {provider} ({model}) in {:.1}s · {}",
                    delay_ms as f64 / 1000.0,
                    one_line(&error)
                );
                self.current_api = Some(text.clone());
                self.status = text.clone();
                Some(ActivityLine::warning(text))
            }
            AgentActivity::ApiResponse {
                input_tokens,
                output_tokens,
                duration_ms,
            } => {
                let text = format!(
                    "API <- {input_tokens}->{output_tokens} tokens · {:.1}s",
                    duration_ms as f64 / 1000.0
                );
                self.last_api_response = Some(text.clone());
                self.status = text.clone();
                Some(ActivityLine::api(text))
            }
            AgentActivity::OutputWarning {
                output_tokens,
                max_tokens,
                stop_reason,
                level,
            } => {
                let text = format!(
                    "Output {level}: {output_tokens}/{max_tokens} tokens, stop_reason={}",
                    stop_reason.as_deref().unwrap_or("unknown")
                );
                self.status = text.clone();
                Some(ActivityLine::warning(text))
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
                self.last_token_breakdown = Some(TokenBreakdownView {
                    wave,
                    system_prompt_tokens,
                    initial_context_tokens,
                    conversation_tokens,
                    tool_definitions_tokens,
                    total_estimated,
                    message_count,
                    context_usage_pct: format!("{context_usage_pct:.1}%"),
                });
                let text = format!(
                    "Wave {wave}: {total_estimated} estimated tokens, {message_count} messages, {context_usage_pct:.1}% used"
                );
                self.status = text.clone();
                Some(ActivityLine::token(text))
            }
            AgentActivity::ToolCall {
                wave,
                tool,
                input_summary,
            } => {
                let text = format!("Wave {wave}: tool {tool}({})", one_line(&input_summary));
                self.status = text.clone();
                Some(ActivityLine::tool_call(text))
            }
            AgentActivity::ToolResult {
                tool,
                is_error,
                output_summary,
            } => {
                let mark = if is_error { "ERR" } else { "OK" };
                let text = format!("{mark} {tool}: {}", one_line(&output_summary));
                self.status = text.clone();
                Some(if is_error {
                    ActivityLine::error(text)
                } else {
                    ActivityLine::tool_result(text)
                })
            }
            AgentActivity::ActionApplied { action } => {
                let text = format!("Action: {}", one_line(&action));
                self.status = text.clone();
                Some(ActivityLine::action(text))
            }
            AgentActivity::BudgetWarning { used, limit, level } => {
                let pct = if limit == 0 {
                    0
                } else {
                    (used as f64 / limit as f64 * 100.0) as u32
                };
                let text = format!("Budget {level}: {used}/{limit} ({pct}%)");
                self.last_budget = Some(text.clone());
                self.status = text.clone();
                Some(ActivityLine::warning(text))
            }
            AgentActivity::ShardProgress {
                paradigm,
                completed,
                total,
            } => {
                let text = format!("Shard {paradigm}: {completed}/{total}");
                self.status = text.clone();
                Some(ActivityLine::token(text))
            }
            AgentActivity::Done {
                iterations,
                actions,
            } => {
                let text = format!("Done: {iterations} waves, {actions} actions");
                self.last_done = Some(text.clone());
                self.status = text.clone();
                Some(ActivityLine::success(text))
            }
            AgentActivity::Error { source, message } => {
                let text = format!("Error {source}: {message}");
                self.status = text.clone();
                Some(ActivityLine::error(text))
            }
            AgentActivity::WebSearch { query, provider } => {
                let text = format!("WebSearch {provider}: {}", one_line(&query));
                self.status = text.clone();
                Some(ActivityLine::search(text))
            }
            AgentActivity::WebSearchDone {
                query,
                provider,
                result_count,
                duration_ms,
            } => {
                let text = format!(
                    "WebSearch {provider}: {result_count} results · {duration_ms}ms · {}",
                    one_line(&query)
                );
                self.status = text.clone();
                Some(ActivityLine::search(text))
            }
            AgentActivity::WikiSearch { query, lang } => {
                let text = format!("WikiSearch {lang}: {}", one_line(&query));
                self.status = text.clone();
                Some(ActivityLine::search(text))
            }
            AgentActivity::WikiSearchDone {
                query,
                lang,
                result_count,
                duration_ms,
            } => {
                let text = format!(
                    "WikiSearch {lang}: {result_count} results · {duration_ms}ms · {}",
                    one_line(&query)
                );
                self.status = text.clone();
                Some(ActivityLine::search(text))
            }
        };

        line
    }

    pub(super) fn detail_lines(&self) -> Vec<ActivityLine> {
        let mut lines = Vec::new();
        if let Some(wave) = self.current_wave {
            lines.push(ActivityLine::info(format!("Wave: {wave}")));
        }
        if let Some(api) = &self.current_api {
            lines.push(ActivityLine::api(api.clone()));
        }
        if let Some(api) = &self.last_api_response {
            lines.push(ActivityLine::api(api.clone()));
        }
        if let Some(budget) = &self.last_budget {
            lines.push(ActivityLine::warning(budget.clone()));
        }
        if let Some(done) = &self.last_done {
            lines.push(ActivityLine::success(done.clone()));
        }
        if let Some(tokens) = &self.last_token_breakdown {
            lines.push(ActivityLine::info(format!(
                "Token breakdown: wave {}, {} messages, {} used",
                tokens.wave, tokens.message_count, tokens.context_usage_pct
            )));
            lines.push(ActivityLine::token(format!(
                "  system: {}  initial: {}  history: {}",
                tokens.system_prompt_tokens,
                tokens.initial_context_tokens,
                tokens.conversation_tokens
            )));
            lines.push(ActivityLine::token(format!(
                "  tools: {}  total: {}",
                tokens.tool_definitions_tokens, tokens.total_estimated
            )));
        }
        if lines.is_empty() {
            lines.push(ActivityLine::normal("Waiting for activity".into()));
        }
        lines
    }
}

impl ActivityLine {
    fn normal(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Normal,
        }
    }

    fn info(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Info,
        }
    }

    fn success(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Success,
        }
    }

    fn api(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Api,
        }
    }

    fn token(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Token,
        }
    }

    fn tool_call(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::ToolCall,
        }
    }

    fn tool_result(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::ToolResult,
        }
    }

    fn action(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Action,
        }
    }

    fn search(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Search,
        }
    }

    fn warning(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Warning,
        }
    }

    fn error(text: String) -> Self {
        Self {
            text,
            kind: ActivityLineKind::Error,
        }
    }
}

pub(super) fn format_activity(activity: &AgentActivity) -> String {
    match activity {
        AgentActivity::ToolExecution { tool, status, .. } => format!("{tool}: {status}"),
        AgentActivity::Idle => "Idle".into(),
        AgentActivity::Reasoning { wave } => format!("Wave {wave}: reasoning"),
        AgentActivity::WaveContent {
            wave,
            kind,
            content,
            truncated,
        } => {
            let suffix = if *truncated { " [truncated]" } else { "" };
            format!(
                "Wave {wave} {}: {}{suffix}",
                wave_content_label(*kind),
                preview(content, STATUS_PREVIEW_CHARS)
            )
        }
        AgentActivity::WaveDecision {
            wave,
            stop_reason,
            tools,
        } => wave_decision_text(*wave, stop_reason.as_deref(), tools),
        AgentActivity::ApiRequest { provider, model } => format!("API -> {provider} ({model})"),
        AgentActivity::ApiRetry {
            provider,
            model,
            attempt,
            max_attempts,
            delay_ms,
            error,
        } => format!(
            "Reconnecting... {attempt}/{max_attempts} API -> {provider} ({model}) in {delay_ms}ms · {}",
            one_line(error)
        ),
        AgentActivity::ApiResponse {
            input_tokens,
            output_tokens,
            duration_ms,
        } => format!("API <- {input_tokens}->{output_tokens} tokens · {duration_ms}ms"),
        AgentActivity::OutputWarning {
            output_tokens,
            max_tokens,
            stop_reason,
            level,
        } => format!(
            "Output {level}: {output_tokens}/{max_tokens} tokens, stop_reason={}",
            stop_reason.as_deref().unwrap_or("unknown")
        ),
        AgentActivity::TokenBreakdown {
            wave,
            total_estimated,
            context_usage_pct,
            message_count,
            ..
        } => format!(
            "Wave {wave}: {total_estimated} estimated tokens, {message_count} messages, {context_usage_pct:.1}% used"
        ),
        AgentActivity::ToolCall {
            wave,
            tool,
            input_summary,
        } => format!("Wave {wave}: tool {tool}({})", one_line(input_summary)),
        AgentActivity::ToolResult {
            tool,
            is_error,
            output_summary,
        } => {
            let mark = if *is_error { "ERR" } else { "OK" };
            format!("{mark} {tool}: {}", one_line(output_summary))
        }
        AgentActivity::ActionApplied { action } => format!("Action: {}", one_line(action)),
        AgentActivity::BudgetWarning { used, limit, level } => {
            format!("Budget {level}: {used}/{limit}")
        }
        AgentActivity::ShardProgress {
            paradigm,
            completed,
            total,
        } => format!("Shard {paradigm}: {completed}/{total}"),
        AgentActivity::Done {
            iterations,
            actions,
        } => format!("Done: {iterations} waves, {actions} actions"),
        AgentActivity::Error { source, message } => format!("Error {source}: {message}"),
        AgentActivity::WebSearch { query, provider } => {
            format!("WebSearch {provider}: {}", one_line(query))
        }
        AgentActivity::WebSearchDone {
            query,
            provider,
            result_count,
            duration_ms,
        } => format!(
            "WebSearch {provider}: {result_count} results · {duration_ms}ms · {}",
            one_line(query)
        ),
        AgentActivity::WikiSearch { query, lang } => {
            format!("WikiSearch {lang}: {}", one_line(query))
        }
        AgentActivity::WikiSearchDone {
            query,
            lang,
            result_count,
            duration_ms,
        } => format!(
            "WikiSearch {lang}: {result_count} results · {duration_ms}ms · {}",
            one_line(query)
        ),
    }
}

fn wave_content_label(kind: WaveContentKind) -> &'static str {
    match kind {
        WaveContentKind::Thinking => "returned thinking",
        WaveContentKind::Text => "intermediate response",
    }
}

fn wave_decision_text(wave: usize, stop_reason: Option<&str>, tools: &[String]) -> String {
    let tools = if tools.is_empty() {
        "none".to_string()
    } else {
        tools.join(", ")
    };
    format!(
        "Wave {wave} decision: stop={} · tools={tools}",
        stop_reason.unwrap_or("unknown")
    )
}

fn preview(text: &str, max_chars: usize) -> String {
    truncate_chars(&one_line(text), max_chars)
}

fn one_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' | '\r' | '\t' => line.push(' '),
            ch if ch.is_control() => {}
            ch => line.push(ch),
        }
    }
    line
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut truncated: String = text.chars().take(max_chars - 3).collect();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_state_assigns_specific_kinds_for_tool_and_api_events() {
        let mut state = ActivityState::default();

        let api = state
            .apply(AgentActivity::ApiRequest {
                provider: "MiniMax".into(),
                model: "MiniMax-M3".into(),
            })
            .expect("api line");
        assert_eq!(api.kind, ActivityLineKind::Api);

        let tool = state
            .apply(AgentActivity::ToolCall {
                wave: 2,
                tool: "get_world_overview".into(),
                input_summary: "{}".into(),
            })
            .expect("tool line");
        assert_eq!(tool.kind, ActivityLineKind::ToolCall);

        let action = state
            .apply(AgentActivity::ActionApplied {
                action: "SpawnEntity".into(),
            })
            .expect("action line");
        assert_eq!(action.kind, ActivityLineKind::Action);
    }

    #[test]
    fn activity_state_exposes_returned_wave_content_and_decision() {
        let mut state = ActivityState::default();

        let thinking = state
            .apply(AgentActivity::WaveContent {
                wave: 4,
                kind: WaveContentKind::Thinking,
                content: "先检查 schema\n再决定是否写入".into(),
                truncated: false,
            })
            .expect("thinking line");
        assert_eq!(thinking.kind, ActivityLineKind::Info);
        assert!(thinking.text.contains("先检查 schema 再决定是否写入"));

        let decision = state
            .apply(AgentActivity::WaveDecision {
                wave: 4,
                stop_reason: Some("tool_use".into()),
                tools: vec!["query_entities".into(), "modify_component".into()],
            })
            .expect("decision line");
        assert_eq!(decision.kind, ActivityLineKind::Token);
        assert!(decision.text.contains("stop=tool_use"));
        assert!(decision.text.contains("query_entities, modify_component"));
    }

    #[test]
    fn wave_content_preview_is_bounded_and_removes_terminal_controls() {
        let rendered = preview("line 1\n\x1b[31mline 2", 12);

        assert_eq!(rendered.chars().count(), 12);
        assert!(!rendered.contains('\x1b'));
    }
}
