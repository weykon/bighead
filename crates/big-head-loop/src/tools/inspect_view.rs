//! Runtime-owned projection of ECS components into bounded inspector panels.

use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use big_head_ai::{ToolCall, ToolDefinition};
use big_head_view::{
    DisplayIntent, InspectorEntry, InspectorField, InspectorMetric, InspectorStatus, InspectorTone,
    PanelContent, PanelKind, ViewToolHandler, display_intent_for_component, display_spec,
    normalize_view_command_input,
};
use serde_json::{Map, Value, json};

use crate::registry::ComponentActionRegistry;

const DEFAULT_ENTRY_LIMIT: usize = 20;
const MAX_ENTRY_LIMIT: usize = 50;
const MAX_FIELDS_PER_ENTRY: usize = 8;
const MAX_LABEL_CHARS: usize = 72;
const MAX_STRING_CHARS: usize = 160;
const MAX_NESTED_CHARS: usize = 240;

pub(crate) struct InspectViewResult {
    pub output: String,
    pub is_error: bool,
    pub persisted_input: Option<Value>,
}

pub(crate) fn tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "inspect_component_view".into(),
        description: "Read one ECS component directly from the runtime and render a bounded, stable Inspector panel. \
                      Prefer this over query_entities + manually assembling render_view JSON when the user wants to inspect \
                      TickAction, Spark, SessionContext, AmbientSignal, AgentState, ToolHistory, ActionRecord, ViewState, \
                      Plan, WorkItem, or TransactionRecord. Rust owns entity enumeration, status derivation, truncation, \
                      panel identity, and layout metadata. The browser view server must be running.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "component": {
                    "type": "string",
                    "description": "Registered ECS component short name, for example TickAction or Spark."
                },
                "intent": {
                    "type": "string",
                    "enum": ["inspect_tickactions", "inspect_sparks", "inspect_context", "inspect_world", "inspect_tools", "inspect_view", "track_work"],
                    "description": "Optional inspector slot override. Known components resolve automatically; use this only for an extension component."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_ENTRY_LIMIT,
                    "description": "Maximum entities rendered. Defaults to 20 and is always capped at 50."
                }
            },
            "required": ["component"]
        }),
    }
}

pub(crate) fn dispatch(
    world: &World,
    registry: &ComponentActionRegistry,
    handler: &ViewToolHandler,
    call: &ToolCall,
) -> InspectViewResult {
    let component = match call
        .input
        .get("component")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(component) => component,
        None => return InspectViewResult::error("Missing non-empty 'component' field"),
    };

    let intent = match resolve_intent(component, call.input.get("intent")) {
        Ok(intent) => intent,
        Err(error) => return InspectViewResult::error(error),
    };
    let limit = call
        .input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_ENTRY_LIMIT)
        .clamp(1, MAX_ENTRY_LIMIT);

    let (render_input, total, shown) =
        match build_inspector_render_input(world, registry, component, intent, limit) {
            Ok(result) => result,
            Err(error) => return InspectViewResult::error(error),
        };
    let normalized = normalize_view_command_input(render_input);
    let render_call = ToolCall {
        id: call.id.clone(),
        name: "render_view".into(),
        input: normalized.clone(),
    };
    let (output, is_error) = handler.dispatch(&render_call);
    if is_error {
        return InspectViewResult::error(output);
    }

    InspectViewResult {
        output: format!(
            "Rendered {component} inspector: {shown} of {total} entities (fields and nested values bounded by runtime)"
        ),
        is_error: false,
        persisted_input: Some(normalized),
    }
}

impl InspectViewResult {
    fn error(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
            persisted_input: None,
        }
    }
}

fn resolve_intent(component: &str, raw_intent: Option<&Value>) -> Result<DisplayIntent, String> {
    if let Some(raw_intent) = raw_intent {
        let intent: DisplayIntent = serde_json::from_value(raw_intent.clone())
            .map_err(|error| format!("Invalid inspector intent: {error}"))?;
        if display_spec(intent).kind != PanelKind::Inspector {
            return Err(format!(
                "Display intent '{}' is not an inspector intent",
                raw_intent.as_str().unwrap_or("unknown")
            ));
        }
        return Ok(intent);
    }

    display_intent_for_component(component).ok_or_else(|| {
        format!(
            "No built-in display intent for component '{component}'. Pass an explicit inspector intent to use a stable extension slot."
        )
    })
}

fn build_inspector_render_input(
    world: &World,
    registry: &ComponentActionRegistry,
    component: &str,
    intent: DisplayIntent,
    limit: usize,
) -> Result<(Value, usize, usize), String> {
    let component_id = registry
        .lookup_component_id(world, component)
        .ok_or_else(|| format!("Unknown ECS component '{component}'"))?;
    let mut entities = component_entities(world, component_id);
    entities.sort_by_key(|entity| entity.index().index());

    let total = entities.len();
    let entries = entities
        .into_iter()
        .take(limit)
        .map(|entity| {
            let value = read_component_value(world, registry, entity, component_id, component);
            inspector_entry(component, entity, value)
        })
        .collect::<Vec<_>>();
    let shown = entries.len();
    let summary = inspector_summary(total, shown, &entries);
    let content = PanelContent::Inspector {
        component: component.to_string(),
        summary,
        entries,
        empty_message: Some(format!("No {component} entities in the current world")),
    };

    Ok((
        json!({
            "op": "spawn",
            "intent": intent,
            "content": serde_json::to_value(content)
                .map_err(|error| format!("Serialize inspector content: {error}"))?
        }),
        total,
        shown,
    ))
}

fn component_entities(world: &World, component_id: ComponentId) -> Vec<Entity> {
    let mut entities = Vec::new();
    for archetype in world.archetypes().iter() {
        if !archetype.components().contains(&component_id) {
            continue;
        }
        entities.extend(archetype.entities().iter().map(|entity| entity.id()));
    }
    entities
}

fn read_component_value(
    world: &World,
    registry: &ComponentActionRegistry,
    entity: Entity,
    component_id: ComponentId,
    component: &str,
) -> Option<Value> {
    if registry.dynamic_store().is_dynamic(component_id) {
        big_head_ecs::dynamic_component::read(world, entity, component_id)
    } else {
        registry.serialize_rust_component(world, entity, component)
    }
}

fn inspector_entry(component: &str, entity: Entity, value: Option<Value>) -> InspectorEntry {
    let entity_id = u64::from(entity.index().index());
    let status = value
        .as_ref()
        .map(|value| derive_status(component, value))
        .unwrap_or(InspectorStatus::Warning);
    let label = value
        .as_ref()
        .map(|value| derive_label(component, entity_id, value))
        .unwrap_or_else(|| format!("{component} #{entity_id}"));
    let fields = value
        .as_ref()
        .map(|value| inspector_fields(component, value))
        .unwrap_or_else(|| {
            vec![InspectorField {
                label: "value".into(),
                value: Value::String("opaque or serializer unavailable".into()),
            }]
        });

    InspectorEntry {
        entity_id: Some(entity_id),
        label,
        status,
        fields,
    }
}

fn derive_label(component: &str, entity_id: u64, value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return format!("{component} #{entity_id}");
    };

    let candidate = match component {
        "TickAction" => object
            .get("script")
            .and_then(Value::as_str)
            .and_then(first_meaningful_line),
        "Spark" => string_field(object, &["text"]),
        "SessionContext" => string_field(object, &["location"]),
        "AmbientSignal" => string_field(object, &["signal_id", "category"]),
        "ActionRecord" => string_field(object, &["tool", "kind", "action"]),
        "TransactionRecord" => string_field(object, &["summary", "intent", "status"]),
        "WorkItem" | "Plan" => string_field(object, &["title", "label", "summary"]),
        "ViewState" => object
            .get("panels")
            .and_then(Value::as_object)
            .map(|panels| format!("{} panels", panels.len())),
        "AgentState" => object
            .get("current_wave")
            .and_then(Value::as_u64)
            .map(|wave| format!("Agent wave {wave}")),
        _ => string_field(object, &["name", "title", "label", "id"]),
    };

    candidate
        .map(|label| truncate_chars(label.trim(), MAX_LABEL_CHARS))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| format!("{component} #{entity_id}"))
}

fn derive_status(component: &str, value: &Value) -> InspectorStatus {
    let Some(object) = value.as_object() else {
        return InspectorStatus::Healthy;
    };

    if object
        .get("last_error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.trim().is_empty())
        || object.get("is_error").and_then(Value::as_bool) == Some(true)
        || object.get("success").and_then(Value::as_bool) == Some(false)
    {
        return InspectorStatus::Error;
    }

    if let Some(status) = object.get("status").and_then(Value::as_str) {
        return match status.to_ascii_lowercase().as_str() {
            "pending" | "queued" | "draft" => InspectorStatus::Pending,
            "processing" | "running" | "in_progress" | "active" => InspectorStatus::Running,
            "warning" | "blocked" | "stale" => InspectorStatus::Warning,
            "error" | "failed" | "cancelled" => InspectorStatus::Error,
            "done" | "complete" | "completed" | "accepted" => InspectorStatus::Done,
            "healthy" | "ok" | "success" => InspectorStatus::Healthy,
            _ => InspectorStatus::Idle,
        };
    }

    if component == "TickAction" {
        if object.get("enabled").and_then(Value::as_bool) == Some(false) {
            return InspectorStatus::Idle;
        }
        let last_run = object.get("last_run").and_then(Value::as_u64).unwrap_or(0);
        let cooldown = object
            .get("cooldown_secs")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if last_run == 0 || unix_now().saturating_sub(last_run) >= cooldown {
            return InspectorStatus::Pending;
        }
    }

    InspectorStatus::Healthy
}

fn inspector_fields(component: &str, value: &Value) -> Vec<InspectorField> {
    let Some(object) = value.as_object() else {
        return vec![InspectorField {
            label: "value".into(),
            value: bounded_value(value),
        }];
    };

    let mut fields = if component == "TickAction" {
        tick_action_timing_fields(object)
    } else {
        Vec::new()
    };
    let mut keys = Vec::new();
    for preferred in preferred_fields(component) {
        if object.contains_key(*preferred) {
            keys.push((*preferred).to_string());
        }
    }
    let mut remaining = object
        .keys()
        .filter(|key| !keys.contains(key))
        .cloned()
        .collect::<Vec<_>>();
    remaining.sort();
    keys.extend(remaining);

    fields.extend(
        keys.into_iter()
            .take(MAX_FIELDS_PER_ENTRY.saturating_sub(fields.len()))
            .filter_map(|key| {
                object.get(&key).map(|value| InspectorField {
                    label: key,
                    value: bounded_value(value),
                })
            }),
    );
    fields
}

fn tick_action_timing_fields(object: &Map<String, Value>) -> Vec<InspectorField> {
    let last_run = object.get("last_run").and_then(Value::as_u64).unwrap_or(0);
    let cooldown = object
        .get("cooldown_secs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let since = if last_run == 0 {
        None
    } else {
        Some(unix_now().saturating_sub(last_run))
    };
    let remaining = since
        .map(|since| cooldown.saturating_sub(since))
        .unwrap_or(0);

    vec![
        InspectorField {
            label: "since_secs".into(),
            value: since.map(Value::from).unwrap_or(Value::Null),
        },
        InspectorField {
            label: "remaining_secs".into(),
            value: Value::from(remaining),
        },
    ]
}

fn preferred_fields(component: &str) -> &'static [&'static str] {
    match component {
        "TickAction" => &["enabled", "cooldown_secs", "last_run", "script"],
        "Spark" => &["status", "timestamp", "cycle_id", "text", "result_summary"],
        "SessionContext" => &["location", "time_of_day", "light", "weather", "source"],
        "AmbientSignal" => &["signal_id", "category", "intensity", "source", "timestamp"],
        "AgentState" => &["current_wave", "total_actions", "max_iterations"],
        "ActionRecord" => &["tool", "kind", "wave", "is_error", "summary"],
        "TransactionRecord" => &["status", "intent", "summary", "started_at", "finished_at"],
        "ViewState" => &["panels"],
        _ => &[],
    }
}

fn bounded_value(value: &Value) -> Value {
    match value {
        Value::String(value) => Value::String(truncate_chars(value, MAX_STRING_CHARS)),
        Value::Array(_) | Value::Object(_) => Value::String(truncate_chars(
            &serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".into()),
            MAX_NESTED_CHARS,
        )),
        _ => value.clone(),
    }
}

fn inspector_summary(
    total: usize,
    shown: usize,
    entries: &[InspectorEntry],
) -> Vec<InspectorMetric> {
    let pending = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.status,
                InspectorStatus::Pending | InspectorStatus::Running
            )
        })
        .count();
    let issues = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.status,
                InspectorStatus::Warning | InspectorStatus::Error
            )
        })
        .count();

    vec![
        metric("total", total, InspectorTone::Neutral),
        metric("shown", shown, InspectorTone::Info),
        metric("active", pending, InspectorTone::Warning),
        metric(
            "issues",
            issues,
            if issues == 0 {
                InspectorTone::Success
            } else {
                InspectorTone::Danger
            },
        ),
    ]
}

fn metric(label: &str, value: usize, tone: InspectorTone) -> InspectorMetric {
    InspectorMetric {
        label: label.into(),
        value: json!(value),
        tone: Some(tone),
    }
}

fn string_field(object: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_str))
        .map(str::to_string)
}

fn first_meaningful_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('/').trim().to_string())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use big_head_ecs::{DynamicSchema, FieldType};
    use std::collections::BTreeMap;

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestSparkLike {
        status: String,
        text: String,
        nested: Value,
    }

    #[test]
    fn inspector_projection_is_bounded_and_uses_stable_intent() {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        registry.register_rust_component::<TestSparkLike>("TestSparkLike");
        world.spawn(TestSparkLike {
            status: "pending".into(),
            text: "x".repeat(300),
            nested: json!({"payload": "y".repeat(400)}),
        });

        let (input, total, shown) = build_inspector_render_input(
            &world,
            &registry,
            "TestSparkLike",
            DisplayIntent::InspectWorld,
            20,
        )
        .unwrap();
        let normalized = normalize_view_command_input(input);

        assert_eq!(total, 1);
        assert_eq!(shown, 1);
        assert_eq!(normalized["id"], "inspector-world");
        assert_eq!(normalized["kind"], "inspector");
        let fields = normalized["content"]["entries"][0]["fields"]
            .as_array()
            .unwrap();
        assert!(fields.len() <= MAX_FIELDS_PER_ENTRY);
        assert!(fields.iter().all(|field| {
            field["value"]
                .as_str()
                .map(|value| value.chars().count() <= MAX_NESTED_CHARS + 3)
                .unwrap_or(true)
        }));
    }

    #[test]
    fn tick_action_label_uses_script_header_and_pending_status() {
        let value = json!({
            "script": "// weather_query_tickaction\nlet now = timestamp();",
            "last_run": 0,
            "cooldown_secs": 21600
        });
        let entry = inspector_entry("TickAction", Entity::from_raw_u32(7).unwrap(), Some(value));

        assert_eq!(entry.label, "weather_query_tickaction");
        assert_eq!(entry.status, InspectorStatus::Pending);
        assert_eq!(entry.fields[0].label, "since_secs");
        assert_eq!(entry.fields[0].value, Value::Null);
        assert_eq!(entry.fields[1].label, "remaining_secs");
        assert_eq!(entry.fields[1].value, 0);
    }

    #[test]
    fn paused_tick_action_is_idle_and_exposes_enabled_state() {
        let value = json!({
            "enabled": false,
            "script": "// paused_action\nlog(\"no-op\");",
            "last_run": 123,
            "cooldown_secs": 60
        });
        let entry = inspector_entry("TickAction", Entity::from_raw_u32(8).unwrap(), Some(value));

        assert_eq!(entry.status, InspectorStatus::Idle);
        assert!(
            entry
                .fields
                .iter()
                .any(|field| { field.label == "enabled" && field.value == Value::Bool(false) })
        );
    }

    #[test]
    fn dynamic_components_use_the_safe_dynamic_reader() {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        registry
            .define_dynamic_component(
                &mut world,
                "CustomRuntime",
                DynamicSchema {
                    fields: BTreeMap::from([
                        ("name".into(), FieldType::String),
                        ("status".into(), FieldType::String),
                    ]),
                },
            )
            .unwrap();
        let entity = world.spawn_empty().id();
        registry
            .insert_component(
                &mut world,
                entity,
                "CustomRuntime",
                r#"{"name":"scheduler","status":"running"}"#,
            )
            .unwrap();

        let (input, total, shown) = build_inspector_render_input(
            &world,
            &registry,
            "CustomRuntime",
            DisplayIntent::InspectWorld,
            20,
        )
        .unwrap();
        let entry = &input["content"]["entries"][0];

        assert_eq!(total, 1);
        assert_eq!(shown, 1);
        assert_eq!(entry["label"], "scheduler");
        assert_eq!(entry["status"], "running");
    }

    #[test]
    fn unknown_component_requires_an_explicit_extension_slot() {
        assert!(resolve_intent("CustomThing", None).is_err());
        assert_eq!(
            resolve_intent("CustomThing", Some(&json!("inspect_world"))).unwrap(),
            DisplayIntent::InspectWorld
        );
    }
}
