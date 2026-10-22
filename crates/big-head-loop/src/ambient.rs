use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use big_head_ai::ToolCall;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::dynamic_component;
use crate::registry::ComponentActionRegistry;
use crate::schema::{DynamicSchema, FieldType};
use crate::time::format_local_timestamp;

pub(crate) const SESSION_CONTEXT: &str = "SessionContext";
pub(crate) const AMBIENT_SIGNAL: &str = "AmbientSignal";
pub(crate) const GLOSSARY: &str = "Glossary";
pub(crate) const INSIGHT_STATUS: &str = "InsightStatus";

pub(crate) fn register_ambient_dynamic_components(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
) {
    define_if_missing(world, registry, SESSION_CONTEXT, session_context_schema());
    define_if_missing(world, registry, AMBIENT_SIGNAL, ambient_signal_schema());
    define_if_missing(world, registry, GLOSSARY, glossary_schema());
    define_if_missing(world, registry, INSIGHT_STATUS, insight_status_schema());
}

pub(crate) fn seed_ambient_glossary(world: &mut World, registry: &mut ComponentActionRegistry) {
    if registry.dynamic_store().get_id(GLOSSARY).is_none() {
        return;
    }
    if !entities_with_dynamic(world, registry, GLOSSARY).is_empty() {
        return;
    }

    let now = now_secs();
    let introduced = format_local_timestamp(now, 8);
    for (term, definition, category, related) in [
        (
            "ambient",
            "Design field for sparse environmental cues, explicit ambient events, and future co-presence.",
            "design",
            "insight-ambient-dwelling",
        ),
        (
            "commensal co-existence",
            "AI and ECS share the same world state without forcing every interaction into direct command-response UI.",
            "meta",
            "insight-ambient-dwelling,insight-ai-ecs-coexistence",
        ),
        (
            "cross-axis bridge",
            "A relation that connects two conceptual axes, such as time and ambient space.",
            "technical",
            "insight-ambient-dwelling",
        ),
        (
            "crystallize",
            "Insight lifecycle step where a loose idea becomes structured enough to drive implementation.",
            "meta",
            "insight-ambient-dwelling",
        ),
        (
            "sparse but meaningful",
            "Ambient principle: emit signals only for meaningful shifts, not every low-value tick.",
            "design",
            "insight-ambient-dwelling",
        ),
    ] {
        let entity = world.spawn_empty().id();
        let value = json!({
            "term": term,
            "definition": definition,
            "category": category,
            "first_introduced_at": introduced,
            "related_insight_ids": related
        });
        if let Err(e) = registry.insert_dynamic(world, entity, GLOSSARY, value) {
            log::warn!("ambient glossary seed failed for {term}: {e}");
            if world.get_entity(entity).is_ok() {
                world.despawn(entity);
            }
        }
    }
}

pub(crate) fn ambient_context_block(world: &World, registry: &ComponentActionRegistry) -> String {
    let mut sections = Vec::new();
    if let Some(ctx) = latest_dynamic_value(world, registry, SESSION_CONTEXT) {
        sections.push(format_session_context(&ctx));
    }
    let active_signals = active_ambient_signal_values(world, registry);
    if !active_signals.is_empty() {
        sections.push(format_ambient_signals(&active_signals));
    }

    if sections.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", sections.join("\n\n"))
    }
}

pub(crate) fn ambient_context_for_spark(
    world: &World,
    registry: &ComponentActionRegistry,
    signal_ref: Option<&str>,
) -> String {
    let Some(signal_ref) = signal_ref else {
        return String::new();
    };
    let Some(signal) = ambient_signal_by_ref(world, registry, signal_ref) else {
        return String::new();
    };
    format!("\n\n{}", format_ambient_signals(&[signal]))
}

pub(crate) fn purge_expired_ambient_signals(
    world: &mut World,
    registry: &ComponentActionRegistry,
) -> usize {
    let Some(id) = registry.dynamic_store().get_id(AMBIENT_SIGNAL) else {
        return 0;
    };
    let now = now_secs();
    let expired = entities_with_dynamic(world, registry, AMBIENT_SIGNAL)
        .into_iter()
        .filter(|entity| {
            dynamic_component::read(world, *entity, id)
                .and_then(|v| {
                    v.get("expires_at_epoch")
                        .and_then(|n| n.as_u64())
                        .map(|n| n <= now)
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let count = expired.len();
    for entity in expired {
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
    }
    count
}

pub(crate) fn dispatch_set_session_context(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool) {
    let now = now_secs();
    let mut value = json!({
        "location": string_arg(call, "location", "未设置"),
        "weather": string_arg(call, "weather", "未设置"),
        "time_of_day": string_arg(call, "time_of_day", "未设置"),
        "light": string_arg(call, "light", "未设置"),
        "updated_at": string_arg(call, "updated_at", &format_local_timestamp(now, 8)),
        "updated_at_epoch": now,
        "source": string_arg(call, "source", "user_manual")
    });

    if let Some(source) = value.get("source").and_then(|v| v.as_str()) {
        if !matches!(source, "user_manual" | "preset" | "auto_detected") {
            value["source"] = Value::String("user_manual".into());
        }
    }

    match upsert_singleton_dynamic(world, registry, SESSION_CONTEXT, value) {
        Ok(entity) => (
            format!(
                "SessionContext updated on {:?}. It will be attached as Ambient Cue at the start of future cycles.",
                entity
            ),
            false,
        ),
        Err(e) => (format!("set_session_context failed: {e}"), true),
    }
}

pub(crate) fn dispatch_push_ambient_signal(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    spark_queue: &mut std::collections::VecDeque<Entity>,
    call: &ToolCall,
) -> (String, bool) {
    let category = string_arg(call, "category", "natural");
    if !matches!(
        category.as_str(),
        "natural" | "human" | "technical" | "emotional"
    ) {
        return (
            "push_ambient_signal failed: category must be natural, human, technical, or emotional"
                .into(),
            true,
        );
    }

    let intensity = number_arg(call, "intensity", 0.5).clamp(0.0, 1.0);
    let ttl_secs = integer_arg(call, "ttl_secs", 300).clamp(1, 86_400) as u64;
    let now = now_secs();
    let signal_id = string_arg(call, "signal_id", &format!("ambient-{}", Uuid::new_v4()));
    let source = string_arg(call, "source", "user_action");
    let value = json!({
        "signal_id": signal_id,
        "category": category,
        "wind": string_arg(call, "wind", "静"),
        "birds": string_arg(call, "birds", "安静"),
        "light": string_arg(call, "light", "未设置"),
        "temperature": string_arg(call, "temperature", "未设置"),
        "intensity": intensity,
        "source": source,
        "timestamp": string_arg(call, "timestamp", &format_local_timestamp(now, 8)),
        "ttl_secs": ttl_secs as u32,
        "created_at_epoch": now,
        "expires_at_epoch": now.saturating_add(ttl_secs),
    });

    let signal_entity = world.spawn_empty().id();
    if let Err(e) = registry.insert_dynamic(world, signal_entity, AMBIENT_SIGNAL, value) {
        if world.get_entity(signal_entity).is_ok() {
            world.despawn(signal_entity);
        }
        return (format!("push_ambient_signal failed: {e}"), true);
    }

    let spark_text = string_arg(
        call,
        "description",
        "Ambient signal arrived. Observe the ambient event and decide whether the world state or status surface should change.",
    );
    let spark = crate::components::Spark {
        text: format!("{spark_text}\n\nambient_signal_ref: {signal_id}"),
        timestamp: now,
        status: crate::components::SparkStatus::Pending,
        ambient_signal_ref: Some(signal_id.clone()),
        ..Default::default()
    };
    let spark_entity = big_head_ecs::execution::enqueue(world, spark_queue, spark, false);

    (
        format!(
            "AmbientSignal {} spawned on {:?}; queued spark {:?}",
            signal_id, signal_entity, spark_entity
        ),
        false,
    )
}

fn define_if_missing(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    name: &str,
    schema: DynamicSchema,
) {
    if registry.dynamic_store().resolve_name(name).is_some() {
        return;
    }
    if let Err(e) = registry.define_dynamic_component(world, name, schema) {
        log::warn!("ambient schema '{}' registration skipped: {e}", name);
    }
}

fn session_context_schema() -> DynamicSchema {
    schema([
        ("location", FieldType::String),
        ("weather", FieldType::String),
        ("time_of_day", FieldType::String),
        ("light", FieldType::String),
        ("updated_at", FieldType::String),
        ("updated_at_epoch", FieldType::Integer),
        ("source", FieldType::String),
    ])
}

fn ambient_signal_schema() -> DynamicSchema {
    schema([
        ("signal_id", FieldType::String),
        ("category", FieldType::String),
        ("wind", FieldType::String),
        ("birds", FieldType::String),
        ("light", FieldType::String),
        ("temperature", FieldType::String),
        ("intensity", FieldType::Number),
        ("source", FieldType::String),
        ("timestamp", FieldType::String),
        ("ttl_secs", FieldType::Integer),
        ("created_at_epoch", FieldType::Integer),
        ("expires_at_epoch", FieldType::Integer),
    ])
}

fn glossary_schema() -> DynamicSchema {
    schema([
        ("term", FieldType::String),
        ("definition", FieldType::String),
        ("category", FieldType::String),
        ("first_introduced_at", FieldType::String),
        ("related_insight_ids", FieldType::String),
    ])
}

fn insight_status_schema() -> DynamicSchema {
    schema([("status", FieldType::String)])
}

fn schema(fields: impl IntoIterator<Item = (&'static str, FieldType)>) -> DynamicSchema {
    DynamicSchema {
        fields: fields
            .into_iter()
            .map(|(name, ty)| (name.to_string(), ty))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn upsert_singleton_dynamic(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    component: &str,
    value: Value,
) -> Result<Entity, String> {
    let entity = entities_with_dynamic(world, registry, component)
        .into_iter()
        .next()
        .unwrap_or_else(|| world.spawn_empty().id());
    registry.patch_dynamic(world, entity, component, value)?;
    Ok(entity)
}

fn latest_dynamic_value(
    world: &World,
    registry: &ComponentActionRegistry,
    component: &str,
) -> Option<Value> {
    let id = registry.dynamic_store().get_id(component)?;
    entities_with_dynamic(world, registry, component)
        .into_iter()
        .filter_map(|entity| dynamic_component::read(world, entity, id))
        .max_by_key(|v| {
            v.get("updated_at_epoch")
                .or_else(|| v.get("created_at_epoch"))
                .and_then(|n| n.as_u64())
                .unwrap_or(0)
        })
}

fn active_ambient_signal_values(world: &World, registry: &ComponentActionRegistry) -> Vec<Value> {
    let Some(id) = registry.dynamic_store().get_id(AMBIENT_SIGNAL) else {
        return Vec::new();
    };
    let now = now_secs();
    let mut values = entities_with_dynamic(world, registry, AMBIENT_SIGNAL)
        .into_iter()
        .filter_map(|entity| dynamic_component::read(world, entity, id))
        .filter(|v| {
            v.get("expires_at_epoch")
                .and_then(|n| n.as_u64())
                .map(|expires| expires > now)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|v| {
        v.get("created_at_epoch")
            .and_then(|n| n.as_u64())
            .unwrap_or(0)
    });
    values.into_iter().rev().take(3).collect()
}

fn ambient_signal_by_ref(
    world: &World,
    registry: &ComponentActionRegistry,
    signal_ref: &str,
) -> Option<Value> {
    active_ambient_signal_values(world, registry)
        .into_iter()
        .find(|v| v.get("signal_id").and_then(|s| s.as_str()) == Some(signal_ref))
}

fn entities_with_dynamic(
    world: &World,
    registry: &ComponentActionRegistry,
    component: &str,
) -> Vec<Entity> {
    let Some(id) = registry.dynamic_store().get_id(component) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    for archetype in world.archetypes().iter() {
        if !archetype.components().iter().any(|c| *c == id) {
            continue;
        }
        for arch_entity in archetype.entities() {
            entities.push(arch_entity.id());
        }
    }
    entities
}

fn format_session_context(v: &Value) -> String {
    format!(
        "[Ambient Cue]\n你身处: {}\n天气: {}\n时间: {}\n光线: {}\n来源: {}",
        str_field(v, "location"),
        str_field(v, "weather"),
        str_field(v, "time_of_day"),
        str_field(v, "light"),
        str_field(v, "source")
    )
}

fn format_ambient_signals(values: &[Value]) -> String {
    let entries = values
        .iter()
        .map(|v| {
            format!(
                "- id={} category={} wind={} birds={} light={} temperature={} intensity={:.2} source={} ttl={}s",
                str_field(v, "signal_id"),
                str_field(v, "category"),
                str_field(v, "wind"),
                str_field(v, "birds"),
                str_field(v, "light"),
                str_field(v, "temperature"),
                v.get("intensity").and_then(|n| n.as_f64()).unwrap_or(0.0),
                str_field(v, "source"),
                v.get("ttl_secs").and_then(|n| n.as_u64()).unwrap_or(0)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("[Ambient Signal]\n{entries}")
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|v| v.as_str()).unwrap_or("未设置")
}

fn string_arg(call: &ToolCall, key: &str, default: &str) -> String {
    call.input
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default)
        .to_string()
}

fn number_arg(call: &ToolCall, key: &str, default: f64) -> f64 {
    call.input
        .get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or(default)
}

fn integer_arg(call: &ToolCall, key: &str, default: u64) -> u64 {
    call.input
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_and_registry() -> (World, ComponentActionRegistry) {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        register_ambient_dynamic_components(&mut world, &mut registry);
        (world, registry)
    }

    #[test]
    fn registers_ambient_schemas_and_seeds_glossary() {
        let (mut world, mut registry) = world_and_registry();
        assert!(registry.dynamic_store().get_id(SESSION_CONTEXT).is_some());
        assert!(registry.dynamic_store().get_id(AMBIENT_SIGNAL).is_some());
        assert!(registry.dynamic_store().get_id(GLOSSARY).is_some());

        seed_ambient_glossary(&mut world, &mut registry);
        let values = entities_with_dynamic(&world, &registry, GLOSSARY);
        assert_eq!(values.len(), 5);
    }

    #[test]
    fn set_session_context_adds_ambient_cue() {
        let (mut world, mut registry) = world_and_registry();
        let (out, is_error) = dispatch_set_session_context(
            &mut world,
            &mut registry,
            &ToolCall {
                id: "t1".into(),
                name: "set_session_context".into(),
                input: json!({
                    "location": "书房",
                    "weather": "雨后",
                    "time_of_day": "黄昏",
                    "light": "golden_hour"
                }),
            },
        );
        assert!(!is_error, "{out}");
        let cue = ambient_context_block(&world, &registry);
        assert!(cue.contains("[Ambient Cue]"));
        assert!(cue.contains("你身处: 书房"));
    }

    #[test]
    fn push_ambient_signal_queues_spark_and_expires() {
        let (mut world, mut registry) = world_and_registry();
        let mut queue = std::collections::VecDeque::new();
        let (out, is_error) = dispatch_push_ambient_signal(
            &mut world,
            &mut registry,
            &mut queue,
            &ToolCall {
                id: "t2".into(),
                name: "push_ambient_signal".into(),
                input: json!({
                    "signal_id": "sig-1",
                    "category": "natural",
                    "wind": "微风",
                    "birds": "雀仔声",
                    "ttl_secs": 1
                }),
            },
        );
        assert!(!is_error, "{out}");
        assert_eq!(queue.len(), 1);
        let spark = world
            .get::<crate::components::Spark>(*queue.front().unwrap())
            .unwrap();
        assert_eq!(spark.ambient_signal_ref.as_deref(), Some("sig-1"));
        assert!(ambient_context_for_spark(&world, &registry, Some("sig-1")).contains("微风"));
    }
}
