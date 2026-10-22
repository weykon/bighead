//! 多供应商 LLM API 客户端。
//!
//! 支持 24 个供应商，覆盖 3 种线协议：
//! - Anthropic Messages API
//! - OpenAI Chat Completions（覆盖大多数供应商）
//! - Google Gemini generateContent

mod client;
mod error;
mod protocol;
mod provider;
pub mod proxy;
pub mod tokens;

pub use client::{
    resolve_api_key, AnthropicResponse, ApiClient, ApiConfig, ChatMessage, ContentBlock,
    ImageSource, QueueConfig, RetryConfig, StructuredMessage, ToolDefinition, Usage,
};
pub use error::ApiError;
pub use protocol::ApiProtocol;
pub use provider::{provider_by_name, provider_meta, ApiProvider, ProviderMeta, PROVIDERS};
pub use proxy::ProxyConfig;
pub use tokens::{estimate_tokens, model_limits, BudgetStatus, ModelLimits, TokenBudget};
