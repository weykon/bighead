//! `define_component` 工具分发 —— 向 [`ComponentActionRegistry`]
//! 注册新的运行时组件 schema。

use bevy_ecs::world::World;
use big_head_ai::{AgentAction, ToolCall};

use crate::dynamic_component;
use crate::registry::ComponentActionRegistry;

/// 分发 `define_component` 工具。返回 `(output, is_error, action)`。
/// 成功时 `action` 为 `Some`，供调用方记录到 `actions_applied`。
pub(crate) fn dispatch_define_component(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let name = call
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return (
            "define_component: missing 'name' parameter".to_string(),
            true,
            None,
        );
    }
    let Some(schema_value) = call.input.get("schema").cloned() else {
        return (
            "define_component: missing 'schema' parameter; use an explicit {} for a marker component".to_string(),
            true,
            None,
        );
    };
    match dynamic_component::parse_schema_from_json(&schema_value) {
        Err(e) => (
            format!("define_component: invalid schema: {}", e),
            true,
            None,
        ),
        Ok(schema) => match registry.define_dynamic_component(world, &name, schema) {
            Ok(_cid) => {
                let action = AgentAction::DefineComponent {
                    name: name.clone(),
                    schema: schema_value,
                };
                (
                    format!(
                        "Defined dynamic component '{}'. Use it in spawn_entity / \
                         modify_component / get_entity just like built-in components.",
                        name
                    ),
                    false,
                    Some(action),
                )
            }
            Err(e) => (format!("define_component failed: {}", e), true, None),
        },
    }
}

/// 分发 `undefine_component` 工具。软删除运行时定义的组件类型。
pub(crate) fn dispatch_undefine_component(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let name = call
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if name.is_empty() {
        return (
            "undefine_component: missing 'name' parameter".to_string(),
            true,
            None,
        );
    }

    match registry.undefine_dynamic_component(world, &name) {
        Ok((id, schema)) => {
            let action = AgentAction::UndefineComponent { name: name.clone() };
            (
                format!(
                    "Undefined dynamic component '{}' (ComponentId: {:?}). \
                     The component is no longer discoverable. \
                     Note: bevy_ecs retains the underlying registration (this is a soft delete).",
                    name, id
                ),
                false,
                Some(action),
            )
        }
        Err(e) => (format!("undefine_component failed: {}", e), true, None),
    }
}
