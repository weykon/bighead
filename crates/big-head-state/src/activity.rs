//! Agent activity states — emitted by the runtime at key lifecycle points.

use serde::{Deserialize, Serialize};

/// API 在一个 wave 中明确返回、可安全投影给观察者的内容类型。
///
/// 这里只表示 provider 响应里真实存在的 content block，不代表模型内部未返回的
/// 隐藏推理过程。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveContentKind {
    Thinking,
    Text,
}

/// Observable agent state transitions.
///
/// Each variant represents a discrete event in the agent loop.
/// Consumers (e.g. [`TerminalRenderer`]) subscribe and render these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentActivity {
    ToolExecution {
        trace_id: String,
        tx_id: String,
        tool_call_id: String,
        tool: String,
        status: String,
        elapsed_ms: Option<u64>,
    },
    /// Agent is idle, waiting for work.
    Idle,
    /// Agent is reasoning (multi-turn tool-use loop wave).
    Reasoning { wave: usize },
    /// Content explicitly returned by the API for the current wave.
    WaveContent {
        wave: usize,
        kind: WaveContentKind,
        content: String,
        truncated: bool,
    },
    /// Why the current API response will continue, call tools, or finish.
    WaveDecision {
        wave: usize,
        stop_reason: Option<String>,
        tools: Vec<String>,
    },
    /// About to send an API request.
    ApiRequest { provider: String, model: String },
    /// API request failed with a transient error and will be retried.
    ApiRetry {
        provider: String,
        model: String,
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error: String,
    },
    /// Received an API response.
    ApiResponse {
        input_tokens: usize,
        output_tokens: usize,
        duration_ms: u64,
    },
    /// API response likely hit the configured output token ceiling.
    OutputWarning {
        output_tokens: usize,
        max_tokens: usize,
        stop_reason: Option<String>,
        level: String, // "warning" or "truncated"
    },
    /// Detailed token breakdown for an API request (emitted before ApiRequest).
    /// Shows composition of context window: system prompt, messages, tool definitions.
    TokenBreakdown {
        wave: usize,
        system_prompt_tokens: usize,
        initial_context_tokens: usize, // messages[0]: Spark + world snapshot
        conversation_tokens: usize,    // messages[1..]: assistant + tool_result history
        tool_definitions_tokens: usize,
        total_estimated: usize,
        message_count: usize,
        context_usage_pct: f32, // used / limit * 100
    },
    /// About to execute a tool.
    ToolCall {
        wave: usize,
        tool: String,
        input_summary: String,
    },
    /// Tool execution completed.
    ToolResult {
        tool: String,
        is_error: bool,
        output_summary: String,
    },
    /// An action was applied to the ECS world.
    ActionApplied { action: String },
    /// Token budget warning or exceeded.
    BudgetWarning {
        used: usize,
        limit: usize,
        level: String, // "warning" or "exceeded"
    },
    /// Shard map phase progress.
    ShardProgress {
        paradigm: String,
        completed: u32,
        total: u32,
    },
    /// Agent loop finished.
    Done { iterations: usize, actions: usize },
    /// An error occurred.
    Error { source: String, message: String },
    /// Web search started — query and provider.
    WebSearch { query: String, provider: String },
    /// Web search completed — result count and provider.
    WebSearchDone {
        query: String,
        provider: String,
        result_count: usize,
        duration_ms: u64,
    },
    /// Wikipedia search started — query and language (en/zh).
    WikiSearch { query: String, lang: String },
    /// Wikipedia search completed.
    WikiSearchDone {
        query: String,
        lang: String,
        result_count: usize,
        duration_ms: u64,
    },
}
