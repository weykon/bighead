//! Agent AI 层：Anthropic Tool Use 协议循环、ECS 工具定义、
//! 以及驱动多轮推理的 agent。

pub use ai_api_provider::{
    ApiClient, ApiConfig, ApiProvider, ContentBlock, ImageSource, ProxyConfig, ToolDefinition,
};

pub mod actions;
pub mod agent;
pub mod guides;
pub mod log;
pub mod parsing;
pub mod prompts;
pub mod tool_definitions;
pub mod tool_protocol;

pub use actions::{AgentAction, ComponentValue};
pub use agent::Agent;
pub use guides::all_builtin_guides;
pub use log::{LogEmitter, LogEntry, LogLevel};
pub use parsing::AgentError;
pub use prompts::{COORDINATOR_SYSTEM_PROMPT, TICK_STEWARD_SYSTEM_PROMPT};
pub use tool_definitions::ecs_tool_definitions;
pub use tool_protocol::{
    ContextProjectionRequest, ContextProjectionResult, ReasoningResult, ReasoningTermination,
    ToolCall, ToolCallResult, ToolExecutor, tool_call_to_action,
};
