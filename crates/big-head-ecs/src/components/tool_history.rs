//! 工具历史组件 —— 记录每个 agent 实体的工具调用。
//!
//! 记录工具调用及输入/输出摘要，供 GUI 元面板展示最近的工具活动。

use std::collections::VecDeque;
use std::time::SystemTime;

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// 附加在 agent 实体上的滚动工具调用日志。
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolHistory {
    /// 最近的工具调用记录（滑动窗口）。
    pub entries: VecDeque<ToolHistoryEntry>,
    /// 最多保留的条目数。
    pub max_entries: usize,
}

fn default_unix_epoch() -> SystemTime {
    SystemTime::UNIX_EPOCH
}

impl Default for ToolHistory {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries: 100,
        }
    }
}

/// 单次工具调用记录。
#[derive(Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct ToolHistoryEntry {
    /// 被调用的工具名称。
    pub tool_name: String,
    /// 输入参数的缩略描述。
    pub input_summary: String,
    /// 输出/结果的缩略描述。
    pub output_summary: String,
    /// 此次工具调用是否出错。
    pub is_error: bool,
    /// 调用发生的时间。
    #[reflect(ignore, default = "default_unix_epoch")]
    pub timestamp: SystemTime,
    /// 发生在哪个 agent 周期。
    pub cycle_id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let th = ToolHistory::default();
        assert!(th.entries.is_empty());
        assert_eq!(th.max_entries, 100);
    }
}
