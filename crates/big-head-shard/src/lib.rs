//! 上下文分片 crate —— 将大内容切分并投影到 ECS。
//!
//! 提供:
//! - [`ShardData`] — ECS 组件，存储单个内容分片
//! - [`Sharder`] — 切分和分片数计算
//! - [`ShardStrategy`] — Agent 指定的切分策略

mod component;
mod sharder;

pub use component::ShardData;
pub use sharder::{ShardConfig, ShardStrategy, Sharder};
