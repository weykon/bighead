//! [`Agent`] —— 围绕 [`ai_api_provider::ApiClient`] 的薄封装，
//! 运行多轮 Tool Use 循环。
//!
//! 核心只有一个方法：[`Agent::reason_with_tools`]。
//! 它拿到初始上下文后，反复调用 LLM → 解析 tool_use → 执行工具 → 回传 tool_result，
//! 直到 LLM 不再要求调用工具（返回纯文本），或触及迭代/预算上限。

use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use ai_api_provider::{
    AnthropicResponse, ApiClient, ApiConfig, ApiError, ApiProvider, BudgetStatus, ContentBlock,
    StructuredMessage, TokenBudget, ToolDefinition, resolve_api_key,
};

use big_head_state::{AgentActivity, StateHook, WaveContentKind};

use crate::log::{LogEmitter, LogLevel, emit as emit_log};
use crate::parsing::AgentError;
use crate::tool_definitions::ecs_tool_definitions;
use crate::tool_protocol::{
    ContextProjectionRequest, ContextProjectionResult, ReasoningResult, ToolCall, ToolExecutor,
};

const OUTBOUND_BLOCK_BYTE_THRESHOLD: usize = 32 * 1024;
const OUTBOUND_BLOCK_TOKEN_THRESHOLD: usize = 8_000;
const OUTBOUND_FINAL_BUDGET_NUMERATOR: usize = 9;
const OUTBOUND_FINAL_BUDGET_DENOMINATOR: usize = 10;
const ROLLING_HISTORY_BUDGET_NUMERATOR: usize = 4;
const ROLLING_HISTORY_BUDGET_DENOMINATOR: usize = 10;
const ROLLING_HISTORY_KEEP_RECENT_MESSAGES: usize = 10;
const ROLLING_HISTORY_MIN_COMPACT_MESSAGES: usize = 8;
const ROLLING_HISTORY_MESSAGE_COUNT_TRIGGER: usize = 60;
const API_RECONNECT_MAX_RETRIES: u32 = 5;
const API_RECONNECT_BASE_DELAY_MS: u64 = 1_000;
const API_RECONNECT_MAX_DELAY_MS: u64 = 8_000;
const TOOL_LOOP_MAX_TOTAL_ACTIONS: usize = 64;
const TOOL_LOOP_MAX_CONSECUTIVE_ERROR_WAVES: usize = 4;
const TOOL_LOOP_MAX_CONSECUTIVE_NON_MUTATING_WAVES: usize = 12;
const WAVE_THINKING_DISPLAY_MAX_CHARS: usize = 8_000;
const WAVE_TEXT_DISPLAY_MAX_CHARS: usize = 4_000;
const WAVE_CONTENT_MAX_BLOCKS: usize = 8;

#[derive(Debug, Clone, Copy)]
enum ToolExposure<'a> {
    None,
    All,
    Allow(&'a [String]),
}

impl ToolExposure<'_> {
    fn select(self, tools: impl IntoIterator<Item = ToolDefinition>) -> Vec<ToolDefinition> {
        match self {
            Self::None => Vec::new(),
            Self::All => tools.into_iter().collect(),
            Self::Allow(allowed) => tools
                .into_iter()
                .filter(|tool| allowed.iter().any(|name| name == &tool.name))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolLoopGuardDecision {
    Converge(String),
    ForceFinal(String),
}

struct PreparedOutboundRequest {
    system_prompt: String,
    tools: Vec<ToolDefinition>,
    messages: Vec<StructuredMessage>,
    projected_blocks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputLimitWarning {
    output_tokens: usize,
    max_tokens: usize,
    stop_reason: Option<String>,
    level: &'static str,
}

/// 通过 Anthropic Tool Use 协议推理 ECS 世界的 AI agent。
///
/// `client` 包裹在 [`OnceLock`] 中，底层的 `tokio::spawn` 驱动的
/// worker 仅首次使用时构造。
pub struct Agent {
    client: OnceLock<ApiClient>,
    config: ApiConfig,
    log_emitter: Option<LogEmitter>,
    state_hook: Option<StateHook>,
}

impl Agent {
    pub fn new(provider: ApiProvider, api_key: String) -> Self {
        let config = ApiConfig::new(provider, api_key);
        Self {
            client: OnceLock::new(),
            config,
            log_emitter: None,
            state_hook: None,
        }
    }

    /// Explicit endpoint configuration, also used by deterministic local replay servers.
    pub fn from_api_config(config: ApiConfig) -> Self {
        Self { client: OnceLock::new(), config, log_emitter: None, state_hook: None }
    }

    /// 返回懒初始化的 API client。首次调用会触发 `tokio::spawn`，
    /// 因此调用方必须已在 Tokio runtime 上下文中。
    fn ensure_client(&self) -> &ApiClient {
        self.client.get_or_init(ApiClient::new)
    }

    /// 绑定日志发射器。所有 agent 诊断输出将通过 `e` 路由，
    /// 而非 stderr。
    pub fn set_log_emitter(&mut self, e: LogEmitter) {
        self.log_emitter = Some(e);
    }

    /// [`set_log_emitter`] 的 builder 风格变体。
    pub fn with_log_emitter(mut self, e: LogEmitter) -> Self {
        self.log_emitter = Some(e);
        self
    }

    /// 绑定状态钩子。所有 agent 活动事件将通过 `h` 广播。
    pub fn set_state_hook(&mut self, h: StateHook) {
        self.state_hook = Some(h);
    }

    /// [`set_state_hook`] 的 builder 风格变体。
    pub fn with_state_hook(mut self, h: StateHook) -> Self {
        self.state_hook = Some(h);
        self
    }

    /// 使用给定 provider 创建新 agent，从环境变量解析 API key。
    pub fn from_env(provider: ApiProvider) -> Option<Self> {
        let key = resolve_api_key(provider)?;
        Some(Self::new(provider, key))
    }

    /// 只读访问此 agent 的 API 配置。供卫星子系统
    ///（如 memory compaction）通过相同的 provider/key/model 发送请求。
    pub fn api_config(&self) -> &ApiConfig {
        &self.config
    }

    /// 克隆底层 API client（廉价 —— 克隆共享串行队列）。
    pub fn api_client(&self) -> ApiClient {
        self.ensure_client().clone()
    }

    /// Independent request queue and no foreground UI events for read-only background work.
    pub fn background_copy(&self) -> Self {
        Self {
            client: OnceLock::new(),
            config: self.config.clone(),
            log_emitter: None,
            state_hook: None,
        }
    }

    /// One bounded, tool-free request; retry scheduling belongs to the background caller.
    pub async fn observe_without_tools(
        &self,
        prompt: &str,
        context: String,
    ) -> Result<String, String> {
        let messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: context,
        }])];
        let prepared = self
            .prepare_tool_free_request(prompt, &messages)
            .map_err(|error| error.to_string())?;
        let response = self
            .ensure_client()
            .chat_with_tools(
                &self.config,
                &prepared.messages,
                &[],
                Some(&prepared.system_prompt),
            )
            .await
            .map_err(|error| error.to_string())?;
        if !matches!(
            response.stop_reason.as_deref(),
            Some("end_turn" | "stop_sequence")
        ) || response
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
        {
            return Err("Background observation did not complete a tool-free response".into());
        }
        collect_final_text(&response.content)
            .ok_or_else(|| "Background observation returned no text".into())
    }

    fn prepare_tool_free_request(
        &self,
        system_prompt: &str,
        messages: &[StructuredMessage],
    ) -> Result<PreparedOutboundRequest, AgentError> {
        // A tool-free stage cannot recover evidence replaced by a shard reference.
        let tokens = estimate_outbound_tokens(system_prompt, &[], messages);
        let limit = self
            .config
            .max_input_tokens
            .unwrap_or_else(|| TokenBudget::for_model(&self.config.model).limit());
        let target = limit * OUTBOUND_FINAL_BUDGET_NUMERATOR / OUTBOUND_FINAL_BUDGET_DENOMINATOR;
        if tokens > target {
            return Err(AgentError::ContextBudget(format!(
                "tool-free context exceeds budget: estimated {tokens}/{limit} tokens, target <= {target}; evidence was not projected"
            )));
        }
        Ok(PreparedOutboundRequest {
            system_prompt: system_prompt.into(),
            tools: Vec::new(),
            messages: messages.to_vec(),
            projected_blocks: 0,
        })
    }

    fn prepare_outbound_request(
        &self,
        system_prompt: &str,
        tools: &[ToolDefinition],
        messages: &[StructuredMessage],
        executor: &mut dyn ToolExecutor,
    ) -> Result<PreparedOutboundRequest, AgentError> {
        let mut projected_blocks = 0usize;

        let mut prepared_system_prompt = system_prompt.to_string();
        if let Some(notice) = self.project_if_large(
            executor,
            "system_prompt",
            "system_prompt",
            &projection_stable_id("system_prompt", system_prompt),
            system_prompt,
        )? {
            prepared_system_prompt = notice;
            projected_blocks += 1;
        }

        let mut prepared_tools = tools.to_vec();
        for (idx, tool) in prepared_tools.iter_mut().enumerate() {
            if let Some(notice) = self.project_if_large(
                executor,
                "tool_description",
                &format!("tool:{}:description", tool.name),
                &projection_stable_id(
                    &format!("tool:{idx}:{}:description", tool.name),
                    &tool.description,
                ),
                &tool.description,
            )? {
                tool.description = notice;
                projected_blocks += 1;
            }

            let schema_text = tool.input_schema.to_string();
            if let Some(notice) = self.project_if_large(
                executor,
                "tool_schema",
                &format!("tool:{}:input_schema", tool.name),
                &projection_stable_id(
                    &format!("tool:{idx}:{}:input_schema", tool.name),
                    &schema_text,
                ),
                &schema_text,
            )? {
                tool.input_schema = serde_json::json!({
                    "type": "object",
                    "additionalProperties": true,
                    "description": notice,
                });
                projected_blocks += 1;
            }
        }

        let mut prepared_messages = messages.to_vec();
        for (message_index, message) in prepared_messages.iter_mut().enumerate() {
            for (block_index, block) in message.content.iter_mut().enumerate() {
                match block {
                    ContentBlock::Text { text } => {
                        if let Some(notice) = self.project_if_large(
                            executor,
                            "message_text",
                            &format!("message:{message_index}:text:{block_index}"),
                            &projection_stable_id(
                                &format!("message:{message_index}:block:{block_index}:text"),
                                text,
                            ),
                            text,
                        )? {
                            *text = notice;
                            projected_blocks += 1;
                        }
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        if let Some(notice) = self.project_if_large(
                            executor,
                            "tool_result",
                            &format!("message:{message_index}:tool_result:{tool_use_id}"),
                            &projection_stable_id(
                                &format!(
                                    "message:{message_index}:block:{block_index}:tool_result:{tool_use_id}"
                                ),
                                content,
                            ),
                            content,
                        )? {
                            *content = notice;
                            projected_blocks += 1;
                        }
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        let input_text = input.to_string();
                        if let Some(notice) = self.project_if_large(
                            executor,
                            "tool_use_input",
                            &format!("message:{message_index}:tool_use:{name}:input"),
                            &projection_stable_id(
                                &format!(
                                    "message:{message_index}:block:{block_index}:tool_use:{name}:input"
                                ),
                                &input_text,
                            ),
                            &input_text,
                        )? {
                            *input = serde_json::json!({
                                "_projected_context_notice": notice,
                            });
                            projected_blocks += 1;
                        }
                    }
                    ContentBlock::Thinking { thinking } => {
                        if let Some(notice) = self.project_if_large(
                            executor,
                            "thinking",
                            &format!("message:{message_index}:thinking:{block_index}"),
                            &projection_stable_id(
                                &format!("message:{message_index}:block:{block_index}:thinking"),
                                thinking,
                            ),
                            thinking,
                        )? {
                            *thinking = notice;
                            projected_blocks += 1;
                        }
                    }
                    ContentBlock::Image { .. } => {}
                    ContentBlock::Unknown(value) => {
                        let value_text = value.to_string();
                        if let Some(notice) = self.project_if_large(
                            executor,
                            "unknown_block",
                            &format!("message:{message_index}:unknown:{block_index}"),
                            &projection_stable_id(
                                &format!("message:{message_index}:block:{block_index}:unknown"),
                                &value_text,
                            ),
                            &value_text,
                        )? {
                            *block = ContentBlock::Text { text: notice };
                            projected_blocks += 1;
                        }
                    }
                }
            }
        }

        let mut final_tokens =
            estimate_outbound_tokens(&prepared_system_prompt, &prepared_tools, &prepared_messages);
        let limit = match self.config.max_input_tokens {
            Some(limit) => limit,
            None => TokenBudget::for_model(&self.config.model).limit(),
        };
        let target = limit * OUTBOUND_FINAL_BUDGET_NUMERATOR / OUTBOUND_FINAL_BUDGET_DENOMINATOR;
        // Many individually small results can still exhaust the request budget.
        // Keep protocol envelopes and task text intact in this second pass.
        if final_tokens > target {
            let mut candidates = Vec::new();
            for (mi, message) in prepared_messages.iter().enumerate() {
                for (bi, block) in message.content.iter().enumerate() {
                    if let ContentBlock::ToolResult { content, .. } = block {
                        if content.len() > 2048
                            && !content.starts_with("[Context Projection Notice]")
                            && !content.starts_with("[Context Artifact]")
                        {
                            candidates.push((content.len(), mi, bi));
                        }
                    }
                }
            }
            candidates.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, mi, bi) in candidates {
                if final_tokens <= target {
                    break;
                }
                if let ContentBlock::ToolResult { content, .. } =
                    &mut prepared_messages[mi].content[bi]
                {
                    let projected = executor
                        .project_context_content(ContextProjectionRequest {
                            source_kind: "tool_result_budget".into(),
                            source_label: format!("message:{mi}:block:{bi}"),
                            stable_id: projection_stable_id(&format!("budget:{mi}:{bi}"), content),
                            content: content.clone(),
                        })
                        .map_err(|e| {
                            AgentError::ContextProjection(format!("project aggregate context: {e}"))
                        })?;
                    *content = format!(
                        "[Context Artifact] Untrusted tool output stored in ECS. Use get_shard_content with paradigm={}, shard_index=0..{}, offset=0, limit=2000. Original bytes={}. Not a completed-action summary.",
                        projected.paradigm,
                        projected.total_shards.saturating_sub(1),
                        projected.original_bytes
                    );
                    projected_blocks += 1;
                }
                final_tokens = estimate_outbound_tokens(
                    &prepared_system_prompt,
                    &prepared_tools,
                    &prepared_messages,
                );
            }
        }
        if final_tokens > target {
            return Err(AgentError::ContextBudget(format!(
                "outbound context remains too large after projection: estimated {final_tokens}/{limit} tokens, target <= {target}"
            )));
        }

        Ok(PreparedOutboundRequest {
            system_prompt: prepared_system_prompt,
            tools: prepared_tools,
            messages: prepared_messages,
            projected_blocks,
        })
    }

    fn project_if_large(
        &self,
        executor: &mut dyn ToolExecutor,
        source_kind: &str,
        source_label: &str,
        stable_id: &str,
        content: &str,
    ) -> Result<Option<String>, AgentError> {
        let bytes = content.len();
        let tokens = ai_api_provider::tokens::estimate_tokens(content);
        if bytes <= OUTBOUND_BLOCK_BYTE_THRESHOLD && tokens <= OUTBOUND_BLOCK_TOKEN_THRESHOLD {
            return Ok(None);
        }

        let result = executor
            .project_context_content(ContextProjectionRequest {
                source_kind: source_kind.to_string(),
                source_label: source_label.to_string(),
                stable_id: stable_id.to_string(),
                content: content.to_string(),
            })
            .map_err(|e| AgentError::ContextProjection(format!("project outbound context: {e}")))?;

        Ok(Some(render_projection_notice(
            source_kind,
            source_label,
            bytes,
            tokens,
            &result,
        )))
    }

    fn maybe_project_rolling_history(
        &self,
        executor: &mut dyn ToolExecutor,
        messages: &mut Vec<StructuredMessage>,
        limit: usize,
        wave: usize,
    ) -> Result<Option<ContextProjectionResult>, AgentError> {
        if messages.len()
            <= 1 + ROLLING_HISTORY_KEEP_RECENT_MESSAGES + ROLLING_HISTORY_MIN_COMPACT_MESSAGES
        {
            return Ok(None);
        }

        let conversation_tokens: usize = messages.iter().skip(1).map(count_message_tokens).sum();
        let token_trigger =
            limit * ROLLING_HISTORY_BUDGET_NUMERATOR / ROLLING_HISTORY_BUDGET_DENOMINATOR;
        if conversation_tokens < token_trigger
            && messages.len() < ROLLING_HISTORY_MESSAGE_COUNT_TRIGGER
        {
            return Ok(None);
        }

        let compact_start = 1;
        let mut compact_count = messages.len() - 1 - ROLLING_HISTORY_KEEP_RECENT_MESSAGES;
        compact_count -= compact_count % 2;
        while compact_count >= ROLLING_HISTORY_MIN_COMPACT_MESSAGES
            && !is_rolling_history_boundary_safe(messages, compact_start + compact_count)
        {
            compact_count -= 1;
        }
        if compact_count < ROLLING_HISTORY_MIN_COMPACT_MESSAGES {
            return Ok(None);
        }

        let compact_end = compact_start + compact_count;
        let compacted = messages[compact_start..compact_end].to_vec();
        let content =
            render_history_projection_content(wave, compact_start, compact_end, &compacted);
        let stable_id = projection_stable_id(
            &format!("history_window:{compact_start}:{compact_end}:wave:{wave}"),
            &content,
        );
        let result = executor
            .project_context_content(ContextProjectionRequest {
                source_kind: "history_window".into(),
                source_label: format!(
                    "messages[{compact_start}..{compact_end}] before wave {wave}"
                ),
                stable_id,
                content,
            })
            .map_err(|e| AgentError::ContextProjection(format!("project rolling history: {e}")))?;

        let notice = render_history_projection_notice(
            wave,
            compact_start,
            compact_end,
            compact_count,
            conversation_tokens,
            &result,
        );
        messages.splice(
            compact_start..compact_end,
            [StructuredMessage::user(vec![ContentBlock::Text {
                text: notice,
            }])],
        );

        Ok(Some(result))
    }

    async fn chat_with_tools_reconnecting(
        &self,
        messages: &[StructuredMessage],
        tools: &[ToolDefinition],
        system_prompt: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<AnthropicResponse, ApiError> {
        for retry in 0..=API_RECONNECT_MAX_RETRIES {
            let result = self
                .ensure_client()
                .chat_with_tools(&self.config, messages, tools, Some(system_prompt))
                .await;

            match result {
                Ok(response) => return Ok(response),
                Err(error)
                    if retry < API_RECONNECT_MAX_RETRIES && is_retryable_api_error(&error) =>
                {
                    let attempt = retry + 1;
                    let delay_ms = api_reconnect_delay_ms(attempt);
                    if let Some(ref h) = self.state_hook {
                        h.emit(AgentActivity::ApiRetry {
                            provider: format!("{:?}", self.config.provider),
                            model: self.config.model.clone(),
                            attempt,
                            max_attempts: API_RECONNECT_MAX_RETRIES,
                            delay_ms,
                            error: error.to_string(),
                        });
                    }
                    emit_log(
                        &self.log_emitter,
                        "coordinator",
                        LogLevel::Warn,
                        format!(
                            "API transient failure; reconnecting {attempt}/{API_RECONNECT_MAX_RETRIES} in {delay_ms}ms: {error}"
                        ),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    if executor.is_cancelled() {
                        return Err(ApiError::Parse(
                            "api retry cancelled by runtime".to_string(),
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        }

        unreachable!("retry loop always returns on success or final error")
    }

    /// 运行多轮 Tool Use 推理循环 —— 这是整个 agent 的核心。
    ///
    /// 每轮做的事情：
    /// 1. 检查迭代上限和 token 预算
    /// 2. 把消息历史 + 工具定义发给 LLM
    /// 3. 解析响应：提取 tool_use block
    /// 4. 如果 LLM 返回纯文本（不再要工具）→ 结束
    /// 5. 否则执行工具，把 tool_result 追加到消息历史 → 回到步骤 1
    ///
    /// 终止条件有三个：
    /// - LLM 自己决定停（返回 stop=end_turn 的文本）
    /// - 到达 `max_iterations`
    /// - 输入 token 预算超限
    pub async fn reason_with_tools(
        &mut self,
        initial_context: &str,
        executor: &mut dyn ToolExecutor,
        max_iterations: usize,
        system_prompt: &str,
    ) -> Result<ReasoningResult, AgentError> {
        self.reason_with_content_blocks(
            vec![ContentBlock::Text {
                text: initial_context.to_string(),
            }],
            executor,
            max_iterations,
            system_prompt,
        )
        .await
    }

    pub async fn reason_with_content_blocks(
        &mut self,
        initial_content: Vec<ContentBlock>,
        executor: &mut dyn ToolExecutor,
        max_iterations: usize,
        system_prompt: &str,
    ) -> Result<ReasoningResult, AgentError> {
        self.reason_with_content_blocks_inner(
            initial_content,
            executor,
            max_iterations,
            system_prompt,
            ToolExposure::All,
        )
        .await
    }

    /// Run a tool loop with an explicit capability allowlist.
    ///
    /// Pipeline agents use this entrypoint so their role is enforced by the
    /// API-visible tool surface, rather than relying on prompt compliance.
    pub async fn reason_with_restricted_tools(
        &mut self,
        initial_content: Vec<ContentBlock>,
        executor: &mut dyn ToolExecutor,
        max_iterations: usize,
        system_prompt: &str,
        allowed_tools: &[String],
    ) -> Result<ReasoningResult, AgentError> {
        self.reason_with_content_blocks_inner(
            initial_content,
            executor,
            max_iterations,
            system_prompt,
            ToolExposure::Allow(allowed_tools),
        )
        .await
    }

    pub async fn reason_without_tools(
        &mut self,
        initial_content: Vec<ContentBlock>,
        executor: &mut dyn ToolExecutor,
        max_iterations: usize,
        system_prompt: &str,
    ) -> Result<ReasoningResult, AgentError> {
        self.reason_with_content_blocks_inner(
            initial_content,
            executor,
            max_iterations,
            system_prompt,
            ToolExposure::None,
        )
        .await
    }

    async fn reason_with_content_blocks_inner(
        &mut self,
        initial_content: Vec<ContentBlock>,
        executor: &mut dyn ToolExecutor,
        max_iterations: usize,
        system_prompt: &str,
        tool_exposure: ToolExposure<'_>,
    ) -> Result<ReasoningResult, AgentError> {
        // 消息历史：首条是 user 消息，包含初始上下文（世界快照 + spark）
        let mut messages: Vec<StructuredMessage> = vec![StructuredMessage::user(initial_content)];
        let mut iterations = 0;

        // 根据模型创建 token 预算（优先用显式配置，否则查模型上限表）
        let mut budget = match self.config.max_input_tokens {
            Some(limit) => TokenBudget::new(limit),
            None => TokenBudget::for_model(&self.config.model),
        };

        let mut total_actions = 0usize;
        let mut nudged_unfulfilled_tool_commitment = false;
        let mut consecutive_error_tool_waves = 0usize;
        let mut consecutive_non_mutating_tool_waves = 0usize;
        let mut convergence_feedback_sent = false;
        let mut force_final_response = false;
        let mut final_validation_feedback_sent = false;

        loop {
            iterations += 1;

            // Rebuild after each tool wave so live registration/removal is visible
            // on the next request, while preserving the stage's capability filter.
            let tools = tool_exposure.select(
                ecs_tool_definitions()
                    .into_iter()
                    .chain(executor.dynamic_tool_definitions()),
            );

            if let Some(ref h) = self.state_hook {
                h.emit(AgentActivity::Reasoning { wave: iterations });
            }

            // [终止条件 1] 达到最大迭代轮数
            // A guard may reserve one tool-free turn so the model can explain
            // what it learned or changed. Do not let the normal iteration
            // boundary swallow that user-facing response.
            let effective_iteration_limit = max_iterations
                .saturating_add(usize::from(convergence_feedback_sent))
                .saturating_add(usize::from(final_validation_feedback_sent))
                .saturating_add(usize::from(force_final_response));
            if iterations > effective_iteration_limit {
                if let Some(ref h) = self.state_hook {
                    h.emit(AgentActivity::Error {
                        source: "agent".into(),
                        message: format!("Max iterations ({}) reached", max_iterations),
                    });
                }
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Warn,
                    format!("Hit max iterations ({})", max_iterations),
                );
                return Ok(ReasoningResult {
                    termination: crate::ReasoningTermination::IterationLimit,
                    final_text: Some(format!(
                        "(stopped: reached {} iteration limit)",
                        max_iterations
                    )),
                    iterations,
                    total_actions,
                });
            }

            // [终止条件 4] Cooperative cancellation — runtime asks us to
            // bail out (e.g. user pressed Ctrl+C). Break before the next
            // API call so we don't waste tokens.
            if executor.is_cancelled() {
                if let Some(ref h) = self.state_hook {
                    h.emit(AgentActivity::Error {
                        source: "agent".into(),
                        message: "Cancelled by runtime (Ctrl+C or similar)".into(),
                    });
                }
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Warn,
                    "Reasoning loop cancelled by runtime".to_string(),
                );
                return Ok(ReasoningResult {
                    final_text: Some("(cancelled)".to_string()),
                    termination: crate::ReasoningTermination::Cancelled,
                    iterations,
                    total_actions,
                });
            }

            let limit = match self.config.max_input_tokens {
                Some(limit) => limit,
                None => TokenBudget::for_model(&self.config.model).limit(),
            };
            if let Some(result) =
                self.maybe_project_rolling_history(executor, &mut messages, limit, iterations)?
            {
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Info,
                    format!(
                        "Projected rolling history into ECS before wave {}: {} ({} shard(s), {} bytes)",
                        iterations, result.paradigm, result.total_shards, result.original_bytes
                    ),
                );
            }

            let request_tools = if force_final_response || final_validation_feedback_sent {
                &[][..]
            } else {
                tools.as_slice()
            };
            let prepared = if request_tools.is_empty() {
                self.prepare_tool_free_request(system_prompt, &messages)?
            } else {
                self.prepare_outbound_request(system_prompt, request_tools, &messages, executor)?
            };
            if prepared.projected_blocks > 0 {
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Info,
                    format!(
                        "Projected {} outbound context block(s) into ECS before API request",
                        prepared.projected_blocks
                    ),
                );
            }
            messages = prepared.messages.clone();

            // [终止条件 3] 每轮先算一遍当前实际出站上下文总 token 数
            budget.reset();
            let system_prompt_tokens =
                ai_api_provider::tokens::estimate_tokens(&prepared.system_prompt);
            budget.add_tokens(system_prompt_tokens);

            // Calculate detailed breakdown
            let initial_context_tokens = if !messages.is_empty() {
                count_message_tokens(&messages[0])
            } else {
                0
            };
            let conversation_tokens: usize = messages
                .iter()
                .skip(1)
                .map(|m| count_message_tokens(m))
                .sum();

            for msg in &messages {
                budget.add_tokens(count_message_tokens(msg));
            }

            let tool_definitions_tokens = count_tool_definition_tokens(&prepared.tools);
            budget.add_tokens(tool_definitions_tokens);

            let total_estimated = budget.used();
            let limit = budget.limit();
            let context_usage_pct = (total_estimated as f32 / limit as f32 * 100.0).min(100.0);

            // Emit detailed token breakdown
            if let Some(ref h) = self.state_hook {
                h.emit(big_head_state::AgentActivity::TokenBreakdown {
                    wave: iterations,
                    system_prompt_tokens,
                    initial_context_tokens,
                    conversation_tokens,
                    tool_definitions_tokens,
                    total_estimated,
                    message_count: messages.len(),
                    context_usage_pct,
                });
            }

            match budget.status() {
                BudgetStatus::Exceeded { used, limit } => {
                    if let Some(ref h) = self.state_hook {
                        h.emit(AgentActivity::BudgetWarning {
                            used,
                            limit,
                            level: "exceeded".into(),
                        });
                    }
                    let message = format!(
                        "Context window exceeded after outbound projection: {used}/{limit} tokens"
                    );
                    emit_log(
                        &self.log_emitter,
                        "coordinator",
                        LogLevel::Error,
                        message.clone(),
                    );
                    return Err(AgentError::ContextBudget(message));
                }
                BudgetStatus::Warning { used, limit } => {
                    if let Some(ref h) = self.state_hook {
                        h.emit(AgentActivity::BudgetWarning {
                            used,
                            limit,
                            level: "warning".into(),
                        });
                    }
                    emit_log(
                        &self.log_emitter,
                        "coordinator",
                        LogLevel::Warn,
                        format!(
                            "Context window warning: {used}/{limit} used. \
                             Consider shard_content for large results."
                        ),
                    );
                }
                BudgetStatus::Ok => {}
            }

            // 调用 LLM：把消息历史 + 工具定义一起发过去
            if let Some(ref h) = self.state_hook {
                h.emit(AgentActivity::ApiRequest {
                    provider: format!("{:?}", self.config.provider),
                    model: self.config.model.clone(),
                });
            }
            let api_start = std::time::Instant::now();
            let response_result = self
                .chat_with_tools_reconnecting(
                    &messages,
                    &prepared.tools,
                    &prepared.system_prompt,
                    &*executor,
                )
                .await;

            let response = match response_result {
                Ok(resp) => resp,
                Err(e) => {
                    if let Some(ref h) = self.state_hook {
                        h.emit(AgentActivity::Error {
                            source: "api".into(),
                            message: e.to_string(),
                        });
                    }
                    return Err(AgentError::ApiError(e.to_string()));
                }
            };

            let api_duration_ms = api_start.elapsed().as_millis() as u64;
            if let Some(ref h) = self.state_hook {
                let (in_tok, out_tok) = response
                    .usage
                    .as_ref()
                    .map(|u| (u.input_tokens, u.output_tokens))
                    .unwrap_or((0, 0));
                h.emit(AgentActivity::ApiResponse {
                    input_tokens: in_tok,
                    output_tokens: out_tok,
                    duration_ms: api_duration_ms,
                });
            }

            let output_warning = detect_output_limit_warning(&response, self.config.max_tokens);
            if let Some(warning) = output_warning.as_ref() {
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Warn,
                    format!(
                        "API output may be incomplete: output_tokens={}/{} stop_reason={}",
                        warning.output_tokens,
                        warning.max_tokens,
                        warning.stop_reason.as_deref().unwrap_or("unknown")
                    ),
                );
                if let Some(ref h) = self.state_hook {
                    h.emit(AgentActivity::OutputWarning {
                        output_tokens: warning.output_tokens,
                        max_tokens: warning.max_tokens,
                        stop_reason: warning.stop_reason.clone(),
                        level: warning.level.into(),
                    });
                }
            }

            // 用 API 返回的精确 input_tokens 校准后续估算
            if let Some(ref usage) = response.usage {
                let estimated = budget.used();
                let exact = usage.input_tokens;
                let diff_pct = if estimated > 0 {
                    ((exact as f64 - estimated as f64) / estimated as f64 * 100.0) as i32
                } else {
                    0
                };
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Info,
                    format!(
                        "API usage: {exact} input tokens (estimated {estimated}, {diff_pct:+}%)"
                    ),
                );
            }

            // 从 LLM 响应中捞出所有 tool_use block（可能多个 → 并行执行）
            let tool_calls: Vec<ToolCall> = response
                .content
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        Some(ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            if final_validation_feedback_sent && !tool_calls.is_empty() {
                return Err(AgentError::ResultValidation(
                    "Pipeline correction requested tools during the tool-free final turn; no tools executed".into(),
                ));
            }

            // 将 provider 明确返回的 thinking、中间文本和本轮决策投影给观察者。
            // 原始响应仍完整进入 messages；这里的长度限制只约束终端/TUI 显示。
            if let Some(ref h) = self.state_hook {
                for activity in project_wave_activities(&response, iterations) {
                    h.emit(activity);
                }
            }

            // 日志：记录本轮 LLM 要调用哪些工具
            if !tool_calls.is_empty() {
                let names: Vec<String> = tool_calls
                    .iter()
                    .map(|c| {
                        if c.name.starts_with("get_")
                            || matches!(c.name.as_str(), "query_entities" | "query_ecs")
                        {
                            let param = c
                                .input
                                .get("entity_id")
                                .and_then(|v| v.as_u64())
                                .map(|id| format!("({})", id));
                            format!("{}{}", c.name, param.unwrap_or_default())
                        } else {
                            c.name.clone()
                        }
                    })
                    .collect();
                let parallel = if tool_calls.len() > 1 {
                    format!("  [{} sequential]", tool_calls.len())
                } else {
                    String::new()
                };
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Info,
                    format!("Wave {}: {}{}", iterations, names.join(", "), parallel),
                );
            }

            // 把 LLM 的完整响应（含 thinking + tool_use block）追加到消息历史
            messages.push(StructuredMessage::assistant(response.content.clone()));

            let stop = response.stop_reason.as_deref().unwrap_or("");

            // [终止条件 2] LLM 不再要求工具，返回了纯文本 → 循环结束
            if stop != "tool_use" || tool_calls.is_empty() {
                let mut final_text = collect_final_text(&response.content);
                if should_retry_unfulfilled_tool_commitment(
                    final_text.as_deref(),
                    total_actions,
                    !prepared.tools.is_empty(),
                    nudged_unfulfilled_tool_commitment,
                ) && iterations < max_iterations
                {
                    nudged_unfulfilled_tool_commitment = true;
                    emit_log(
                        &self.log_emitter,
                        "coordinator",
                        LogLevel::Warn,
                        "Model promised tool-layer work without a tool call; retrying with a tool-use guard"
                            .to_string(),
                    );
                    messages.push(StructuredMessage::user(vec![ContentBlock::Text {
                        text: render_unfulfilled_tool_commitment_notice(final_text.as_deref()),
                    }]));
                    continue;
                }
                if let Some(warning) = output_warning.as_ref() {
                    final_text = Some(render_output_limit_notice(warning, final_text.as_deref()));
                }
                if let Err(error) =
                    executor.validate_final_response(final_text.as_deref().unwrap_or_default())
                {
                    let preview: String = final_text
                        .as_deref()
                        .unwrap_or_default()
                        .chars()
                        .take(320)
                        .collect();
                    emit_log(
                        &self.log_emitter,
                        "coordinator",
                        LogLevel::Warn,
                        format!(
                            "Final response rejected: {error}; text_bytes={}; text_preview={preview:?}",
                            final_text.as_ref().map_or(0, String::len)
                        ),
                    );
                    if final_validation_feedback_sent {
                        return Err(AgentError::ResultValidation(format!(
                            "Pipeline result rejected: {error}; text_preview={preview:?}"
                        )));
                    }
                    final_validation_feedback_sent = true;
                    messages.push(StructuredMessage::user(vec![ContentBlock::Text {
                        text: format!("Pipeline result rejected: {error}. Use only the evidence already in this conversation. Return the required JSON disposition for every leased event, with no prose or code fences. No tools or additional writes are allowed in this single correction turn. Do not invent event IDs or evidence, or claim unperformed work."),
                    }]));
                    continue;
                }
                emit_log(
                    &self.log_emitter,
                    "coordinator",
                    LogLevel::Info,
                    format!("Done: {} waves", iterations),
                );
                if let Some(ref h) = self.state_hook {
                    h.emit(AgentActivity::Done {
                        iterations,
                        actions: total_actions,
                    });
                }
                return Ok(ReasoningResult {
                    termination: if output_warning.is_some() {
                        crate::ReasoningTermination::OutputLimit
                    } else if force_final_response {
                        crate::ReasoningTermination::Converged
                    } else {
                        crate::ReasoningTermination::Completed
                    },
                    final_text,
                    iterations,
                    total_actions,
                });
            }

            // 执行所有工具调用，每个产生一个 ToolResult block
            executor.set_wave(iterations);
            let mut result_blocks = Vec::new();
            let mut error_results = 0usize;
            let mut has_mutating_tool = false;
            for call in &tool_calls {
                if is_mutating_call(call) {
                    has_mutating_tool = true;
                }
                let result = executor.execute(call).await;
                if result.is_error {
                    error_results += 1;
                }
                result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: result.tool_use_id,
                    content: result.output,
                    is_error: result.is_error,
                });
                if executor.waiting_for_user() {
                    return Ok(ReasoningResult {
                        termination: crate::ReasoningTermination::AwaitingInput,
                        final_text: Some("有一項需要你選擇，已放喺 Questions。選擇或輸入答案後會繼續；未執行本批次餘下操作。".into()),
                        iterations,
                        total_actions: total_actions + result_blocks.len(),
                    });
                }
            }

            total_actions += tool_calls.len();
            if error_results == tool_calls.len() {
                consecutive_error_tool_waves += 1;
            } else {
                consecutive_error_tool_waves = 0;
            }
            if has_mutating_tool {
                consecutive_non_mutating_tool_waves = 0;
            } else {
                consecutive_non_mutating_tool_waves += 1;
            }

            let guard_decision = tool_loop_guard_decision(
                total_actions,
                consecutive_error_tool_waves,
                consecutive_non_mutating_tool_waves,
                convergence_feedback_sent,
            );

            if let Some(decision) = guard_decision.as_ref() {
                let notice = match decision {
                    ToolLoopGuardDecision::Converge(reason) => {
                        render_tool_loop_convergence_notice(reason, total_actions)
                    }
                    ToolLoopGuardDecision::ForceFinal(reason) => {
                        render_tool_loop_final_notice(reason, total_actions)
                    }
                };
                result_blocks.push(ContentBlock::Text {
                    text: notice.clone(),
                });
                emit_log(&self.log_emitter, "coordinator", LogLevel::Warn, notice);
            }

            // 把工具结果作为 user 消息追加，下一轮 LLM 就能看到"我调用了 X，结果是 Y"，
            // 以及 runtime 对这一段行为的收敛反馈。
            messages.push(StructuredMessage::user(result_blocks));

            match guard_decision {
                Some(ToolLoopGuardDecision::Converge(_)) => {
                    convergence_feedback_sent = true;
                    consecutive_non_mutating_tool_waves = 0;
                }
                Some(ToolLoopGuardDecision::ForceFinal(_)) => {
                    force_final_response = true;
                }
                None if convergence_feedback_sent => {
                    // The model chose one more tool wave after seeing the
                    // convergence feedback. Its result is now available, so
                    // the next turn must report back without more tools.
                    let notice = render_tool_loop_final_notice(
                        "The model chose an additional tool wave after runtime convergence feedback.",
                        total_actions,
                    );
                    if let Some(StructuredMessage { content, .. }) = messages.last_mut() {
                        content.push(ContentBlock::Text {
                            text: notice.clone(),
                        });
                    }
                    emit_log(&self.log_emitter, "coordinator", LogLevel::Warn, notice);
                    force_final_response = true;
                }
                None => {}
            }
        }
    }
}

fn tool_loop_guard_decision(
    total_actions: usize,
    consecutive_error_tool_waves: usize,
    consecutive_non_mutating_tool_waves: usize,
    convergence_feedback_sent: bool,
) -> Option<ToolLoopGuardDecision> {
    if total_actions >= TOOL_LOOP_MAX_TOTAL_ACTIONS {
        return Some(ToolLoopGuardDecision::ForceFinal(format!(
            "The tool loop used {total_actions} tool calls and reached the hard action budget of {TOOL_LOOP_MAX_TOTAL_ACTIONS}."
        )));
    }
    if consecutive_error_tool_waves >= TOOL_LOOP_MAX_CONSECUTIVE_ERROR_WAVES {
        return Some(ToolLoopGuardDecision::ForceFinal(format!(
            "The last {consecutive_error_tool_waves} consecutive tool waves all failed."
        )));
    }
    if consecutive_non_mutating_tool_waves >= TOOL_LOOP_MAX_CONSECUTIVE_NON_MUTATING_WAVES {
        let reason = format!(
            "The last {consecutive_non_mutating_tool_waves} consecutive tool waves were read-only and did not mutate ECS, render a view, shard content, or update a system."
        );
        return Some(if convergence_feedback_sent {
            ToolLoopGuardDecision::ForceFinal(reason)
        } else {
            ToolLoopGuardDecision::Converge(reason)
        });
    }
    None
}

fn render_tool_loop_convergence_notice(reason: &str, total_actions: usize) -> String {
    format!(
        "[Runtime Convergence Feedback]\n\
         The runtime is showing you your own recent behavior instead of terminating this cycle.\n\
         Observed: {reason}\n\
         Total tool calls so far: {total_actions}.\n\n\
         The user is still waiting for a useful response. Treat the accumulated tool results as sufficient evidence.\n\
         - If this is an investigation, stop browsing and answer now with findings, evidence, and the next concrete step.\n\
         - If the user requested a change and the target is known, perform at most one focused write/render action now.\n\
         - Do not repeat broad filesystem searches, schema discovery, or the same failed query.\n\
         If you call a tool once more, the runtime will preserve its result and then require a tool-free final response."
    )
}

fn render_tool_loop_final_notice(reason: &str, total_actions: usize) -> String {
    format!(
        "[Runtime Final Response Required]\n\
         Observed: {reason}\n\
         Total tool calls: {total_actions}.\n\n\
         Tool access is disabled for the next turn. Respond directly to the user now.\n\
         Summarize what you found or changed, identify anything incomplete, and state a concrete blocker when one exists.\n\
         Do not promise a later tool layer and do not output this runtime notice as the answer."
    )
}

fn is_mutating_call(call: &ToolCall) -> bool {
    if call.name == "dream_task" {
        return matches!(
            call.input.get("action").and_then(serde_json::Value::as_str),
            Some("create" | "next" | "submit" | "finish" | "rollback")
        );
    }
    is_mutating_tool(&call.name)
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(
        name,
        "spawn_entity"
            | "dream_policy"
            | "define_lens_app"
            | "define_lens_binding"
            | "record_lens_feedback"
            | "propose_lens_candidate"
            | "trial_lens_candidate"
            | "adopt_lens_candidate"
            | "modify_component"
            | "remove_component"
            | "despawn_entity"
            | "define_component"
            | "define_system"
            | "update_system"
            | "remove_system"
            | "run_system"
            | "render_view"
            | "render_document"
            | "request_user_input"
            | "render_mind_view"
            | "render_status"
            | "shard_content"
            | "push_spark"
            | "export_lens"
            | "import_lens"
            | "send_lens"
            | "receive_lens"
    )
}

fn is_rolling_history_boundary_safe(messages: &[StructuredMessage], compact_end: usize) -> bool {
    messages
        .get(compact_end)
        .is_none_or(|message| !message_has_tool_result(message))
}

fn message_has_tool_result(message: &StructuredMessage) -> bool {
    message
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
}

fn collect_final_text(blocks: &[ContentBlock]) -> Option<String> {
    let text: String = blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    (!text.trim().is_empty()).then_some(text)
}

fn should_retry_unfulfilled_tool_commitment(
    final_text: Option<&str>,
    total_actions: usize,
    tools_available: bool,
    already_nudged: bool,
) -> bool {
    if already_nudged || total_actions > 0 || !tools_available {
        return false;
    }
    final_text
        .map(contains_unfulfilled_tool_commitment)
        .unwrap_or(false)
}

fn contains_unfulfilled_tool_commitment(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "转给 tool",
        "轉給 tool",
        "转给tool",
        "轉給tool",
        "给到tool",
        "給到tool",
        "交给工具",
        "交畀工具",
        "我会调用工具",
        "我會調用工具",
        "我将调用工具",
        "i will call tool",
        "i will use tool",
        "去动手",
        "去動手",
        "开始执行",
        "開始執行",
        "转给 tool 层",
        "轉給 tool 層",
        "已更新 sessioncontext",
        "已修改 sessioncontext",
        "已写进 sessioncontext",
        "已寫進 sessioncontext",
        "已写入 ecs",
        "已寫入 ecs",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn render_unfulfilled_tool_commitment_notice(final_text: Option<&str>) -> String {
    let mut notice = String::from(
        "[Runtime Tool Commitment Notice]\n\
You just described handing work to the tool layer, executing an update, or mutating ECS, but no tool call was made.\n\
There is no separate later tool layer: this current loop is the tool layer.\n\
If the user asked to execute, call the appropriate tool now. If you cannot execute, reply explicitly with `未执行` and the reason.\n",
    );
    if let Some(text) = final_text {
        notice.push_str("\nPrevious final text that was not accepted:\n");
        notice.push_str(text);
    }
    notice
}

fn is_retryable_api_error(error: &ApiError) -> bool {
    match error {
        ApiError::Request(err) => {
            err.is_timeout()
                || err.is_connect()
                || err.is_request()
                || err.is_body()
                || err.to_string().to_ascii_lowercase().contains("connection")
        }
        ApiError::ApiResponse { status, .. } => {
            matches!(*status, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
        }
        ApiError::QueueFull { .. } => true,
        ApiError::Parse(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("queue closed")
                || lower.contains("worker dropped")
                || lower.contains("connection")
                || lower.contains("timeout")
        }
        ApiError::NoApiKey { .. } => false,
    }
}

fn api_reconnect_delay_ms(attempt: u32) -> u64 {
    let multiplier = 2_u64.saturating_pow(attempt.saturating_sub(1));
    (API_RECONNECT_BASE_DELAY_MS * multiplier).min(API_RECONNECT_MAX_DELAY_MS)
}

fn project_wave_activities(response: &AnthropicResponse, wave: usize) -> Vec<AgentActivity> {
    let has_tool_use = response
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolUse { .. }));
    let mut activities = Vec::new();
    let mut content_blocks = 0;

    for block in &response.content {
        if content_blocks >= WAVE_CONTENT_MAX_BLOCKS {
            break;
        }

        let projected = match block {
            ContentBlock::Thinking { thinking } => {
                project_wave_content(thinking, WAVE_THINKING_DISPLAY_MAX_CHARS)
                    .map(|(content, truncated)| (WaveContentKind::Thinking, content, truncated))
            }
            // 最终文本仍由正常的 coordinator 输出显示；这里只投影工具轮中的中间说明，
            // 避免最终回答在终端重复一次。
            ContentBlock::Text { text } if has_tool_use => {
                project_wave_content(text, WAVE_TEXT_DISPLAY_MAX_CHARS)
                    .map(|(content, truncated)| (WaveContentKind::Text, content, truncated))
            }
            _ => None,
        };

        if let Some((kind, content, truncated)) = projected {
            activities.push(AgentActivity::WaveContent {
                wave,
                kind,
                content,
                truncated,
            });
            content_blocks += 1;
        }
    }

    let tools = response
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    activities.push(AgentActivity::WaveDecision {
        wave,
        stop_reason: response.stop_reason.clone(),
        tools,
    });
    activities
}

fn project_wave_content(content: &str, max_chars: usize) -> Option<(String, bool)> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let sanitized: String = normalized
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect();
    let sanitized = sanitized.trim();
    if sanitized.is_empty() {
        return None;
    }

    let truncated = sanitized.chars().count() > max_chars;
    let projected = if truncated {
        sanitized.chars().take(max_chars).collect()
    } else {
        sanitized.to_owned()
    };
    Some((projected, truncated))
}

fn detect_output_limit_warning(
    response: &AnthropicResponse,
    max_tokens: u32,
) -> Option<OutputLimitWarning> {
    let output_tokens = response.usage.as_ref().map(|usage| usage.output_tokens);
    let max_tokens = max_tokens as usize;

    let stop_reason = response.stop_reason.clone();
    let stop_lower = stop_reason
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stopped_for_length =
        stop_lower.contains("max") || stop_lower.contains("length") || stop_lower.contains("token");
    let near_limit = max_tokens > 0 && output_tokens.is_some_and(|tokens| tokens >= max_tokens.saturating_mul(95) / 100);

    if stopped_for_length || near_limit {
        Some(OutputLimitWarning {
            output_tokens: output_tokens.unwrap_or(0),
            max_tokens,
            stop_reason,
            level: if stopped_for_length {
                "truncated"
            } else {
                "warning"
            },
        })
    } else {
        None
    }
}

fn render_output_limit_notice(warning: &OutputLimitWarning, final_text: Option<&str>) -> String {
    let stop_reason = warning.stop_reason.as_deref().unwrap_or("unknown");
    let notice = format!(
        "[Output Limit Notice]\n\
         The model response may be incomplete because the API returned {}/{} output tokens \
         (stop_reason: {}). Treat the text below as a partial answer. Ask the agent to continue, \
         or split the task into smaller steps if a complete long report is required.",
        warning.output_tokens, warning.max_tokens, stop_reason
    );

    match final_text {
        Some(text) if !text.trim().is_empty() => format!("{notice}\n\n{text}"),
        _ => notice,
    }
}

/// 估算结构化消息（role + 所有 content block）的 token 数。
fn count_message_tokens(msg: &StructuredMessage) -> usize {
    use ai_api_provider::estimate_tokens;

    let mut total = estimate_tokens(&msg.role);
    for block in &msg.content {
        total += match block {
            ContentBlock::Text { text } => estimate_tokens(text),
            ContentBlock::ToolUse { name, input, .. } => {
                estimate_tokens(name) + estimate_tokens(&input.to_string())
            }
            ContentBlock::ToolResult { content, .. } => estimate_tokens(content),
            ContentBlock::Thinking { thinking } => estimate_tokens(thinking),
            ContentBlock::Image { detail, .. } => match detail.as_deref() {
                Some("low") => 600,
                Some("high") => 5_000,
                _ => 2_000,
            },
            ContentBlock::Unknown(v) => estimate_tokens(&v.to_string()),
        };
    }
    total
}

fn count_tool_definition_tokens(tools: &[ToolDefinition]) -> usize {
    use ai_api_provider::estimate_tokens;

    tools
        .iter()
        .map(|tool| {
            estimate_tokens(&tool.name)
                + estimate_tokens(&tool.description)
                + estimate_tokens(&tool.input_schema.to_string())
        })
        .sum()
}

fn estimate_outbound_tokens(
    system_prompt: &str,
    tools: &[ToolDefinition],
    messages: &[StructuredMessage],
) -> usize {
    ai_api_provider::tokens::estimate_tokens(system_prompt)
        + count_tool_definition_tokens(tools)
        + messages.iter().map(count_message_tokens).sum::<usize>()
}

fn projection_stable_id(location: &str, content: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    location.hash(&mut hasher);
    content.hash(&mut hasher);
    format!(
        "{}:bytes:{}:hash:{:016x}",
        location,
        content.len(),
        hasher.finish()
    )
}

fn render_projection_notice(
    source_kind: &str,
    source_label: &str,
    original_bytes: usize,
    original_tokens: usize,
    result: &ContextProjectionResult,
) -> String {
    format!(
        "[Context Projection Notice]\n\
         The runtime detected an outbound context segment that exceeded the inline budget.\n\
         That segment has been projected into ECS before this model request.\n\n\
         Location:\n\
         - source_kind: {source_kind}\n\
         - source_label: {source_label}\n\
         - component: {component}\n\
         - paradigm: {paradigm}\n\
         - shards: {shards}\n\
         - original_bytes: {original_bytes}\n\
         - original_estimated_tokens: {original_tokens}\n\n\
         Preview:\n\
         {preview}\n\n\
         Treat the ECS projection above as the place to continue work. Use available tools to inspect, \
         split further, index, summarize, or create facts and relations as needed. Do not assume the \
         projected content is fully visible inline.",
        component = result.component,
        paradigm = result.paradigm,
        shards = result.total_shards,
        preview = result.preview,
    )
}

fn render_history_projection_content(
    wave: usize,
    compact_start: usize,
    compact_end: usize,
    messages: &[StructuredMessage],
) -> String {
    let mut out = format!(
        "# Rolling History Projection\n\n\
         wave: {wave}\n\
         message_range: [{compact_start}..{compact_end})\n\
         projected_messages: {}\n\n",
        messages.len()
    );

    for (offset, message) in messages.iter().enumerate() {
        let idx = compact_start + offset;
        out.push_str(&format!("## message[{idx}] role={}\n", message.role));
        for (block_idx, block) in message.content.iter().enumerate() {
            out.push_str(&format!(
                "### block[{block_idx}] {}\n{}\n\n",
                block_kind(block),
                block_to_projection_text(block)
            ));
        }
    }

    out
}

fn render_history_projection_notice(
    wave: usize,
    compact_start: usize,
    compact_end: usize,
    compacted_messages: usize,
    original_conversation_tokens: usize,
    result: &ContextProjectionResult,
) -> String {
    format!(
        "[Rolling History Projection Notice]\n\
         The runtime detected that many small prior tool-use messages had accumulated into a large conversation history.\n\
         Completed old waves were projected into ECS before wave {wave}; the latest messages remain inline.\n\n\
         Projected history:\n\
         - message_range: [{compact_start}..{compact_end})\n\
         - projected_messages: {compacted_messages}\n\
         - component: {component}\n\
         - paradigm: {paradigm}\n\
         - shards: {shards}\n\
         - original_bytes: {bytes}\n\
         - original_conversation_estimated_tokens_before_projection: {original_conversation_tokens}\n\n\
         Preview:\n\
         {preview}\n\n\
         Continue from the latest inline context. If you need older details, inspect this ECS projection with \
         get_shards/get_shard_content using the paradigm above, or query ActionRecord/TransactionRecord/ToolHistory. \
         Do not repeat broad verification just to reconstruct the projected history.",
        component = result.component,
        paradigm = result.paradigm,
        shards = result.total_shards,
        bytes = result.original_bytes,
        preview = result.preview,
    )
}

fn block_kind(block: &ContentBlock) -> &'static str {
    match block {
        ContentBlock::Text { .. } => "text",
        ContentBlock::Image { .. } => "image",
        ContentBlock::ToolUse { .. } => "tool_use",
        ContentBlock::ToolResult { .. } => "tool_result",
        ContentBlock::Thinking { .. } => "thinking",
        ContentBlock::Unknown(_) => "unknown",
    }
}

fn block_to_projection_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text { text } => text.clone(),
        ContentBlock::Image { detail, .. } => {
            format!(
                "[image block projected; detail={}]",
                detail.as_deref().unwrap_or("auto")
            )
        }
        ContentBlock::ToolUse { id, name, input } => {
            format!("tool_use id={id} name={name}\ninput={input}")
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            format!("tool_result tool_use_id={tool_use_id} is_error={is_error}\n{content}")
        }
        ContentBlock::Thinking { thinking } => thinking.clone(),
        ContentBlock::Unknown(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn tool_exposure_allowlist_filters_the_api_surface() {
        let allowed = vec!["get_entity".to_string(), "modify_component".to_string()];
        let selected = ToolExposure::Allow(&allowed).select(ecs_tool_definitions());
        let names = selected
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["get_entity", "modify_component"]);
        assert!(ToolExposure::None.select(ecs_tool_definitions()).is_empty());
    }

    #[derive(Default)]
    struct MockExecutor {
        projected: Vec<ContextProjectionRequest>,
    }

    #[async_trait]
    impl ToolExecutor for MockExecutor {
        async fn execute(&mut self, call: &ToolCall) -> crate::tool_protocol::ToolCallResult {
            crate::tool_protocol::ToolCallResult {
                tool_use_id: call.id.clone(),
                output: String::new(),
                is_error: false,
            }
        }

        fn project_context_content(
            &mut self,
            request: ContextProjectionRequest,
        ) -> Result<ContextProjectionResult, String> {
            let preview: String = request.content.chars().take(64).collect();
            let original_bytes = request.content.len();
            self.projected.push(request.clone());
            Ok(ContextProjectionResult {
                paradigm: format!("context:test:{}", request.stable_id),
                component: "ShardData".into(),
                total_shards: 1,
                original_bytes,
                preview,
            })
        }
    }

    #[test]
    fn outbound_request_projects_large_message_block() {
        let agent = Agent::new(ApiProvider::MiniMax, "test-key".into());
        let mut executor = MockExecutor::default();
        let large_text = "large context line\n".repeat(3_000);
        let messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: large_text.clone(),
        }])];

        let prepared = agent
            .prepare_outbound_request("system", &[], &messages, &mut executor)
            .expect("large block should project before outbound request");

        assert_eq!(prepared.projected_blocks, 1);
        assert_eq!(executor.projected.len(), 1);
        assert_eq!(executor.projected[0].source_kind, "message_text");
        assert_eq!(executor.projected[0].content, large_text);

        let ContentBlock::Text { text } = &prepared.messages[0].content[0] else {
            panic!("projected block should remain a text notice");
        };
        assert!(text.contains("[Context Projection Notice]"));
        assert!(text.contains("source_kind: message_text"));
        assert!(text.contains("component: ShardData"));
        assert!(!text.contains(&"large context line\n".repeat(100)));
    }

    #[test]
    fn tool_free_report_keeps_large_evidence_inline_and_rejects_over_budget() {
        let mut agent = Agent::new(ApiProvider::MiniMax, "test-key".into());
        agent.config.max_input_tokens = Some(200_000);
        let receipt = format!(
            "{}\n19644 weather_query_tickaction\nUnfinished: historical usage",
            "evidence line\n".repeat(3000)
        );
        let messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: receipt.clone(),
        }])];
        let prepared = agent
            .prepare_tool_free_request("report all subtasks", &messages)
            .unwrap();
        assert_eq!(prepared.projected_blocks, 0);
        assert!(prepared.tools.is_empty());
        let ContentBlock::Text { text } = &prepared.messages[0].content[0] else {
            panic!("missing report")
        };
        assert_eq!(text, &receipt);
        agent.config.max_input_tokens = Some(100);
        assert!(
            agent
                .prepare_tool_free_request("report all subtasks", &messages)
                .is_err()
        );
    }

    #[test]
    fn outbound_request_keeps_small_message_inline() {
        let agent = Agent::new(ApiProvider::MiniMax, "test-key".into());
        let mut executor = MockExecutor::default();
        let messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: "small context".into(),
        }])];

        let prepared = agent
            .prepare_outbound_request("system", &[], &messages, &mut executor)
            .expect("small block should stay inline");

        assert_eq!(prepared.projected_blocks, 0);
        assert!(executor.projected.is_empty());
        let ContentBlock::Text { text } = &prepared.messages[0].content[0] else {
            panic!("small block should remain text");
        };
        assert_eq!(text, "small context");
    }

    #[test]
    fn final_response_collects_all_text_without_thinking_or_json_repair() {
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "private reasoning".into(),
            },
            ContentBlock::Text {
                text: "{\"dispositions\":".into(),
            },
            ContentBlock::Text { text: "[]}".into() },
        ];
        assert_eq!(
            collect_final_text(&blocks).unwrap(),
            "{\"dispositions\":[]}"
        );
        assert!(collect_final_text(&[]).is_none());
        let prose = vec![
            ContentBlock::Text {
                text: "Completed. ".into(),
            },
            ContentBlock::Text { text: "{}".into() },
        ];
        assert_eq!(collect_final_text(&prose).unwrap(), "Completed. {}");
    }

    #[test]
    fn aggregate_budget_projects_small_results_without_changing_tool_ids() {
        let mut agent = Agent::new(ApiProvider::MiniMax, "test-key".into());
        agent.config.max_input_tokens = Some(10_000);
        let mut executor = MockExecutor::default();
        let mut messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: "Keep the task intact".into(),
        }])];
        for i in 0..8 {
            messages.push(StructuredMessage::assistant(vec![ContentBlock::ToolUse {
                id: format!("call-{i}"),
                name: "read".into(),
                input: serde_json::json!({}),
            }]));
            messages.push(StructuredMessage::user(vec![ContentBlock::ToolResult {
                tool_use_id: format!("call-{i}"),
                content: "word ".repeat(1500),
                is_error: false,
            }]));
        }
        let prepared = agent
            .prepare_outbound_request("system", &[], &messages, &mut executor)
            .unwrap();
        assert!(prepared.projected_blocks > 0);
        assert!(
            estimate_outbound_tokens(&prepared.system_prompt, &prepared.tools, &prepared.messages)
                <= 9000
        );
        assert!(
            matches!(&prepared.messages[0].content[0], ContentBlock::Text {text} if text == "Keep the task intact")
        );
        for i in 0..8 {
            assert!(
                matches!(&prepared.messages[2+i*2].content[0], ContentBlock::ToolResult {tool_use_id,..} if tool_use_id == &format!("call-{i}"))
            );
        }
    }

    #[test]
    fn rolling_history_projects_many_small_completed_messages() {
        let agent = Agent::new(ApiProvider::MiniMax, "test-key".into());
        let mut executor = MockExecutor::default();
        let mut messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: "initial spark".into(),
        }])];

        for i in 0..70 {
            messages.push(StructuredMessage {
                role: if i % 2 == 0 { "assistant" } else { "user" }.into(),
                content: vec![ContentBlock::Text {
                    text: format!("small history message {i}"),
                }],
            });
        }

        let projected = agent
            .maybe_project_rolling_history(&mut executor, &mut messages, 200_000, 42)
            .expect("rolling projection should not fail");

        assert!(projected.is_some());
        assert_eq!(executor.projected.len(), 1);
        assert_eq!(executor.projected[0].source_kind, "history_window");
        assert!(
            executor.projected[0]
                .content
                .contains("small history message 0")
        );
        assert!(matches!(
            &messages[1].content[0],
            ContentBlock::Text { text } if text.contains("Rolling History Projection Notice")
        ));
        assert!(messages.iter().any(|message| {
            message.content.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::Text { text } if text.contains("small history message 69")
                )
            })
        }));
    }

    #[test]
    fn rolling_history_keeps_tool_result_with_matching_tool_use() {
        let agent = Agent::new(ApiProvider::MiniMax, "test-key".into());
        let mut executor = MockExecutor::default();
        let mut messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: "initial spark".into(),
        }])];

        for i in 0..70 {
            messages.push(StructuredMessage {
                role: if i % 2 == 0 { "assistant" } else { "user" }.into(),
                content: vec![ContentBlock::Text {
                    text: format!("small history message {i}"),
                }],
            });
        }

        messages[60] = StructuredMessage::assistant(vec![ContentBlock::ToolUse {
            id: "call_boundary".into(),
            name: "get_world_overview".into(),
            input: serde_json::json!({}),
        }]);
        messages[61] = StructuredMessage::user(vec![ContentBlock::ToolResult {
            tool_use_id: "call_boundary".into(),
            content: "ok".into(),
            is_error: false,
        }]);

        assert!(!is_rolling_history_boundary_safe(&messages, 61));
        assert!(is_rolling_history_boundary_safe(&messages, 60));

        let projected = agent
            .maybe_project_rolling_history(&mut executor, &mut messages, 200_000, 42)
            .expect("rolling projection should not fail");

        assert!(projected.is_some());
        assert_eq!(executor.projected.len(), 1);
        assert!(matches!(
            &messages[2].content[0],
            ContentBlock::ToolUse { id, .. } if id == "call_boundary"
        ));
        assert!(matches!(
            &messages[3].content[0],
            ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "call_boundary"
        ));
    }

    #[test]
    fn tool_loop_guard_forces_final_response_at_total_action_budget() {
        let decision = tool_loop_guard_decision(TOOL_LOOP_MAX_TOTAL_ACTIONS, 0, 0, false)
            .expect("total action budget should activate the guard");

        let ToolLoopGuardDecision::ForceFinal(reason) = decision else {
            panic!("hard action budget must force a final response");
        };
        assert!(reason.contains("hard action budget"));
        assert!(reason.contains("64"));
    }

    #[test]
    fn tool_loop_guard_forces_final_response_after_error_waves() {
        let decision = tool_loop_guard_decision(0, TOOL_LOOP_MAX_CONSECUTIVE_ERROR_WAVES, 0, false)
            .expect("repeated all-error waves should activate the guard");

        let ToolLoopGuardDecision::ForceFinal(reason) = decision else {
            panic!("repeated failures must force a final response");
        };
        assert!(reason.contains("consecutive tool waves all failed"));
    }

    #[test]
    fn tool_loop_guard_exposes_read_only_behavior_before_forcing_final() {
        let first =
            tool_loop_guard_decision(12, 0, TOOL_LOOP_MAX_CONSECUTIVE_NON_MUTATING_WAVES, false)
                .expect("read-only threshold should activate convergence feedback");
        assert!(matches!(first, ToolLoopGuardDecision::Converge(_)));

        let repeated =
            tool_loop_guard_decision(13, 0, TOOL_LOOP_MAX_CONSECUTIVE_NON_MUTATING_WAVES, true)
                .expect("repeated read-only behavior should force a final response");
        assert!(matches!(repeated, ToolLoopGuardDecision::ForceFinal(_)));
    }

    #[test]
    fn convergence_notice_tells_model_to_answer_the_waiting_user() {
        let notice = render_tool_loop_convergence_notice("12 read-only waves", 18);

        assert!(notice.contains("your own recent behavior"));
        assert!(notice.contains("user is still waiting"));
        assert!(notice.contains("at most one focused write/render action"));
    }

    #[test]
    fn mutating_tool_classification_marks_writes_and_views() {
        for action in ["create", "next", "submit", "finish", "rollback", "inspect"] {
            let call = ToolCall {
                id: "test".into(),
                name: "dream_task".into(),
                input: serde_json::json!({"action":action}),
            };
            assert_eq!(is_mutating_call(&call), action != "inspect");
        }
        assert!(is_mutating_tool("dream_policy"));
        assert!(is_mutating_tool("modify_component"));
        assert!(is_mutating_tool("render_view"));
        assert!(is_mutating_tool("render_mind_view"));
        assert!(is_mutating_tool("shard_content"));
        assert!(!is_mutating_tool("get_world_overview"));
        assert!(!is_mutating_tool("query_entities"));
        assert!(!is_mutating_tool("execute_cli"));
    }

    #[test]
    fn wave_projection_exposes_returned_thinking_intermediate_text_and_decision() {
        let response = AnthropicResponse {
            content: vec![
                ContentBlock::Thinking {
                    thinking: "先检查 ECS，再修改目标组件。".into(),
                },
                ContentBlock::Text {
                    text: "我先读取当前状态。".into(),
                },
                ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "query_entities".into(),
                    input: serde_json::json!({"query_string": "AgentState"}),
                },
            ],
            stop_reason: Some("tool_use".into()),
            usage: None,
        };

        let activities = project_wave_activities(&response, 4);

        assert!(matches!(
            &activities[0],
            AgentActivity::WaveContent {
                wave: 4,
                kind: WaveContentKind::Thinking,
                content,
                truncated: false,
            } if content.contains("检查 ECS")
        ));
        assert!(matches!(
            &activities[1],
            AgentActivity::WaveContent {
                wave: 4,
                kind: WaveContentKind::Text,
                content,
                truncated: false,
            } if content.contains("读取当前状态")
        ));
        assert!(matches!(
            &activities[2],
            AgentActivity::WaveDecision {
                wave: 4,
                stop_reason: Some(reason),
                tools,
            } if reason == "tool_use" && tools == &["query_entities"]
        ));
    }

    #[test]
    fn wave_projection_leaves_final_text_to_normal_final_output() {
        let response = AnthropicResponse {
            content: vec![
                ContentBlock::Thinking {
                    thinking: "已经完成，可以回答。".into(),
                },
                ContentBlock::Text {
                    text: "最终回答".into(),
                },
            ],
            stop_reason: Some("end_turn".into()),
            usage: None,
        };

        let activities = project_wave_activities(&response, 2);

        assert_eq!(activities.len(), 2);
        assert!(matches!(
            activities[0],
            AgentActivity::WaveContent {
                kind: WaveContentKind::Thinking,
                ..
            }
        ));
        assert!(matches!(activities[1], AgentActivity::WaveDecision { .. }));
    }

    #[test]
    fn wave_content_projection_is_unicode_safe_bounded_and_control_free() {
        let (content, truncated) =
            project_wave_content("  你好\r\n\x1b[31m继续\0处理  ", 8).expect("visible content");

        assert!(truncated);
        assert_eq!(content.chars().count(), 8);
        assert!(!content.contains('\r'));
        assert!(!content.contains('\x1b'));
        assert!(!content.contains('\0'));
    }

    #[test]
    fn output_limit_warning_detects_length_stop_reason() {
        let response = AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "partial".into(),
            }],
            stop_reason: Some("max_tokens".into()),
            usage: Some(ai_api_provider::Usage {
                input_tokens: 100,
                output_tokens: 128,
            }),
        };

        let warning = detect_output_limit_warning(&response, 4096)
            .expect("max token stop reason should warn");

        assert_eq!(warning.level, "truncated");
        assert_eq!(warning.output_tokens, 128);
    }

    #[test]
    fn output_limit_warning_detects_near_limit_usage() {
        let response = AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "maybe partial".into(),
            }],
            stop_reason: Some("end_turn".into()),
            usage: Some(ai_api_provider::Usage {
                input_tokens: 100,
                output_tokens: 3900,
            }),
        };

        let warning =
            detect_output_limit_warning(&response, 4096).expect("near output limit should warn");

        assert_eq!(warning.level, "warning");
        assert_eq!(warning.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn output_limit_warning_ignores_normal_response() {
        let response = AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "complete".into(),
            }],
            stop_reason: Some("end_turn".into()),
            usage: Some(ai_api_provider::Usage {
                input_tokens: 100,
                output_tokens: 500,
            }),
        };

        assert!(detect_output_limit_warning(&response, 4096).is_none());
    }

    #[test]
    fn output_limit_notice_preserves_partial_text() {
        let warning = OutputLimitWarning {
            output_tokens: 4096,
            max_tokens: 4096,
            stop_reason: Some("max_tokens".into()),
            level: "truncated",
        };

        let notice = render_output_limit_notice(&warning, Some("partial answer"));

        assert!(notice.contains("[Output Limit Notice]"));
        assert!(notice.contains("partial answer"));
    }

    #[test]
    fn unfulfilled_tool_commitment_retries_when_no_actions_were_taken() {
        assert!(should_retry_unfulfilled_tool_commitment(
            Some("收到，转给 tool 层去动手：更新 SessionContext.weather"),
            0,
            true,
            false,
        ));
    }

    #[test]
    fn unfulfilled_tool_commitment_does_not_retry_after_actions() {
        assert!(!should_retry_unfulfilled_tool_commitment(
            Some("已更新 ECS，并写入 SessionContext。"),
            1,
            true,
            false,
        ));
    }

    #[test]
    fn unfulfilled_tool_commitment_does_not_retry_direct_mode() {
        assert!(!should_retry_unfulfilled_tool_commitment(
            Some("这应该交给工具层执行。"),
            0,
            false,
            false,
        ));
    }

    #[test]
    fn tool_free_answer_is_not_mistaken_for_a_tool_commitment() {
        assert!(!should_retry_unfulfilled_tool_commitment(
            Some("看不到原话；按你的要求不调用工具，也不会编造。"),
            0,
            true,
            false,
        ));
    }

    #[test]
    fn api_retry_policy_retries_transient_http_statuses() {
        for status in [408, 409, 425, 429, 500, 502, 503, 504, 529] {
            assert!(
                is_retryable_api_error(&ApiError::ApiResponse {
                    status,
                    body: "temporary".into(),
                }),
                "status {status} should be retryable"
            );
        }
    }

    #[test]
    fn api_retry_policy_does_not_retry_deterministic_http_statuses() {
        for status in [400, 401, 403, 404, 413, 422] {
            assert!(
                !is_retryable_api_error(&ApiError::ApiResponse {
                    status,
                    body: "deterministic".into(),
                }),
                "status {status} should not be retryable"
            );
        }
    }

    #[test]
    fn api_reconnect_delay_uses_bounded_exponential_backoff() {
        assert_eq!(api_reconnect_delay_ms(1), 1_000);
        assert_eq!(api_reconnect_delay_ms(2), 2_000);
        assert_eq!(api_reconnect_delay_ms(3), 4_000);
        assert_eq!(api_reconnect_delay_ms(4), 8_000);
        assert_eq!(api_reconnect_delay_ms(5), 8_000);
    }
}
