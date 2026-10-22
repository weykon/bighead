use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::error::ApiError;
use crate::protocol::ApiProtocol;
use crate::provider::{provider_meta, ApiProvider};
use crate::proxy::ProxyConfig;

// ── Tool Use 协议类型 ───────────────────────────────────────────────────

/// Anthropic Messages API 中的内容块。
///
/// 使用手动 Deserialize 实现，以优雅处理未知块类型
///（如 MiniMax 的 "thinking"、未来 Anthropic 新增的块类型）。
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
        detail: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Thinking {
        thinking: String,
    },
    Unknown(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

impl Serialize for ContentBlock {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ContentBlock::Text { text } => {
                serde_json::json!({"type": "text", "text": text}).serialize(serializer)
            }
            ContentBlock::Image { source, detail } => {
                let mut value = serde_json::json!({"type": "image", "source": source});
                if let Some(detail) = detail {
                    value["detail"] = serde_json::json!(detail);
                }
                value.serialize(serializer)
            }
            ContentBlock::ToolUse { id, name, input } => {
                serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input}).serialize(serializer)
            }
            ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                serde_json::json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error}).serialize(serializer)
            }
            ContentBlock::Thinking { thinking } => {
                serde_json::json!({"type": "thinking", "thinking": thinking}).serialize(serializer)
            }
            ContentBlock::Unknown(v) => v.serialize(serializer),
        }
    }
}

impl<'de> serde::Deserialize<'de> for ContentBlock {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(deserializer)?;
        let block_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                let text = v
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ContentBlock::Text { text })
            }
            "image" => {
                let source = v
                    .get("source")
                    .cloned()
                    .ok_or_else(|| serde::de::Error::custom("image block missing source"))
                    .and_then(|source| {
                        serde_json::from_value(source).map_err(serde::de::Error::custom)
                    })?;
                let detail = v.get("detail").and_then(|t| t.as_str()).map(str::to_string);
                Ok(ContentBlock::Image { source, detail })
            }
            "tool_use" => {
                let id = v
                    .get("id")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = v
                    .get("name")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = v
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                Ok(ContentBlock::ToolUse { id, name, input })
            }
            "tool_result" => {
                let tool_use_id = v
                    .get("tool_use_id")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = v
                    .get("content")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_error = v.get("is_error").and_then(|t| t.as_bool()).unwrap_or(false);
                Ok(ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                })
            }
            "thinking" => {
                let thinking = v
                    .get("thinking")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ContentBlock::Thinking { thinking })
            }
            _ => Ok(ContentBlock::Unknown(v)),
        }
    }
}

/// 模型可调用的工具定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// 带内容块的结构化消息（用于 tool use 对话）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl StructuredMessage {
    pub fn user(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: "user".into(),
            content: blocks,
        }
    }

    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: "assistant".into(),
            content: blocks,
        }
    }
}

/// API 返回的 token 用量信息。
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: usize,
    pub output_tokens: usize,
}

/// Anthropic Messages API（tool use 模式）的解析后响应。
#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// 所有 API 请求的默认 User-Agent。
/// 部分供应商（如 Kimi For Coding）会检查此头以验证客户端是否为认可的编码 agent。
pub const DEFAULT_USER_AGENT: &str = "Claude-Code/1.0";

/// 单次 API 调用的配置。
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub provider: ApiProvider,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
    /// 响应最大 token 数，默认 4096。
    pub max_tokens: u32,
    /// 可选的最大输入 token 数。设置后，agent 循环在每次 API 调用前
    /// 会检查累计输入 token 是否超出此限制。
    /// `None` 时使用 [`model_limits`] 中该模型的默认值。
    pub max_input_tokens: Option<usize>,
    /// User-Agent 头。默认为 `DEFAULT_USER_AGENT`。
    /// 部分供应商（如 Kimi For Coding）要求使用认可的 coding agent User-Agent。
    pub user_agent: String,
}

impl ApiConfig {
    pub fn new(provider: ApiProvider, api_key: String) -> Self {
        let model = provider_meta(provider).default_model.to_string();
        Self {
            provider,
            api_key,
            model,
            base_url: None,
            max_tokens: 4096,
            max_input_tokens: None,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }

    /// 解析供应商的端点 URL。
    pub fn endpoint(&self) -> String {
        let meta = provider_meta(self.provider);
        let base = self.base_url.as_deref().unwrap_or(meta.base_url);
        match meta.protocol {
            ApiProtocol::Anthropic => {
                let base = base.trim_end_matches('/');
                if base.ends_with("/v1/messages") {
                    base.to_string()
                } else {
                    format!("{}/v1/messages", base)
                }
            }
            ApiProtocol::OpenAiCompat => {
                let base = base.trim_end_matches('/');
                if base.ends_with("/chat/completions") {
                    base.to_string()
                } else {
                    format!("{}/chat/completions", base)
                }
            }
            ApiProtocol::Gemini => {
                format!("{}/models/{}:generateContent", base, self.model)
            }
        }
    }
}

/// 单条聊天消息（角色 + 内容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

// ── 队列 / 重试配置 ─────────────────────────────────────────────────────

/// 串行请求队列、重试行为和速率限制处理的配置。
/// 所有 API 调用通过单个后台 worker 执行，worker 强制执行这些限制。
///
/// # 默认值
///
/// | 字段 | 默认值 | 含义 |
/// |-------|---------|---------|
/// | `max_retries` | 5 | 速率限制时最多重试 5 次 |
/// | `base_delay_ms` | 1 000 | 退避起始延迟 1 秒 |
/// | `max_delay_ms` | 30 000 | 退避延迟上限 30 秒 |
/// | `backoff_multiplier` | 2.0 | 每次重试延迟翻倍 |
/// | `min_interval_ms` | 1 000 | 请求间隔 ≥ 1 秒 |
/// | `request_timeout_secs` | 120 | 单次请求硬超时 |
/// | `rate_limit_codes` | [429, 529] | 触发重试的 HTTP 状态码 |
/// | `max_queue_depth` | 0（无限制） | 队列超此值时拒绝新请求 |
/// | `adaptive_interval` | false | 遇速率限制时自动增加间隔 |
/// | `queue_capacity` | 128 | Tokio 通道缓冲区大小 |
///
/// # 示例
///
/// ```ignore
/// use ai_api_provider::QueueConfig;
///
/// let config = QueueConfig {
///     max_queue_depth: 50,          // 超过 50 个待处理请求时拒绝
///     adaptive_interval: true,      // 遇到 429 时自动降速
///     rate_limit_codes: vec![429, 503, 529],
///     ..Default::default()
/// };
/// let client = ApiClient::with_config(config);
/// ```
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// 初始请求之后的重试次数（0 = 不重试）。
    pub max_retries: u32,
    /// 初始退避延迟（毫秒）。
    pub base_delay_ms: u64,
    /// 退避延迟上限。
    pub max_delay_ms: u64,
    /// 每次重试的延迟倍率（如 2.0 = 指数退避）。
    pub backoff_multiplier: f64,
    /// 连续请求之间的最小间隔（毫秒）。
    /// worker 在每次请求完成后等待此时间再处理下一个。
    pub min_interval_ms: u64,
    /// 单次请求超时（秒），含连接 + 完成。
    pub request_timeout_secs: u64,
    /// 触发重试 + 退避的 HTTP 状态码。
    /// 默认：`[429, 529]`。可添加 `500`、`502`、`503` 处理瞬时服务器错误。
    pub rate_limit_codes: Vec<u16>,
    /// 队列中可等待的最大请求数。0 = 无限制。
    /// 超限时，`chat()` / `chat_with_tools()` 立即返回
    /// `ApiError::QueueFull { depth }` 而非阻塞。
    pub max_queue_depth: usize,
    /// 为 true 时，worker 在每次收到速率限制码后将 `min_interval_ms` 翻倍
    ///（上限为 `max_delay_ms`），并在请求成功后逐步衰减回配置值。
    pub adaptive_interval: bool,
    /// 内部 tokio 通道缓冲区大小（在途请求槽位数）。
    pub queue_capacity: usize,
}

/// 向后兼容别名。
pub type RetryConfig = QueueConfig;

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 1_000,
            max_delay_ms: 30_000,
            backoff_multiplier: 2.0,
            min_interval_ms: 1_000,
            request_timeout_secs: 120,
            rate_limit_codes: vec![429, 529],
            max_queue_depth: 0,
            adaptive_interval: false,
            queue_capacity: 128,
        }
    }
}

/// API 客户端串行请求队列的可观测统计。
/// 所有字段均为原子变量——读取不阻塞，开销极小。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiStats {
    /// 队列中当前等待的请求数。
    pub queue_depth: usize,
    /// worker 当前是否因 429/529 速率限制而休眠。
    pub rate_limit_active: bool,
    /// 当前重试次数（0 = 无重试进行中）。
    pub retry_attempt: u32,
    /// 距离下次重试还剩多少毫秒（不在退避中则为 0）。
    pub next_retry_in_ms: u64,
}

impl Default for ApiStats {
    fn default() -> Self {
        Self {
            queue_depth: 0,
            rate_limit_active: false,
            retry_attempt: 0,
            next_retry_in_ms: 0,
        }
    }
}

/// api_worker 与所有 ApiClient 克隆之间共享的内部状态。
/// 所有字段均为原子变量，`ApiClient::stats()` 可无锁读取。
struct ClientState {
    queue_depth: AtomicUsize,
    rate_limit_active: AtomicBool,
    retry_attempt: AtomicU32,
    next_retry_in_ms: AtomicU64,
}

impl ClientState {
    fn new() -> Self {
        Self {
            queue_depth: AtomicUsize::new(0),
            rate_limit_active: AtomicBool::new(false),
            retry_attempt: AtomicU32::new(0),
            next_retry_in_ms: AtomicU64::new(0),
        }
    }

    fn stats(&self) -> ApiStats {
        ApiStats {
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            rate_limit_active: self.rate_limit_active.load(Ordering::Relaxed),
            retry_attempt: self.retry_attempt.load(Ordering::Relaxed),
            next_retry_in_ms: self.next_retry_in_ms.load(Ordering::Relaxed),
        }
    }
}

// ── 内部队列类型 ─────────────────────────────────────────────────────────

struct ChatReq {
    config: ApiConfig,
    messages: Vec<ChatMessage>,
    tx: oneshot::Sender<Result<String, ApiError>>,
}

struct ToolsReq {
    config: ApiConfig,
    messages: Vec<StructuredMessage>,
    tools: Vec<ToolDefinition>,
    system: Option<String>,
    tx: oneshot::Sender<Result<AnthropicResponse, ApiError>>,
}

enum ApiRequest {
    Chat(ChatReq),
    ChatWithTools(ToolsReq),
}

// ── 纯 HTTP 辅助函数 ─────────────────────────────────────────────────────

async fn do_chat(
    http: &reqwest::Client,
    config: &ApiConfig,
    messages: &[ChatMessage],
) -> Result<String, ApiError> {
    let meta = provider_meta(config.provider);
    let body = build_request_body(config, messages);

    let request = match meta.protocol {
        ApiProtocol::Anthropic => http
            .post(&config.endpoint())
            .header("Content-Type", "application/json")
            .header("User-Agent", &config.user_agent)
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01"),
        ApiProtocol::OpenAiCompat => {
            let mut req = http
                .post(&config.endpoint())
                .header("Content-Type", "application/json")
                .header("User-Agent", &config.user_agent);
            if !config.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", config.api_key));
            }
            req
        }
        ApiProtocol::Gemini => {
            let url = format!("{}?key={}", config.endpoint(), config.api_key);
            http.post(&url)
                .header("Content-Type", "application/json")
                .header("User-Agent", &config.user_agent)
        }
    };

    let response = request.json(&body).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(ApiError::ApiResponse {
            status: status.as_u16(),
            body: text.chars().take(300).collect(),
        });
    }
    extract_response_text(meta.protocol, &text)
}

async fn do_chat_with_tools(
    http: &reqwest::Client,
    config: &ApiConfig,
    messages: &[StructuredMessage],
    tools: &[ToolDefinition],
    system: Option<&str>,
) -> Result<AnthropicResponse, ApiError> {
    let meta = provider_meta(config.provider);
    if meta.protocol != ApiProtocol::Anthropic {
        return Err(ApiError::Parse(format!(
            "chat_with_tools only supports Anthropic protocol, got {:?}",
            meta.protocol
        )));
    }

    let body = build_tools_request_body(config, messages, tools, system);
    let response = http
        .post(&config.endpoint())
        .header("Content-Type", "application/json")
        .header("User-Agent", &config.user_agent)
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(ApiError::ApiResponse {
            status: status.as_u16(),
            body: text.chars().take(300).collect(),
        });
    }
    serde_json::from_str::<AnthropicResponse>(&text)
        .map_err(|e| ApiError::Parse(format!("Failed to parse AnthropicResponse: {e}")))
}

// ── 重试循环 ─────────────────────────────────────────────────────────────

/// 最多执行 `config.max_retries + 1` 次 `f`，遇速率限制码时休眠重试。
async fn with_retry<F, Fut, T>(
    config: &QueueConfig,
    state: Option<&Arc<ClientState>>,
    mut f: F,
) -> Result<T, ApiError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ApiError>>,
{
    let mut delay = config.base_delay_ms;
    for attempt in 0..=config.max_retries {
        match f().await {
            Err(ApiError::ApiResponse { status, ref body })
                if config.rate_limit_codes.contains(&status) && attempt < config.max_retries =>
            {
                if let Some(s) = state {
                    s.retry_attempt.store(attempt + 1, Ordering::Relaxed);
                    s.rate_limit_active.store(true, Ordering::Relaxed);
                    s.next_retry_in_ms.store(delay, Ordering::Relaxed);
                }
                eprintln!(
                    "[api-queue] rate-limited ({status}), retry {}/{} in {delay}ms — {}",
                    attempt + 1,
                    config.max_retries,
                    body
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
                delay =
                    ((delay as f64 * config.backoff_multiplier) as u64).min(config.max_delay_ms);
            }
            other => {
                if let Some(s) = state {
                    s.rate_limit_active.store(false, Ordering::Relaxed);
                    s.retry_attempt.store(0, Ordering::Relaxed);
                    s.next_retry_in_ms.store(0, Ordering::Relaxed);
                }
                return other;
            }
        }
    }
    unreachable!()
}

// ── 后台 worker ──────────────────────────────────────────────────────────

async fn api_worker(
    http: reqwest::Client,
    mut rx: mpsc::Receiver<ApiRequest>,
    config: QueueConfig,
    state: Arc<ClientState>,
) {
    use std::time::Instant;
    let mut last_request_at: Option<Instant> = None;
    let mut effective_interval = config.min_interval_ms;

    while let Some(req) = rx.recv().await {
        // 队列深度追踪
        state.queue_depth.fetch_add(1, Ordering::Relaxed);

        // 强制执行连续请求之间的最小间隔
        if effective_interval > 0 {
            if let Some(last) = last_request_at {
                let elapsed = last.elapsed().as_millis() as u64;
                if elapsed < effective_interval {
                    tokio::time::sleep(Duration::from_millis(effective_interval - elapsed)).await;
                }
            }
        }
        match req {
            ApiRequest::Chat(r) => {
                let result = {
                    let http = &http;
                    let req_config = &r.config;
                    let messages = &r.messages;
                    with_retry(&config, Some(&state), || async move {
                        do_chat(http, req_config, messages).await
                    })
                    .await
                };
                last_request_at = Some(Instant::now());
                // 自适应间隔：遇速率限制时降速，成功时衰减
                if config.adaptive_interval {
                    match &result {
                        Err(ApiError::ApiResponse { status, .. })
                            if config.rate_limit_codes.contains(status) =>
                        {
                            effective_interval = (effective_interval * 2).min(config.max_delay_ms);
                            eprintln!("[api-queue] adaptive interval → {effective_interval}ms");
                        }
                        Ok(_) if effective_interval > config.min_interval_ms => {
                            effective_interval =
                                (effective_interval * 3 / 4).max(config.min_interval_ms);
                        }
                        _ => {}
                    }
                }
                let _ = r.tx.send(result);
            }
            ApiRequest::ChatWithTools(r) => {
                let result = {
                    let http = &http;
                    let req_config = &r.config;
                    let messages = &r.messages;
                    let tools = &r.tools;
                    let system = r.system.as_deref();
                    with_retry(&config, Some(&state), || async move {
                        do_chat_with_tools(http, req_config, messages, tools, system).await
                    })
                    .await
                };
                last_request_at = Some(Instant::now());
                // 自适应间隔：遇速率限制时降速，成功时衰减
                if config.adaptive_interval {
                    match &result {
                        Err(ApiError::ApiResponse { status, .. })
                            if config.rate_limit_codes.contains(status) =>
                        {
                            effective_interval = (effective_interval * 2).min(config.max_delay_ms);
                            eprintln!("[api-queue] adaptive interval → {effective_interval}ms");
                        }
                        Ok(_) if effective_interval > config.min_interval_ms => {
                            effective_interval =
                                (effective_interval * 3 / 4).max(config.min_interval_ms);
                        }
                        _ => {}
                    }
                }
                let _ = r.tx.send(result);
            }
        }

        state.queue_depth.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── 公开客户端 ───────────────────────────────────────────────────────────

/// 异步 LLM API 客户端，由串行请求队列支持。
///
/// 所有请求由后台 worker 任务逐一处理。
/// 遇速率限制（可配置状态码，默认 429/529）时，worker 退避并重试
/// **同一个**请求，不会丢弃。
///
/// `ApiClient` 可廉价 `Clone`——所有克隆共享同一队列，
/// 因此所有 agent 共享同一全局速率限制窗口。
#[derive(Clone)]
pub struct ApiClient {
    tx: mpsc::Sender<ApiRequest>,
    state: Arc<ClientState>,
    config: Arc<QueueConfig>,
}

impl ApiClient {
    /// 使用默认队列配置创建客户端。
    ///
    /// **必须在 Tokio runtime 上下文内调用**（会 spawn 后台任务）。
    pub fn new() -> Self {
        Self::with_config(QueueConfig::default())
    }

    /// 使用自定义重试配置创建客户端（`with_config` 的别名）。
    pub fn with_retry(retry: RetryConfig) -> Self {
        Self::with_config(retry)
    }

    /// 使用完整队列配置创建客户端。
    ///
    /// HTTP 代理从环境变量读取（`BIG_HEAD_PROXY` → `HTTPS_PROXY` → `HTTP_PROXY` → `ALL_PROXY`）。
    /// 见 [`crate::proxy::ProxyConfig`]。
    pub fn with_config(config: QueueConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.queue_capacity);
        let state = Arc::new(ClientState::new());
        let worker_state = Arc::clone(&state);
        let config_arc = Arc::new(config.clone());
        let proxy = ProxyConfig::from_env();
        let http = proxy
            .apply(reqwest::Client::builder())
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .expect("Failed to build reqwest::Client");
        tokio::spawn(api_worker(http, rx, config, worker_state));
        Self {
            tx,
            state,
            config: config_arc,
        }
    }

    /// 返回当前队列和速率限制统计。
    /// 廉价操作——直接读取原子变量，无锁、无通道通信。
    pub fn stats(&self) -> ApiStats {
        self.state.stats()
    }

    /// 发送聊天补全请求，返回完整响应文本。
    pub async fn chat(
        &self,
        config: &ApiConfig,
        messages: &[ChatMessage],
    ) -> Result<String, ApiError> {
        // 队列深度守卫：队列满时提前拒绝
        if self.config.max_queue_depth > 0 {
            let depth = self.state.queue_depth.load(Ordering::Relaxed);
            if depth >= self.config.max_queue_depth {
                return Err(ApiError::QueueFull { depth });
            }
        }
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(ApiRequest::Chat(ChatReq {
                config: config.clone(),
                messages: messages.to_vec(),
                tx: resp_tx,
            }))
            .await
            .map_err(|_| ApiError::Parse("api queue closed".into()))?;
        resp_rx
            .await
            .map_err(|_| ApiError::Parse("api worker dropped response".into()))?
    }

    /// 发送 tool-use 聊天请求，返回完整结构化响应。
    ///
    /// 仅支持 `ApiProtocol::Anthropic`（Claude + MiniMax + MiMo）。
    pub async fn chat_with_tools(
        &self,
        config: &ApiConfig,
        messages: &[StructuredMessage],
        tools: &[ToolDefinition],
        system: Option<&str>,
    ) -> Result<AnthropicResponse, ApiError> {
        // 队列深度守卫：队列满时提前拒绝
        if self.config.max_queue_depth > 0 {
            let depth = self.state.queue_depth.load(Ordering::Relaxed);
            if depth >= self.config.max_queue_depth {
                return Err(ApiError::QueueFull { depth });
            }
        }
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(ApiRequest::ChatWithTools(ToolsReq {
                config: config.clone(),
                messages: messages.to_vec(),
                tools: tools.to_vec(),
                system: system.map(str::to_string),
                tx: resp_tx,
            }))
            .await
            .map_err(|_| ApiError::Parse("api queue closed".into()))?;
        resp_rx
            .await
            .map_err(|_| ApiError::Parse("api worker dropped response".into()))?
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

/// 根据供应商线协议构建请求体 JSON。
fn build_request_body(config: &ApiConfig, messages: &[ChatMessage]) -> serde_json::Value {
    let meta = provider_meta(config.provider);
    match meta.protocol {
        ApiProtocol::Anthropic => {
            let api_messages: Vec<serde_json::Value> = messages
                .iter()
                .filter(|m| m.role != "system")
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect();
            let system = messages
                .iter()
                .find(|m| m.role == "system")
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let mut body = serde_json::json!({
                "model": config.model,
                "max_tokens": config.max_tokens,
                "messages": api_messages,
            });
            if !system.is_empty() {
                body["system"] = serde_json::json!(system);
            }
            body
        }
        ApiProtocol::OpenAiCompat => {
            let api_messages: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect();
            serde_json::json!({
                "model": config.model,
                "messages": api_messages,
                "max_tokens": config.max_tokens,
            })
        }
        ApiProtocol::Gemini => {
            let parts: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "role": if m.role == "assistant" { "model" } else { "user" },
                        "parts": [{ "text": m.content }],
                    })
                })
                .collect();
            serde_json::json!({ "contents": parts })
        }
    }
}

/// 构建 tool-use 模式请求体（仅 Anthropic 协议）。
fn build_tools_request_body(
    config: &ApiConfig,
    messages: &[StructuredMessage],
    tools: &[ToolDefinition],
    system: Option<&str>,
) -> serde_json::Value {
    let api_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
        .collect();

    let mut body = serde_json::json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "messages": api_messages,
    });

    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap();
    }

    if let Some(sys) = system {
        if !sys.is_empty() {
            body["system"] = serde_json::json!(sys);
        }
    }

    body
}

/// 从协议特定的响应 JSON 中提取助手的文本。
fn extract_response_text(protocol: ApiProtocol, raw: &str) -> Result<String, ApiError> {
    let json: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| ApiError::Parse(e.to_string()))?;

    match protocol {
        ApiProtocol::Anthropic => {
            // Anthropic 标准格式: { content: [{ type: "text", text: "..."}, ...] }
            if let Some(arr) = json.get("content").and_then(|v| v.as_array()) {
                let texts: Vec<&str> = arr
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect();
                if !texts.is_empty() {
                    return Ok(texts.join(""));
                }

                // 部分兼容层省略 `type` 但仍提供 `text`
                let texts: Vec<&str> = arr
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect();
                if !texts.is_empty() {
                    return Ok(texts.join(""));
                }
            }

            // 部分供应商直接在这些字段中返回纯字符串
            if let Some(s) = json.get("content").and_then(|v| v.as_str()) {
                return Ok(s.to_string());
            }
            if let Some(s) = json.get("completion").and_then(|v| v.as_str()) {
                return Ok(s.to_string());
            }
            if let Some(s) = json.get("output_text").and_then(|v| v.as_str()) {
                return Ok(s.to_string());
            }

            // 兼容性回退：部分"Anthropic 兼容"端点回复 OpenAI 格式
            let openai_like = json
                .get("choices")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    json.pointer("/result/choices/0/message/content")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| {
                    json.pointer("/data/choices/0/message/content")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                });
            if let Some(s) = openai_like {
                return Ok(s);
            }

            // 检测纯 thinking 响应（模型将所有 token 用于思考，
            // 未产生文本输出）。常见于 MiniMax-M2.7 和 Claude extended thinking。
            if let Some(arr) = json.get("content").and_then(|v| v.as_array()) {
                let has_thinking = arr
                    .iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking"));
                if has_thinking {
                    return Err(ApiError::Parse(
                        "Thinking-only response: model used all output tokens on reasoning \
                         with no text produced. Try increasing max_tokens or simplifying the prompt."
                            .to_string(),
                    ));
                }
            }

            let keys = json
                .as_object()
                .map(|o| {
                    let mut ks: Vec<&str> = o.keys().map(|k| k.as_str()).collect();
                    ks.sort_unstable();
                    ks.join(",")
                })
                .unwrap_or_else(|| "(non-object)".to_string());
            let excerpt: String = raw
                .chars()
                .take(400)
                .collect::<String>()
                .replace('\n', "\\n")
                .replace('\r', "\\r");
            Err(ApiError::Parse(format!(
                "No text content in Anthropic response (keys: {keys}; excerpt: {excerpt})"
            )))
        }

        ApiProtocol::OpenAiCompat => json["choices"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|c| c["message"]["content"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ApiError::Parse("No content in OpenAI-compatible response".into())),

        ApiProtocol::Gemini => json["candidates"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|c| c["content"]["parts"].as_array())
            .and_then(|parts| parts.first())
            .and_then(|p| p["text"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ApiError::Parse("No text in Gemini response".into())),
    }
}

/// 从环境变量解析 API key（尝试该供应商的所有环境变量名）。
pub fn resolve_api_key(provider: ApiProvider) -> Option<String> {
    let meta = provider_meta(provider);
    for var_name in meta.env_vars {
        if let Ok(val) = std::env::var(var_name) {
            let val = val.trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    if meta.local {
        return Some(String::new());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_anthropic() {
        let config = ApiConfig::new(ApiProvider::Claude, "test".into());
        assert!(config.endpoint().ends_with("/v1/messages"));
    }

    #[test]
    fn test_endpoint_anthropic_auto_appends_path() {
        let mut config = ApiConfig::new(ApiProvider::Claude, "test".into());
        config.base_url = Some("http://relay.example.com:8021".into());
        assert_eq!(
            config.endpoint(),
            "http://relay.example.com:8021/v1/messages"
        );
    }

    #[test]
    fn test_endpoint_anthropic_no_double_path() {
        let mut config = ApiConfig::new(ApiProvider::Claude, "test".into());
        config.base_url = Some("http://relay.example.com:8021/v1/messages".into());
        assert_eq!(
            config.endpoint(),
            "http://relay.example.com:8021/v1/messages"
        );
    }

    #[test]
    fn test_endpoint_openai_appends_path() {
        let config = ApiConfig::new(ApiProvider::DeepSeek, "test".into());
        assert!(config.endpoint().ends_with("/chat/completions"));
    }

    #[test]
    fn test_endpoint_gemini_includes_model() {
        let config = ApiConfig::new(ApiProvider::Gemini, "test".into());
        assert!(config.endpoint().contains("gemini-2.0-flash"));
    }

    #[test]
    fn test_build_body_anthropic() {
        let config = ApiConfig::new(ApiProvider::Claude, "test".into());
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
        ];
        let body = build_request_body(&config, &messages);
        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_extract_anthropic() {
        let raw = r#"{"content":[{"type":"text","text":"Hello!"}]}"#;
        assert_eq!(
            extract_response_text(ApiProtocol::Anthropic, raw).unwrap(),
            "Hello!"
        );
    }

    #[test]
    fn test_extract_openai() {
        let raw = r#"{"choices":[{"message":{"content":"Hi!"}}]}"#;
        assert_eq!(
            extract_response_text(ApiProtocol::OpenAiCompat, raw).unwrap(),
            "Hi!"
        );
    }

    #[test]
    fn test_extract_gemini() {
        let raw = r#"{"candidates":[{"content":{"parts":[{"text":"Hey!"}]}}]}"#;
        assert_eq!(
            extract_response_text(ApiProtocol::Gemini, raw).unwrap(),
            "Hey!"
        );
    }

    #[test]
    fn test_chat_message_builders() {
        let msg = ChatMessage::system("test");
        assert_eq!(msg.role, "system");
        let msg = ChatMessage::user("hello");
        assert_eq!(msg.role, "user");
    }

    #[test]
    fn test_build_tools_body() {
        let config = ApiConfig::new(ApiProvider::Claude, "test".into());
        let messages = vec![StructuredMessage::user(vec![ContentBlock::Text {
            text: "Hello".into(),
        }])];
        let tools = vec![ToolDefinition {
            name: "get_world_overview".into(),
            description: "Get overview of the ECS world".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }];
        let body = build_tools_request_body(&config, &messages, &tools, Some("system prompt"));
        assert!(body["tools"].is_array());
        assert_eq!(body["tools"][0]["name"], "get_world_overview");
        assert_eq!(body["system"], "system prompt");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_build_tools_body_with_image_block() {
        let config = ApiConfig::new(ApiProvider::MiniMax, "test".into());
        let messages = vec![StructuredMessage::user(vec![
            ContentBlock::Text {
                text: "What is in this image?".into(),
            },
            ContentBlock::Image {
                source: ImageSource::Base64 {
                    media_type: "image/png".into(),
                    data: "aGVsbG8=".into(),
                },
                detail: Some("low".into()),
            },
        ])];

        let body = build_tools_request_body(&config, &messages, &[], None);
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aGVsbG8=");
        assert_eq!(content[1]["detail"], "low");
    }

    #[test]
    fn test_parse_tool_use_response() {
        let raw = r#"{
            "content": [
                {"type": "text", "text": "I'll check the world."},
                {"type": "tool_use", "id": "toolu_01", "name": "get_world_overview", "input": {}}
            ],
            "stop_reason": "tool_use"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(resp.content.len(), 2);
        match &resp.content[0] {
            ContentBlock::Text { text } => assert!(text.contains("check")),
            _ => panic!("Expected Text block"),
        }
        match &resp.content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "get_world_overview");
                assert!(input.is_object());
            }
            _ => panic!("Expected ToolUse block"),
        }
    }

    #[test]
    fn test_parse_thinking_response() {
        let raw = r#"{
            "content": [
                {"type": "thinking", "thinking": "Let me analyze...", "signature": "abc123"},
                {"type": "text", "text": "Here is my answer."},
                {"type": "tool_use", "id": "t1", "name": "get_world_overview", "input": {}}
            ],
            "stop_reason": "tool_use"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.content.len(), 3);
        match &resp.content[0] {
            ContentBlock::Thinking { thinking } => assert!(thinking.contains("analyze")),
            other => panic!("Expected Thinking, got {:?}", other),
        }
        match &resp.content[1] {
            ContentBlock::Text { text } => assert!(text.contains("answer")),
            other => panic!("Expected Text, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_unknown_block_type() {
        let raw = r#"{
            "content": [
                {"type": "some_future_type", "data": 42},
                {"type": "text", "text": "Hello"}
            ],
            "stop_reason": "end_turn"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.content.len(), 2);
        assert!(matches!(&resp.content[0], ContentBlock::Unknown(_)));
        assert!(matches!(&resp.content[1], ContentBlock::Text { .. }));
    }

    #[test]
    fn test_parse_end_turn_response() {
        let raw = r#"{
            "content": [{"type": "text", "text": "Done!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 42, "output_tokens": 5}
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(resp.content.len(), 1);
        let usage = resp.usage.unwrap();
        assert_eq!(usage.input_tokens, 42);
        assert_eq!(usage.output_tokens, 5);
    }
}
