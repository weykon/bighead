//! Big-Head 编排循环：ECS 世界 + spark 分发 + LLM agent 工具调用。
//!
//! ## 模块
//!
//! - [`audit`] — Action Log / Transaction 记录。
//! - [`runtime`] — [`AgentRuntime`] 和循环编排。
//! - [`tools`] — [`tools::RuntimeToolExecutor`] 及各工具分发。
//! - [`registry`] — `apply_action` 桥接函数。
//! - [`scripting`] — Rhai 引擎再导出 + 测试。
//! - [`snapshot`] — 世界快照再导出。
//! - [`system_def`] — `coordinator_system()` 工厂函数。
//! - [`persistence`] — 文件持久化再导出。
//! - [`time`] — 时间格式化工具（Unix timestamp → 本地可读格式）。
//!
//! ECS 数据类型从 [`big_head_ecs`] 重新导出。

// Re-export pure-ECS types from the dedicated crate.
pub use big_head_ecs::{
    ActionRecord, AgentProfile, AgentState, DynamicComponentStore, DynamicSchema, FieldType,
    ManagedProcess, Spark, SparkStatus, TickAction, ToolHistory, ToolHistoryEntry,
    TransactionRecord, components, dynamic_component, schema,
};

pub(crate) mod ambient;
pub mod a2a_origin;
pub use a2a_origin::A2aOrigin;
pub mod audit;
mod activity;
pub mod character_demo;
mod character_room;
mod character_scene;
mod scene_world;
mod work_world;
mod context_artifact;
mod dream;
mod experience;
mod experience_observer;
mod conversation_feedback;
mod completion;
mod tool_tracking;
pub use tool_tracking::ToolExecution;
mod user_question;
pub use user_question::UserQuestion;
pub(crate) mod image_input;
pub(crate) mod interaction;
pub mod interpretation;
mod interpretation_method;
mod interpretation_stream;
mod lens_bindings;
mod lens_cache;
mod lens_contract;
mod lens_learning;
mod lens_runtime;
mod laya_spec;
mod laya_workflows;
pub mod laya_shadow;
pub mod observation;
pub mod persistence;
pub mod registry;
pub mod runtime;
pub mod scripting;
mod shard_retention;
pub mod snapshot;
mod steward;
pub mod system_def;
pub mod time;
pub mod tools;
mod world_shape;

pub use observation::TickActionReport;
pub use registry::ComponentActionRegistry;
pub use runtime::{AgentRuntime, CycleResult};
pub use snapshot::{ComponentRegistryEntry, ComponentSnapshot, EntitySnapshot, WorldSnapshot};
pub use system_def::{SystemDefinition, SystemKind, SystemRegistry};
pub use time::{format_local_readable, format_local_timestamp};
