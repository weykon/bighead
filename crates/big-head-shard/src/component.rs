//! 分片内容记录 — 存储投影到 ECS 的大段内容块。
//!
//! 按 `paradigm` + `shard_index` 查询，支持按来源和索引检索。

use bevy_ecs::component::Component;

/// 单个投影内容块。
///
/// 一次分片操作产生 n 个 ShardData 实体，每个包含一个内容块。
/// 查询入口：`get_shards(paradigm)` 或 `query_entities component=ShardData`。
#[derive(Component, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ShardData {
    /// 范式标识 — 描述分片来源（如 `tool:execute_cli:abc123`）
    pub paradigm: String,
    /// 当前分片序号 (0..total_shards)
    pub shard_index: u32,
    /// 总分片数
    pub total_shards: u32,
    /// 投影后的内容块
    pub content: String,
    /// 数据来源标签（如 "execute_cli output"）
    pub source_label: String,
    /// 创建时间戳 (unix seconds)
    pub created_at: u64,
}
