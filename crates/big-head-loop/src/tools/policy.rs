//! Capability and runtime-owned state checks before dispatch.

use super::RuntimeToolExecutor;
use bevy_ecs::world::World;
use big_head_ai::ToolCall;

impl RuntimeToolExecutor<'_> {
    pub(super) fn pipeline_policy_violation(&self, call: &ToolCall) -> Option<String> {
        let protected = |name: &str| matches!(name.rsplit("::").next(), Some("ToolExecution" | "UserQuestion"));
        let writes_receipt = match call.name.as_str() {
            "modify_component" | "remove_component" | "define_component" | "undefine_component" =>
                call.input.get("component").or_else(|| call.input.get("name")).and_then(|v| v.as_str()).is_some_and(protected),
            "spawn_entity" => call.input.get("components").and_then(|v| v.as_array()).is_some_and(|items| items.iter().any(|v| v["name"].as_str().is_some_and(protected))),
            "despawn_entity" => call.input["entity_id"].as_u64().and_then(|id| big_head_ecs::resolve_entity(self.world, id).ok()).is_some_and(|e| self.world.get::<crate::tool_tracking::ToolExecution>(e).is_some() || self.world.get::<crate::UserQuestion>(e).is_some()),
            _ => false,
        };
        if writes_receipt { return Some("ToolExecution and UserQuestion are runtime-owned receipts; user answers can only come from the interactive UI.".into()); }
        let activity_name = |name: &str| name.rsplit("::").next() == Some("ActivityRun");
        let writes_activity = match call.name.as_str() {
            "modify_component" | "remove_component" => call
                .input
                .get("component")
                .and_then(|v| v.as_str())
                .is_some_and(activity_name),
            "define_component" | "undefine_component" => call
                .input
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(activity_name),
            "spawn_entity" => call
                .input
                .get("components")
                .and_then(|v| v.as_array())
                .is_some_and(|items| {
                    items.iter().any(|c| {
                        c.get("name")
                            .and_then(|v| v.as_str())
                            .is_some_and(activity_name)
                    })
                }),
            "despawn_entity" => call
                .input
                .get("entity_id")
                .and_then(|v| v.as_u64())
                .and_then(|id| big_head_ecs::resolve_entity(self.world, id).ok())
                .is_some_and(|e| self.world.get::<crate::activity::ActivityRun>(e).is_some()),
            _ => false,
        };
        if writes_activity {
            return Some("ActivityRun is runtime-owned; use create_activity/update_activity_plan/control_activity/inspect_activity".into());
        }
        let is_a2a_origin = |name: &str| name.rsplit("::").next() == Some("A2aOrigin");
        let writes_a2a_origin = match call.name.as_str() {
            "modify_component" | "remove_component" | "define_component" | "undefine_component" => {
                call.input
                    .get(
                        if call.name == "define_component" || call.name == "undefine_component" {
                            "name"
                        } else {
                            "component"
                        },
                    )
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_a2a_origin)
            }
            "spawn_entity" => call
                .input
                .get("components")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|components| {
                    components.iter().any(|component| {
                        component
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(is_a2a_origin)
                    })
                }),
            "despawn_entity" => call
                .input
                .get("entity_id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| big_head_ecs::resolve_entity(self.world, id).ok())
                .is_some_and(|entity| self.world.get::<crate::A2aOrigin>(entity).is_some()),
            _ => false,
        };
        if writes_a2a_origin {
            return Some("A2aOrigin is runtime-owned message provenance".into());
        }
        if (call.name == "define_tool"
            || (call.name == "define_system"
                && call
                    .input
                    .get("is_tool")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)))
            && call
                .input
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| {
                    big_head_ai::ecs_tool_definitions()
                        .into_iter()
                        .chain(self.runtime_tool_definitions())
                        .any(|tool| tool.name == id)
                })
        {
            return Some("system id collides with a runtime tool name".into());
        }
        let is_dream = |name: &str| name.rsplit("::").next() == Some("DreamExperiment");
        let writes_dream = match call.name.as_str() {
            "modify_component" | "remove_component" => call
                .input
                .get("component")
                .and_then(serde_json::Value::as_str)
                .is_some_and(is_dream),
            "define_component" => call
                .input
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(is_dream),
            "spawn_entity" => call
                .input
                .get("components")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|components| {
                    components.iter().any(|c| {
                        c.get("name")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(is_dream)
                    })
                }),
            _ => false,
        };
        if writes_dream {
            return Some(
                "DreamExperiment is runtime-owned evidence; use dream_task / dream_policy".into(),
            );
        }
        if writes_interpretation_state(self.world, call) {
            return Some(
                "Interpretation evidence is runtime-owned; use record_fact, record_interpretation, or materialize_interpretation"
                    .into(),
            );
        }
        let allowed = self.allowed_tools?;
        if !allowed.iter().any(|name| name == &call.name) {
            return Some(format!(
                "Tool '{}' is outside this pipeline system's capability set",
                call.name
            ));
        }
        if tool_call_mutates_tick_action(self.world, call) {
            return Some(
                "Restricted pipeline systems must use update_tick_action for TickAction scripts"
                    .into(),
            );
        }
        let reserved = |name: &str| {
            matches!(
                name.rsplit("::").next().unwrap_or(name),
                "Spark"
                    | "RuntimeIdentity"
                    | "RuntimeJournal"
                    | "SparkDispatch"
                    | "ActionRecord"
                    | "TransactionRecord"
                    | "FactRecord"
                    | "InterpretationMethod"
                    | "InterpretationFrame"
                    | "InterpretationRun"
                    | "InterpretationResult"
                    | "RelationAssertion"
                    | "MaterializationState"
                    | "InterpretationStream"
                    | "InterpretationObservation"
                    | "InteractionExperience"
                    | "ExperienceReading"
            )
        };
        let writes_reserved = match call.name.as_str() {
            "modify_component" | "remove_component" => call
                .input
                .get("component")
                .and_then(|v| v.as_str())
                .is_some_and(reserved),
            "define_component" => call
                .input
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(reserved),
            "spawn_entity" => call
                .input
                .get("components")
                .and_then(|v| v.as_array())
                .is_some_and(|components| {
                    components
                        .iter()
                        .any(|c| c.get("name").and_then(|v| v.as_str()).is_some_and(reserved))
                }),
            _ => false,
        };
        if writes_reserved {
            return Some("Observation stages cannot edit runtime delivery or audit records".into());
        }
        None
    }
}

fn writes_interpretation_state(world: &World, call: &ToolCall) -> bool {
    let reserved = |name: &str| {
        matches!(
            name.rsplit("::").next().unwrap_or(name),
            "FactRecord"
                | "InterpretationMethod"
                | "InterpretationFrame"
                | "InterpretationRun"
                | "InterpretationResult"
                | "RelationAssertion"
                | "MaterializationState"
                | "InterpretationStream"
                | "InterpretationObservation"
                | "InteractionExperience"
                | "ExperienceReading"
        )
    };
    match call.name.as_str() {
        "modify_component" | "remove_component" => call
            .input
            .get("component")
            .and_then(serde_json::Value::as_str)
            .is_some_and(reserved),
        "define_component" | "undefine_component" => call
            .input
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(reserved),
        "spawn_entity" => call
            .input
            .get("components")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|components| {
                components.iter().any(|component| {
                    component
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(reserved)
                })
            }),
        "despawn_entity" => call
            .input
            .get("entity_id")
            .and_then(serde_json::Value::as_u64)
            .and_then(|entity_id| big_head_ecs::resolve_entity(world, entity_id).ok())
            .is_some_and(|entity| {
                world
                    .get::<crate::interpretation::FactRecord>(entity)
                    .is_some()
                    || world
                        .get::<crate::interpretation_method::InterpretationMethod>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation::InterpretationFrame>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation::InterpretationRun>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation::InterpretationResult>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation::RelationAssertion>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation::MaterializationState>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation_stream::InterpretationStream>(entity)
                        .is_some()
                    || world
                        .get::<crate::interpretation_stream::InterpretationObservation>(entity)
                        .is_some()
                    || world
                        .get::<crate::experience::InteractionExperience>(entity)
                        .is_some()
                    || world
                        .get::<crate::experience::ExperienceReading>(entity)
                        .is_some()
            }),
        _ => false,
    }
}

fn tool_call_mutates_tick_action(world: &World, call: &ToolCall) -> bool {
    match call.name.as_str() {
        "modify_component" | "remove_component" => {
            call.input
                .get("component")
                .and_then(serde_json::Value::as_str)
                == Some("TickAction")
        }
        "spawn_entity" => call
            .input
            .get("components")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|components| {
                components.iter().any(|component| {
                    component.get("name").and_then(serde_json::Value::as_str) == Some("TickAction")
                })
            }),
        "despawn_entity" => call
            .input
            .get("entity_id")
            .and_then(serde_json::Value::as_u64)
            .and_then(|entity_id| big_head_ecs::resolve_entity(world, entity_id).ok())
            .is_some_and(|entity| world.get::<big_head_ecs::TickAction>(entity).is_some()),
        _ => false,
    }
}
