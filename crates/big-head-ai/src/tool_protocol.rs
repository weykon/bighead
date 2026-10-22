//! Tool-use 协议类型：[`ToolCall`]、[`ToolCallResult`]、
//! [`ToolExecutor`] trait，以及将写入工具调用转换为 [`AgentAction`]
//! 以追踪的 [`tool_call_to_action`] 桥接。

use ai_api_provider::ToolDefinition;
use async_trait::async_trait;

use crate::actions::{AgentAction, ComponentValue};

/// 从 AI 响应中提取的工具调用。
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// 工具调用执行的结果。
pub struct ToolCallResult {
    pub tool_use_id: String,
    pub output: String,
    pub is_error: bool,
}

/// API 出口投影请求。由 agent loop 在发送模型请求前创建，
/// 由 runtime/tool executor 负责落到 ECS。
#[derive(Debug, Clone)]
pub struct ContextProjectionRequest {
    pub source_kind: String,
    pub source_label: String,
    pub stable_id: String,
    pub content: String,
}

/// API 出口投影结果。返回给 agent loop 用于生成模型可见 notice。
#[derive(Debug, Clone)]
pub struct ContextProjectionResult {
    pub paradigm: String,
    pub component: String,
    pub total_shards: u32,
    pub original_bytes: usize,
    pub preview: String,
}

/// 在世界上执行工具调用的 trait，由运行时实现。
#[async_trait]
pub trait ToolExecutor: Send {
    async fn execute(&mut self, call: &ToolCall) -> ToolCallResult;

    fn waiting_for_user(&mut self) -> bool { false }

    /// Pipeline stages may require a verifiable result before accepting completion.
    fn validate_final_response(&mut self, _text: &str) -> Result<(), String> {
        Ok(())
    }

    /// 返回动态定义的工具（如 Rhai 定义的工具），供 LLM 调用。
    /// 默认返回空 —— 支持动态工具的实现应覆盖此方法。
    fn dynamic_tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    /// 将即将发给模型的大块上下文投影到 runtime 持有的世界中。
    fn project_context_content(
        &mut self,
        _request: ContextProjectionRequest,
    ) -> Result<ContextProjectionResult, String> {
        Err("context projection is not supported by this executor".into())
    }

    /// 设置当前 wave 编号。agent 每轮循环调用一次，用于 ToolCall 状态发射。
    fn set_wave(&mut self, _wave: usize) {}

    /// 推一个 view command 到 external view channel.
    /// 默认 no-op —— `big-head-loop` 实现会覆盖, 推到 `ViewChannel`.
    /// Input 形状: `ViewCommand` (JSON) from `big-head-view` crate.
    /// 用 raw `serde_json::Value` 而非 `ViewCommand` 类型, 避免
    /// `big-head-ai` 强依赖 `big-head-view` (会引入循环).
    fn render_view(&mut self, _view: serde_json::Value) {}

    /// Cooperative cancellation check. Called by the agent's inner
    /// reasoning loop between iterations; returning `true` causes
    /// `reason_with_tools` to break out early (no API call, no tool
    /// execution). Default `false` — runtime implementations that own
    /// a cancel flag should override.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// 多轮推理会话的结果。
pub struct ReasoningResult {
    pub final_text: Option<String>,
    pub iterations: usize,
    /// 本次会话中成功执行的操作（工具调用）总数。
    pub total_actions: usize,
    pub termination: ReasoningTermination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningTermination {
    AwaitingInput,
    Incomplete,
    Completed,
    Converged,
    IterationLimit,
    Cancelled,
    OutputLimit,
}

/// 将写入工具调用转换为 AgentAction 以追踪。读取工具返回 None。
pub fn tool_call_to_action(call: &ToolCall) -> Option<AgentAction> {
    match call.name.as_str() {
        "spawn_entity" => {
            let components: Vec<ComponentValue> = call
                .input
                .get("components")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            Some(ComponentValue {
                                name: c.get("name")?.as_str()?.to_string(),
                                value: c.get("value")?.clone(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let description = call
                .input
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(AgentAction::SpawnEntity {
                components,
                description,
            })
        }
        "modify_component" => {
            let entity_id = call.input.get("entity_id")?.as_u64()?;
            let component = call.input.get("component")?.as_str()?.to_string();
            let value = call.input.get("value")?.clone();
            Some(AgentAction::ModifyComponent {
                entity_id,
                component,
                value,
            })
        }
        "remove_component" => {
            let entity_id = call.input.get("entity_id")?.as_u64()?;
            let component = call.input.get("component")?.as_str()?.to_string();
            Some(AgentAction::RemoveComponent {
                entity_id,
                component,
            })
        }
        "despawn_entity" => {
            let entity_id = call.input.get("entity_id")?.as_u64()?;
            Some(AgentAction::DespawnEntity { entity_id })
        }
        "define_component" => {
            let name = call.input.get("name")?.as_str()?.to_string();
            let schema = call.input.get("schema")?.clone();
            Some(AgentAction::DefineComponent { name, schema })
        }
        "undefine_component" => {
            let name = call.input.get("name")?.as_str()?.to_string();
            Some(AgentAction::UndefineComponent { name })
        }
        _ => None, // 读取工具不是 action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_to_action_spawn() {
        let call = ToolCall {
            id: "test".into(),
            name: "spawn_entity".into(),
            input: serde_json::json!({
                "components": [{"name": "Health", "value": 100}],
                "description": "A warrior"
            }),
        };
        let action = tool_call_to_action(&call).unwrap();
        match action {
            AgentAction::SpawnEntity {
                components,
                description,
            } => {
                assert_eq!(components[0].name, "Health");
                assert_eq!(description.as_deref(), Some("A warrior"));
            }
            _ => panic!("Expected SpawnEntity"),
        }
    }

    #[test]
    fn test_tool_call_to_action_modify() {
        let call = ToolCall {
            id: "test".into(),
            name: "modify_component".into(),
            input: serde_json::json!({
                "entity_id": 5,
                "component": "Session",
                "value": {"status": "running"}
            }),
        };
        let action = tool_call_to_action(&call).unwrap();
        match action {
            AgentAction::ModifyComponent {
                entity_id,
                component,
                ..
            } => {
                assert_eq!(entity_id, 5);
                assert_eq!(component, "Session");
            }
            _ => panic!("Expected ModifyComponent"),
        }
    }

    #[test]
    fn test_tool_call_to_action_read_returns_none() {
        let call = ToolCall {
            id: "test".into(),
            name: "get_world_overview".into(),
            input: serde_json::json!({}),
        };
        assert!(tool_call_to_action(&call).is_none());
    }

    #[test]
    fn test_tool_call_to_action_despawn() {
        let call = ToolCall {
            id: "test".into(),
            name: "despawn_entity".into(),
            input: serde_json::json!({"entity_id": 3}),
        };
        let action = tool_call_to_action(&call).unwrap();
        assert!(matches!(
            action,
            AgentAction::DespawnEntity { entity_id: 3 }
        ));
    }

    #[test]
    fn test_tool_call_to_action_remove() {
        let call = ToolCall {
            id: "test".into(),
            name: "remove_component".into(),
            input: serde_json::json!({"entity_id": 2, "component": "Active"}),
        };
        let action = tool_call_to_action(&call).unwrap();
        match action {
            AgentAction::RemoveComponent {
                entity_id,
                component,
            } => {
                assert_eq!(entity_id, 2);
                assert_eq!(component, "Active");
            }
            _ => panic!("Expected RemoveComponent"),
        }
    }

    #[test]
    fn test_tool_call_to_action_define_component() {
        let call = ToolCall {
            id: "test".into(),
            name: "define_component".into(),
            input: serde_json::json!({
                "name": "Deadline",
                "schema": {"date": "string", "urgent": "boolean"}
            }),
        };
        let action = tool_call_to_action(&call).unwrap();
        match action {
            AgentAction::DefineComponent { name, schema } => {
                assert_eq!(name, "Deadline");
                assert_eq!(schema["date"], "string");
            }
            _ => panic!("Expected DefineComponent"),
        }
    }
}
