//! 运行时工具执行器 —— Anthropic Tool Use 协议与 ECS 世界之间的桥接。

use std::collections::VecDeque;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use big_head_ai::{AgentAction, LogEmitter};
use big_head_cli::{CliExecutor, SecurityValidator};
use big_head_shard::Sharder;
use big_head_state::StateHook;
use big_head_view::ViewToolHandler;
use big_head_world::WorldManifest;

use crate::registry::ComponentActionRegistry;
use crate::system_def::SystemRegistry;

mod action_audit;
mod catalog;
mod dispatch;
mod executor;
mod policy;
mod projection;
mod recovery;
mod structured_query;
#[cfg(test)]
mod tests;
mod view_state;

pub(crate) mod brave;
pub(crate) mod cli;
pub(crate) mod define;
mod ecs_capabilities;
pub(crate) mod fetch_url;
pub(crate) mod github;
pub(crate) mod hackernews;
pub(crate) mod helpers;
pub(crate) mod html_utils;
pub(crate) mod http_client;
mod inspect_project;
mod inspect_runtime;
mod inspect_transaction;
pub(crate) mod inspect_view;
pub(crate) mod laya;
pub(crate) mod lens;
pub(crate) mod lens_app;
pub(crate) mod pipeline;
pub(crate) mod provider_health;
pub(crate) mod read;
pub(crate) mod run_system;
pub(crate) mod search_provider;
pub(crate) mod shard;
pub(crate) mod shard_catalog;
pub(crate) mod system;
pub(crate) mod tick_action;
pub(crate) mod unified_search;
pub(crate) mod web_search;
pub(crate) mod wiki;
pub(crate) mod write;

/// 工具执行器，持有对运行时各子系统的可变引用。
/// 利用 Rust 结构体字段独立借用，避免 `RefCell`。
pub(crate) struct RuntimeToolExecutor<'a> {
    pub(crate) world: &'a mut World,
    pub(crate) registry: &'a mut ComponentActionRegistry,
    pub(crate) system_registry: &'a mut SystemRegistry,
    pub(crate) rhai_engine: &'a std::sync::Arc<rhai::Engine>,
    pub(crate) spark_queue: &'a mut VecDeque<Entity>,
    pub(crate) actions_applied: Vec<AgentAction>,
    pub(crate) log_emitter: Option<LogEmitter>,
    pub(crate) state_hook: Option<StateHook>,
    pub(crate) cli_executor: &'a CliExecutor,
    pub(crate) cli_security: &'a SecurityValidator,
    pub(crate) sharder: &'a Sharder,
    /// Optional view handler for `render_view` tool. None = view server not running.
    pub(crate) view_handler: Option<ViewToolHandler>,
    /// Current world manifest for Lens inbox/outbox exchange tools.
    pub(crate) world_manifest: Option<&'a WorldManifest>,
    /// 最近一条工具执行的输出内容（供 shard_content 解析 `__last_tool_result`）。
    pub(crate) last_tool_result: Option<String>,
    /// 工具调用日志：(工具名, 输入摘要, 输出摘要, 是否错误)
    pub(crate) tool_call_log: Vec<(String, String, String, bool)>,
    /// 当前 wave 编号（由 agent 通过 `set_wave()` 设置）。
    pub(crate) current_wave: usize,
    /// Current transaction id for action audit records.
    pub(crate) tx_id: Option<String>,
    /// Cooperative cancellation flag. `agent.reason_with_tools` polls this
    /// each iteration; setting it (e.g. via `AgentRuntime::request_cancel`
    /// from a SIGINT handler) breaks the inner loop at the next iteration.
    pub(crate) cancel_flag: &'a std::sync::atomic::AtomicBool,
    /// Optional capability boundary for pipeline-specific LLM systems.
    pub(crate) allowed_tools: Option<&'a [String]>,
}
