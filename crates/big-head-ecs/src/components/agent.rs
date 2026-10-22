//! Agent 实体组件 —— Agent 的标识、配置和运行时状态。
//!
//! 将 Agent 表示为 ECS 世界中的实体，使其可查询、可修改、可持久化。

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// Agent 的标识和配置组件。
///
/// 包含 agent 的角色和 system prompt。修改此组件即可热切换 agent 的
/// 行为——下一个 spark 周期会读取新的 system_prompt。
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Agent 角色标识（如 "coordinator"、"code-reviewer"）。
    pub role: String,
    /// 发送给 LLM 的 system prompt。
    pub system_prompt: String,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            role: String::new(),
            system_prompt: String::new(),
        }
    }
}

/// Agent 的运行时状态组件。
///
/// 每个 spark 周期结束后写回，供持久化和自省。
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// 最近一次推理周期的 wave 数。
    pub current_wave: usize,
    /// 累计工具调用次数。
    pub total_actions: usize,
    /// 每个 spark 周期允许的最大推理迭代轮数。
    pub max_iterations: usize,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            current_wave: 0,
            total_actions: 0,
            max_iterations: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_profile() {
        let p = AgentProfile::default();
        assert!(p.role.is_empty());
        assert!(p.system_prompt.is_empty());
    }

    #[test]
    fn test_default_state() {
        let s = AgentState::default();
        assert_eq!(s.current_wave, 0);
        assert_eq!(s.total_actions, 0);
        assert_eq!(s.max_iterations, 100);
    }

    #[test]
    fn test_agent_state_serde_roundtrip() {
        let state = AgentState {
            current_wave: 5,
            total_actions: 12,
            max_iterations: 50,
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: AgentState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.current_wave, 5);
        assert_eq!(restored.total_actions, 12);
        assert_eq!(restored.max_iterations, 50);
    }
}
