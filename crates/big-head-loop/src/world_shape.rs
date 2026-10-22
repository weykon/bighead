//! Read-only archetype counts and bounded samples; no persistent membership index.
use bevy_ecs::world::World;
use big_head_ai::ToolDefinition;
use big_head_ecs::{ComponentActionRegistry, execution::RuntimeIdentity};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn limit() -> usize {
    16
}
fn samples() -> usize {
    3
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    with: Vec<String>,
    #[serde(default)]
    without: Vec<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "limit")]
    limit: usize,
    #[serde(default = "samples")]
    sample_limit: usize,
}

pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "inspect_world_shape".into(),
        description: "Read-only ECS shape: exact matched entity/archetype counts, bounded component counts and archetype pages with sample IDs, no value serialization or new ECS index. with/without are component-name arrays, AND semantics. Samples are not the full result; use query_entities or Lens for values. No Lens or Spark executes.".into(),
        input_schema: json!({"type":"object","properties":{
            "with":{"type":"array","items":{"type":"string"},"maxItems":32},
            "without":{"type":"array","items":{"type":"string"},"maxItems":32},
            "offset":{"type":"integer","minimum":0},
            "limit":{"type":"integer","minimum":1,"maximum":64},
            "sample_limit":{"type":"integer","minimum":0,"maximum":8}
        },"additionalProperties":false}),
    }
}

pub(crate) fn inspect(
    world: &World,
    registry: &ComponentActionRegistry,
    input: &Value,
) -> Result<Value, String> {
    let r: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if r.with.len() > 32
        || r.without.len() > 32
        || !(1..=64).contains(&r.limit)
        || r.sample_limit > 8
    {
        return Err("shape budget: <=32 names per filter, limit=1..64, sample_limit=0..8".into());
    }
    let resolve = |names: &[String]| {
        names
            .iter()
            .map(|name| {
                if name.is_empty() || name.len() > 256 {
                    return Err("invalid component name".into());
                }
                registry
                    .lookup_component_id(world, name)
                    .ok_or_else(|| format!("unknown component: {name}"))
            })
            .collect::<Result<Vec<_>, String>>()
    };
    let required = resolve(&r.with)?;
    let excluded = resolve(&r.without)?;
    let mut total = 0usize;
    let mut matched = 0usize;
    let mut groups = vec![];
    let mut counts = BTreeMap::<String, usize>::new();
    for archetype in world.archetypes().iter() {
        let ids = archetype.components();
        if archetype.entities().is_empty()
            || !required.iter().all(|id| ids.contains(id))
            || excluded.iter().any(|id| ids.contains(id))
        {
            continue;
        }
        let count = archetype.entities().len();
        total += count;
        let names: Vec<_> = ids
            .iter()
            .filter_map(|id| {
                world
                    .components()
                    .get_info(*id)
                    .map(|i| i.name().to_string())
            })
            .collect();
        for name in &names {
            *counts.entry(name.clone()).or_default() += count;
        }
        if matched >= r.offset && groups.len() < r.limit {
            let sample: Vec<_> = archetype.entities().iter().take(r.sample_limit).map(|e| {
                let entity = e.id();
                json!({"entity_id":entity.index().index(),"source_id":world.get::<RuntimeIdentity>(entity).map(|id|&id.id)})
            }).collect();
            groups.push(json!({"archetype_id":archetype.id().index(),"entity_count":count,"component_count":names.len(),"components":names.iter().take(64).collect::<Vec<_>>(),"components_truncated":names.len()>64,"samples":sample}));
        }
        matched += 1;
    }
    let component_count = counts.len();
    let component_counts: Vec<_> = counts
        .into_iter()
        .take(64)
        .map(|(component, count)| json!({"component":component,"entity_count":count}))
        .collect();
    Ok(
        json!({"matched_entities":total,"matched_archetypes":matched,"archetypes":groups,
        "component_counts":component_counts,"component_counts_truncated":component_count>64,
        "next_offset":(r.offset.saturating_add(groups.len()) < matched).then_some(r.offset.saturating_add(groups.len())),
        "scope":{"with":r.with,"without":r.without},"read_only":true,"samples_are_complete":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_and_samples_match_query_without_mutation() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for name in ["Topic", "TopicInsight"] {
            rt.component_registry
                .define_dynamic_component(
                    &mut rt.world,
                    name,
                    big_head_ecs::schema::DynamicSchema {
                        fields: Default::default(),
                    },
                )
                .unwrap();
        }
        for n in 0..20 {
            let e = rt.world.spawn_empty().id();
            rt.component_registry
                .insert_component(&mut rt.world, e, "Topic", "{}")
                .unwrap();
            if n % 2 == 0 {
                rt.component_registry
                    .insert_component(&mut rt.world, e, "TopicInsight", "{}")
                    .unwrap();
            }
        }
        let before = rt.world.entities().len();
        let shape = inspect(
            &rt.world,
            &rt.component_registry,
            &json!({"with":["Topic"],"without":["TopicInsight"],"sample_limit":2}),
        )
        .unwrap();
        assert_eq!(shape["matched_entities"], 10);
        assert_eq!(shape["matched_archetypes"], 1);
        assert_eq!(
            shape["archetypes"][0]["samples"].as_array().unwrap().len(),
            2
        );
        let q = big_head_query::parse_query(
            &rt.world,
            "Topic where WITH(Topic) AND WITHOUT(TopicInsight)",
        )
        .unwrap();
        assert_eq!(
            big_head_query::execute_dynamic_query(&rt.world, &q).len(),
            10
        );
        assert_eq!(rt.world.entities().len(), before);
        assert!(
            inspect(
                &rt.world,
                &rt.component_registry,
                &json!({"without":["Unknown"]})
            )
            .is_err()
        );
        assert!(inspect(&rt.world, &rt.component_registry, &json!({"limit":65})).is_err());
        let page = inspect(
            &rt.world,
            &rt.component_registry,
            &json!({"with":["Topic"],"limit":1}),
        )
        .unwrap();
        assert_eq!(page["matched_entities"], 20);
        assert_eq!(page["next_offset"], 1);
        let next = inspect(
            &rt.world,
            &rt.component_registry,
            &json!({"with":["Topic"],"limit":1,"offset":1}),
        )
        .unwrap();
        assert!(next["next_offset"].is_null());
    }
}
