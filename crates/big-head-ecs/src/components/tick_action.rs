//! TickAction 组件 —— agent 定义周期或条件触发行为的方式。
//!
//! 每个 tick runtime 会评估所有 TickAction entity 嘅 script，
//! 唔需要外部 prompt 或工具调用。

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// 标记 entity 喺每个 tick 自动跑一段 Rhai 脚本。
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct TickAction {
    /// 是否参与 runtime 自动调度。旧 checkpoint 缺失该字段时默认启用。
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// 每次 tick 跑嘅 Rhai 脚本。
    pub script: String,

    /// 上次跑嘅时间（epoch secs）。0 = 从未跑过。
    pub last_run: u64,

    /// 冷却时间（秒）。启用时 `now - last_run >= cooldown_secs` 才会跑；0 表示每个 runtime tick。
    pub cooldown_secs: u64,
}

impl Default for TickAction {
    fn default() -> Self {
        Self {
            enabled: true,
            script: String::new(),
            last_run: 0,
            cooldown_secs: 60,
        }
    }
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::TickAction;

    #[test]
    fn legacy_serialized_tick_action_defaults_to_enabled() {
        let action: TickAction = serde_json::from_str(
            r#"{"script":"log(\"legacy\");","last_run":0,"cooldown_secs":60}"#,
        )
        .unwrap();
        assert!(action.enabled);
    }
}
