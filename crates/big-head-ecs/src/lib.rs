//! Big-Head 的纯 ECS 数据类型和动态组件操作。
//!
//! 本 crate 包含 ECS 层原语、组件注册表、系统定义和辅助函数，
//! 不引入 AI/agent 编排依赖。
//!
//! ## 模块
//!
//! - [`schema`] — [`DynamicSchema`]、[`FieldType`]、[`DynamicComponentStore`]。
//! - [`dynamic_component`] — 基于 unsafe Bevy `ComponentDescriptor` 的运行时组件。
//! - [`components`] — ECS 原生组件（AgentProfile、AgentState、ToolHistory）。
//! - [`helpers`] — 实体解析、JSON 解包、动态 reader 等共享工具。

pub mod components;
pub mod dynamic_component;
pub mod execution;
pub mod helpers;
pub mod lens;
pub mod registry;
pub mod schema;
pub mod system_def;

pub use components::{
    ActionRecord, AgentProfile, AgentState, ManagedProcess, Spark, SparkStatus, TickAction,
    ToolHistory, ToolHistoryEntry, TransactionRecord,
};
pub use helpers::{query_reader, resolve_entity, snapshot_reader, unwrap_stringified_json};
pub use registry::{ComponentActionRegistry, ComponentHandler};
pub use schema::{DynamicComponentStore, DynamicSchema, FieldType};
pub use system_def::{SystemDefinition, SystemKind, SystemRegistry};
