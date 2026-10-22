//! 写入 ECS 工具分发：`spawn_entity`、`modify_component`、
//! `remove_component`、`despawn_entity`。
//!
//! 全部四个通过 `big_head_ai::tool_call_to_action` 构建类型化的
//! [`AgentAction`]，然后调用 [`apply_action`] 通过
//! [`ComponentActionRegistry`] 变更世界。

use bevy_ecs::world::World;
use big_head_ai::{AgentAction, ToolCall};

use crate::registry::{ComponentActionRegistry, apply_action};

/// 执行写入工具调用。成功时返回人类可读的输出消息和已应用的
/// `AgentAction`，供调用方追踪。
pub(crate) fn dispatch_write_action(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    if targets_view_state(call) {
        return (
            "ViewState is managed by the view server. Use render_view to create/update browser panels; do not edit ViewState with modify_component.\n\n\
             Recovery hint for modify_component:\n\
             - To update mind-view, call render_view with {\"op\":\"update\",\"id\":\"mind-view\",\"patch\":{\"content\":{\"kind\":\"html\",\"markup\":\"...\"}}}.\n\
             - To create/replace a 2D panel, call render_view with {\"op\":\"spawn\",\"id\":\"...\",\"surface\":\"html2d\",\"kind\":\"html\",\"content\":{\"kind\":\"html\",\"markup\":\"...\"}}.\n\
             - Keep HTML inside content.markup as a string; do not put HTML tags as JSON fields."
                .to_string(),
            true,
            None,
        );
    }

    match big_head_ai::tool_call_to_action(call) {
        Some(action) => match apply_action(world, &action, registry) {
            Ok(msg) => {
                let output = msg.unwrap_or_else(|| "OK".to_string());
                (output, false, Some(action))
            }
            Err(e) => (e, true, None),
        },
        None => (
            format!("Failed to parse tool call '{}' parameters", call.name),
            true,
            None,
        ),
    }
}

fn targets_view_state(call: &ToolCall) -> bool {
    call.name == "modify_component"
        && call
            .input
            .get("component")
            .and_then(|component| component.as_str())
            .is_some_and(|component| component == "ViewState")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;
    use serde_json::json;

    #[test]
    fn modify_component_rejects_direct_view_state_edit() {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        let call = ToolCall {
            id: "call-1".into(),
            name: "modify_component".into(),
            input: json!({
                "entity_id": 424,
                "component": "ViewState",
                "value": {
                    "$text": "<html></html>"
                }
            }),
        };

        let (output, is_error, action) = dispatch_write_action(&mut world, &mut registry, &call);

        assert!(is_error);
        assert!(action.is_none());
        assert!(output.contains("Use render_view"));
        assert!(output.contains("content.markup"));
    }
}
