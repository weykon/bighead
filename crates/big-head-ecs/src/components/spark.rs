//! Spark 组件 —— 每个 spark 对应一个 ECS 实体。
//!
//! Agent 可以通过 query_entities 查看所有 spark，通过 modify_component 修改状态。

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// 每个 spark 实体上的核心组件。
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct Spark {
    /// spark 文本内容。
    pub text: String,
    /// 创建时的 epoch 秒。
    pub timestamp: u64,
    /// 当前状态。
    pub status: SparkStatus,
    /// 处理的 cycle 编号。
    pub cycle_id: u64,
    /// agent 的最终回复摘要。
    pub result_summary: String,
    /// Optional AmbientSignal.signal_id attached to this spark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient_signal_ref: Option<String>,
}

impl Default for Spark {
    fn default() -> Self {
        Self {
            text: String::new(),
            timestamp: 0,
            status: SparkStatus::Pending,
            cycle_id: 0,
            result_summary: String::new(),
            ambient_signal_ref: None,
        }
    }
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SparkStatus {
    Pending,
    Processing,
    Done,
    Failed,
    Interrupted,
    Blocked,
}
