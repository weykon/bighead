//! Capture real tool actions and SystemRegistry definitions in the existing audit.

use crate::{audit, registry::ComponentActionRegistry, system_def::SystemRegistry};
use bevy_ecs::world::World;
use big_head_ai::ToolCall;

pub(super) fn record_tool_action(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    tx_id: &str,
    call: &ToolCall,
    current_wave: usize,
    output: &str,
    is_error: bool,
    entity: Option<String>,
    component: Option<String>,
    before: Option<serde_json::Value>,
    after_override: Option<serde_json::Value>,
) {
    let after = if after_override.is_some() {
        after_override
    } else if is_error {
        None
    } else {
        audit::write_after_value(
            world,
            registry,
            &call.name,
            entity.as_deref(),
            component.as_deref(),
        )
    };
    audit::record_action(
        world,
        registry,
        audit::ActionRecordDraft {
            action_id: audit::new_action_id(),
            tx_id: tx_id.to_string(),
            wave: current_wave as u32,
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            kind: audit::tool_action_kind(&call.name).into(),
            entity,
            component,
            input: call.input.clone(),
            output_summary: audit::short_output(output),
            is_error,
            before,
            after,
            created_at: audit::now_secs(),
        },
    );
}

pub(super) fn system_audit_id<'a>(
    call: &'a ToolCall,
    system_registry: &SystemRegistry,
) -> Option<&'a str> {
    match call.name.as_str() {
        "define_system" | "define_tool" | "run_system" | "update_system" | "remove_system" => {
            call.input.get("id").and_then(|value| value.as_str())
        }
        name if system_registry.get(name).is_some() => Some(name),
        _ => None,
    }
}

pub(super) fn system_definition_json(
    system_registry: &SystemRegistry,
    id: &str,
) -> Option<serde_json::Value> {
    serde_json::to_value(system_registry.get(id)?).ok()
}
