//! [`AgentAction`] —— agent 发出的序列化 action schema，
//! 由运行时的 apply-action 桥接消费。

use serde::{Deserialize, Serialize};

/// AI agent 可以产生以修改世界的 action。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum AgentAction {
    /// 创建带给定组件的新实体。
    SpawnEntity {
        components: Vec<ComponentValue>,
        #[serde(default)]
        description: Option<String>,
    },
    /// 修改已有实体上的组件。
    ModifyComponent {
        entity_id: u64,
        component: String,
        value: serde_json::Value,
    },
    /// 从实体移除组件。
    RemoveComponent { entity_id: u64, component: String },
    /// 彻底销毁一个实体。
    DespawnEntity { entity_id: u64 },
    /// 运行时定义新的组件类型。
    DefineComponent {
        name: String,
        schema: serde_json::Value,
    },
    /// 注销运行时定义的组件类型（软删除，仅从注册表移除名称映射）。
    /// 注意：bevy_ecs 不支持真正的组件注销，只能从我们的 store 移除。
    UndefineComponent { name: String },
    /// 运行时定义新系统（Rhai 脚本或 LLM）。
    DefineSystem {
        id: String,
        description: String,
        /// "rhai" 或 "llm"
        kind: String,
        source: Option<String>,
        tool_names: Vec<String>,
        max_iterations: u32,
        #[serde(default)]
        is_tool: bool,
    },
    /// 按 id 执行已注册的 Rhai 系统。
    ExecuteSystem { id: String, result_summary: String },
    /// A registered Rhai definition was replaced without executing its code.
    UpdateSystem {
        id: String,
        before_hash: String,
        after_hash: String,
    },
    /// A registered Rhai definition was removed; its previous ECS effects remain.
    RemoveSystem { id: String, before_hash: String },
    /// 执行了一个 shell 命令。
    ExecuteCli {
        command: String,
        exit_code: Option<i32>,
        duration_ms: u64,
        timed_out: bool,
    },
    /// 将大段内容投影为 ECS 中的 ShardData 内容块。
    ShardContent {
        paradigm: String,
        strategy: String,
        total_shards: u32,
    },
}

/// 创建实体时使用的组件名 + JSON 值对。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentValue {
    pub name: String,
    pub value: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_define_component_action_roundtrip() {
        let action = AgentAction::DefineComponent {
            name: "Deadline".into(),
            schema: serde_json::json!({"date": "string", "urgent": "boolean"}),
        };
        let json = serde_json::to_string(&action).unwrap();
        let parsed: AgentAction = serde_json::from_str(&json).unwrap();
        match parsed {
            AgentAction::DefineComponent { name, schema } => {
                assert_eq!(name, "Deadline");
                assert_eq!(schema["date"], "string");
                assert_eq!(schema["urgent"], "boolean");
            }
            _ => panic!("expected DefineComponent"),
        }
    }

    #[test]
    fn test_define_system_action_roundtrip() {
        let action = AgentAction::DefineSystem {
            id: "health_check".into(),
            description: "checks health".into(),
            kind: "rhai".into(),
            source: Some("log(\"ok\");".into()),
            tool_names: vec!["get_world_overview".into()],
            max_iterations: 10,
            is_tool: false,
        };
        let json = serde_json::to_string(&action).unwrap();
        let parsed: AgentAction = serde_json::from_str(&json).unwrap();
        match parsed {
            AgentAction::DefineSystem {
                id,
                description,
                kind,
                source,
                max_iterations,
                ..
            } => {
                assert_eq!(id, "health_check");
                assert_eq!(description, "checks health");
                assert_eq!(kind, "rhai");
                assert_eq!(source.as_deref(), Some("log(\"ok\");"));
                assert_eq!(max_iterations, 10);
            }
            _ => panic!("expected DefineSystem"),
        }
    }
}

impl AgentAction {
    /// Human-readable summary for logging.
    pub fn summary(&self) -> String {
        match self {
            AgentAction::SpawnEntity {
                components,
                description,
            } => {
                format!(
                    "Spawn {} with [{}]",
                    description.as_deref().unwrap_or("entity"),
                    components
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            AgentAction::ModifyComponent {
                entity_id,
                component,
                ..
            } => {
                format!("Modify entity {} component '{}'", entity_id, component)
            }
            AgentAction::RemoveComponent {
                entity_id,
                component,
            } => {
                format!("Remove '{}' from entity {}", component, entity_id)
            }
            AgentAction::DespawnEntity { entity_id } => {
                format!("Despawn entity {}", entity_id)
            }
            AgentAction::DefineComponent { name, .. } => {
                format!("Define component '{}'", name)
            }
            AgentAction::UndefineComponent { name } => {
                format!("Undefine component '{}'", name)
            }
            AgentAction::DefineSystem { id, .. } => {
                format!("Define system '{}'", id)
            }
            AgentAction::ExecuteSystem { id, .. } => {
                format!("Execute system '{}'", id)
            }
            AgentAction::UpdateSystem { id, .. } => format!("Update system '{}'", id),
            AgentAction::RemoveSystem { id, .. } => format!("Remove system '{}'", id),
            AgentAction::ExecuteCli { command, .. } => {
                format!("CLI: {}", command.chars().take(50).collect::<String>())
            }
            AgentAction::ShardContent {
                paradigm,
                total_shards,
                ..
            } => {
                format!("Shard '{}' into {} parts", paradigm, total_shards)
            }
        }
    }
}
