//! Guarded TickAction updates for stewardship agents.

use std::sync::Arc;

use bevy_ecs::world::World;
use big_head_ai::{AgentAction, ToolCall};
use big_head_ecs::{TickAction, resolve_entity};
use rhai::Engine;
use serde_json::{Map, Value};

use crate::observation::stable_script_hash;
use crate::scripting;

pub(crate) fn dispatch_update_tick_action(
    world: &mut World,
    rhai_engine: &Arc<Engine>,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    match update_tick_action(world, rhai_engine, &call.input) {
        Ok((output, action)) => (output, false, Some(action)),
        Err(error) => (format!("update_tick_action: {error}"), true, None),
    }
}

fn update_tick_action(
    world: &mut World,
    rhai_engine: &Engine,
    input: &Value,
) -> Result<(String, AgentAction), String> {
    let entity_id = input
        .get("entity_id")
        .and_then(Value::as_u64)
        .ok_or("missing integer entity_id")?;
    let expected_hash = input
        .get("expected_script_hash")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("missing expected_script_hash")?;
    let replacement_script = input.get("script").map(|value| {
        value
            .as_str()
            .filter(|script| !script.trim().is_empty())
            .ok_or("script must be a non-empty string")
    });
    let replacement_script = replacement_script.transpose()?;
    let reason = input
        .get("reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("reason must be a non-empty string")?;
    let cooldown_secs = input.get("cooldown_secs").map(|value| {
        value
            .as_u64()
            .ok_or("cooldown_secs must be an unsigned integer")
    });
    let cooldown_secs = cooldown_secs.transpose()?;
    let enabled = input
        .get("enabled")
        .map(|value| value.as_bool().ok_or("enabled must be a boolean"))
        .transpose()?;

    let entity = resolve_entity(world, entity_id)?;
    let current = world
        .get::<TickAction>(entity)
        .cloned()
        .ok_or_else(|| format!("entity {entity_id} has no TickAction"))?;
    let current_hash = stable_script_hash(&current.script);
    if current_hash != expected_hash {
        return Err(format!(
            "script changed concurrently: expected {expected_hash}, current {current_hash}; inspect the entity again"
        ));
    }
    let next_cooldown = cooldown_secs.unwrap_or(current.cooldown_secs);
    let next_enabled = enabled.unwrap_or(current.enabled);
    let next_script = replacement_script.unwrap_or(&current.script);
    if replacement_script.is_some() {
        scripting::check_syntax(rhai_engine, next_script)
            .map_err(|error| format!("Rhai syntax validation failed: {error}"))?;
    }
    let next_hash = stable_script_hash(next_script);
    let Some(mut action) = world.get_mut::<TickAction>(entity) else {
        return Err(format!("entity {entity_id} disappeared before update"));
    };
    action.script = next_script.to_string();
    action.cooldown_secs = next_cooldown;
    action.enabled = next_enabled;

    let mut value = Map::new();
    value.insert("script".into(), Value::String(next_script.to_string()));
    value.insert("cooldown_secs".into(), Value::from(next_cooldown));
    value.insert("enabled".into(), Value::Bool(next_enabled));
    let agent_action = AgentAction::ModifyComponent {
        entity_id,
        component: "TickAction".into(),
        value: Value::Object(value),
    };
    Ok((
        format!(
            "Updated TickAction {entity_id}: {current_hash} -> {next_hash}; enabled={}; cooldown={}s; reason={}",
            next_enabled,
            next_cooldown,
            reason.trim()
        ),
        agent_action,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::build_ecs_engine;

    #[test]
    fn validates_hash_and_syntax_before_mutating() {
        let mut world = World::new();
        let entity = world
            .spawn(TickAction {
                enabled: true,
                script: "log(\"old\");".into(),
                last_run: 17,
                cooldown_secs: 60,
            })
            .id();
        let engine = build_ecs_engine();
        let old_hash = stable_script_hash("log(\"old\");");

        let (output, _) = update_tick_action(
            &mut world,
            &engine,
            &serde_json::json!({
                "entity_id": entity.index().index(),
                "expected_script_hash": old_hash,
                "script": "log(\"fixed\");",
                "cooldown_secs": 120,
                "reason": "replace failing helper"
            }),
        )
        .expect("valid update");

        let updated = world.get::<TickAction>(entity).unwrap();
        assert_eq!(updated.script, "log(\"fixed\");");
        assert_eq!(updated.cooldown_secs, 120);
        assert!(updated.enabled);
        assert_eq!(updated.last_run, 17);
        assert!(output.contains("replace failing helper"));

        let error = update_tick_action(
            &mut world,
            &engine,
            &serde_json::json!({
                "entity_id": entity.index().index(),
                "expected_script_hash": "stale",
                "script": "log(\"bad\");",
                "reason": "stale edit"
            }),
        )
        .unwrap_err();
        assert!(error.contains("changed concurrently"));
        assert_eq!(
            world.get::<TickAction>(entity).unwrap().script,
            "log(\"fixed\");"
        );

        let current_hash = stable_script_hash("log(\"fixed\");");
        let (output, _) = update_tick_action(
            &mut world,
            &engine,
            &serde_json::json!({
                "entity_id": entity.index().index(),
                "expected_script_hash": current_hash,
                "script": "log(\"fixed\");",
                "enabled": false,
                "reason": "pause until the dependency is repaired"
            }),
        )
        .expect("pause update");
        assert!(output.contains("enabled=false"));
        assert!(!world.get::<TickAction>(entity).unwrap().enabled);

        let (output, _) = update_tick_action(
            &mut world,
            &engine,
            &serde_json::json!({
                "entity_id": entity.index().index(),
                "expected_script_hash": current_hash,
                "enabled": true,
                "reason": "resume the existing action"
            }),
        )
        .expect("resume without replacing script");
        assert!(output.contains("enabled=true"));
        let resumed = world.get::<TickAction>(entity).unwrap();
        assert!(resumed.enabled);
        assert_eq!(resumed.script, "log(\"fixed\");");
    }

    #[test]
    fn rejects_invalid_rhai_without_mutating() {
        let mut world = World::new();
        let entity = world
            .spawn(TickAction {
                script: "log(\"stable\");".into(),
                ..Default::default()
            })
            .id();
        let original_hash = stable_script_hash("log(\"stable\");");
        let error = update_tick_action(
            &mut world,
            &build_ecs_engine(),
            &serde_json::json!({
                "entity_id": entity.index().index(),
                "expected_script_hash": original_hash,
                "script": "let = ;",
                "reason": "invalid test"
            }),
        )
        .unwrap_err();
        assert!(error.contains("syntax validation"));
        assert_eq!(
            world.get::<TickAction>(entity).unwrap().script,
            "log(\"stable\");"
        );
    }
}
