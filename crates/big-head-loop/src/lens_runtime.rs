//! Shared Lens execution for manual tools and automatic context bindings.
use bevy_ecs::{entity::Entity, world::World};
use big_head_ecs::{ComponentActionRegistry, lens::LensApplication};
use big_head_world::application::{self, Catalog, Fact, InputScope, LensDefinition};
use serde_json::{Value, json};

pub(crate) fn catalog(
    world: &mut World,
) -> Result<(Vec<(Entity, LensApplication)>, Catalog), String> {
    let entries: Vec<_> = world
        .query::<(bevy_ecs::entity::Entity, &LensApplication)>()
        .iter(world)
        .map(|(e, a)| (e, a.clone()))
        .collect();
    let catalog = parse_catalog(entries.iter().map(|(_, app)| app))?;
    Ok((entries, catalog))
}

pub(crate) fn definition_catalog(world: &mut World) -> Result<Catalog, String> {
    parse_catalog(world.query::<&LensApplication>().iter(world))
}

fn parse_catalog<'a>(apps: impl Iterator<Item = &'a LensApplication>) -> Result<Catalog, String> {
    let mut catalog = Catalog::new();
    for (index, app) in apps.enumerate() {
        if index >= 128 {
            return Err("Lens catalog exceeds 128 entries".into());
        }
        let definition: LensDefinition = serde_json::from_str(&app.definition_json)
            .map_err(|e| format!("invalid stored Lens {}: {e}", app.id))?;
        if definition.id != app.id || catalog.insert(app.id.clone(), definition).is_some() {
            return Err("duplicate or inconsistent Lens identity".into());
        }
    }
    Ok(catalog)
}

pub(crate) fn observe(
    world: &mut World,
    registry: &ComponentActionRegistry,
    id: &str,
    catalog: &Catalog,
) -> Result<Value, String> {
    observe_with_policy(world, registry, id, catalog, &Default::default())
}

pub(crate) fn observe_with_policy(
    world: &mut World,
    registry: &ComponentActionRegistry,
    id: &str,
    catalog: &Catalog,
    policy: &crate::lens_cache::Policy<'_>,
) -> Result<Value, String> {
    let scope = application::input_scope(id, catalog)?;
    let facts = collect_facts(world, registry, &scope)?;
    let now = big_head_ecs::execution::now_secs();
    let freshness = freshness(
        &facts,
        policy.source_pointer,
        policy.stale_threshold_ms,
        now,
    );
    let inputs: Vec<_> = facts
        .iter()
        .map(|f| json!({"source_id":f.source_id,"version":f.source_version}))
        .collect();
    // Include definition bodies, not just versions: direct ECS edits must invalidate reuse.
    let key_input = json!({"lens_id":id,"catalog":catalog,"inputs":inputs,"source_pointer":policy.source_pointer,"threshold":policy.stale_threshold_ms}).to_string();
    let cache_key = format!(
        "{:016x}",
        crate::observation::stable_hash(key_input.as_bytes())
    );
    if let Some(mut receipt) = crate::lens_cache::reuse(policy, &cache_key, now) {
        receipt["freshness"] = freshness;
        return Ok(receipt);
    }
    let output = application::run(id, catalog, facts)?;
    let receipt = json!({"run_id":uuid::Uuid::new_v4().to_string(),"lens_id":id,"lens_version":catalog[id].version,"observed_at":now,"executed_at":now,"cache_hit":false,"cache_key":cache_key,"kind":"observation","freshness":freshness,"inputs":inputs,"data":output,"catalog_versions":catalog.iter().map(|(id,d)|(id.clone(),d.version.clone())).collect::<std::collections::BTreeMap<_,_>>()});
    if receipt.to_string().len() > 262144 {
        return Err("Lens output exceeds 256 KiB".into());
    }
    Ok(receipt)
}

fn freshness(facts: &[Fact], pointer: Option<&str>, threshold: Option<u64>, now: u64) -> Value {
    let unknown = || json!({"age_ms":null,"stale":null,"status":"unknown"});
    let Some(pointer) = pointer else {
        return unknown();
    };
    let mut oldest = now;
    if facts.is_empty() {
        return unknown();
    }
    for fact in facts {
        let Some(updated) = fact.value.pointer(pointer).and_then(Value::as_u64) else {
            return unknown();
        };
        if updated > now {
            return unknown();
        }
        oldest = oldest.min(updated);
    }
    let age_ms = now.saturating_sub(oldest).saturating_mul(1000);
    json!({"age_ms":age_ms,"stale":threshold.map(|limit| age_ms > limit),"status":"known"})
}

#[cfg(test)]
mod freshness_tests {
    use super::*;

    #[test]
    fn source_time_is_not_observation_time() {
        let fact = Fact {
            source_id: "source".into(),
            source_version: "1".into(),
            value: json!({"Context":{"updated":100}}),
            branches: vec![],
            membership: Default::default(),
        };
        assert_eq!(
            freshness(&[fact.clone()], Some("/Context/updated"), Some(60000), 200)["age_ms"],
            100000
        );
        assert_eq!(
            freshness(&[fact.clone()], Some("/Context/updated"), Some(60000), 200)["stale"],
            true
        );
        assert!(
            freshness(&[fact.clone()], Some("/Context/missing"), Some(60000), 200)["stale"]
                .is_null()
        );
        assert!(freshness(&[fact], Some("/Context/updated"), Some(60000), 50)["age_ms"].is_null());
    }
}

#[cfg(test)]
mod membership_tests {
    use super::*;
    use big_head_ecs::schema::{DynamicSchema, FieldType};
    use std::collections::BTreeMap;

    #[test]
    fn presence_only_components_do_not_expand_reads_or_serialize_payloads() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for (name, fields) in [
            (
                "DemoWork",
                BTreeMap::from([("title".into(), FieldType::String)]),
            ),
            (
                "NeedsReview",
                BTreeMap::from([("payload".into(), FieldType::String)]),
            ),
        ] {
            rt.component_registry
                .define_dynamic_component(&mut rt.world, name, DynamicSchema { fields })
                .unwrap();
        }
        let work = rt.world.spawn_empty().id();
        let outside = rt.world.spawn_empty().id();
        rt.component_registry
            .insert_component(&mut rt.world, work, "DemoWork", r#"{"title":"review"}"#)
            .unwrap();
        for entity in [work, outside] {
            rt.component_registry
                .insert_component(
                    &mut rt.world,
                    entity,
                    "NeedsReview",
                    &json!({"payload":"x".repeat(70000)}).to_string(),
                )
                .unwrap();
        }
        let definition: LensDefinition = serde_json::from_value(json!({
            "id":"demo", "version":"1", "purpose":"review work", "reads":["DemoWork"],
            "stage":{"kind":"archetype_with","component":"NeedsReview"}
        }))
        .unwrap();
        let catalog = Catalog::from([("demo".into(), definition)]);
        let receipt = observe(&mut rt.world, &rt.component_registry, "demo", &catalog).unwrap();
        assert_eq!(receipt["data"].as_array().unwrap().len(), 1);
        assert_eq!(
            receipt["data"][0]["value"],
            json!({"DemoWork":{"title":"review"}})
        );
        assert_eq!(
            receipt["data"][0]["membership"],
            json!({"NeedsReview":true})
        );
        let scope = application::input_scope("demo", &catalog).unwrap();
        let first = collect_facts(&mut rt.world, &rt.component_registry, &scope).unwrap();
        rt.component_registry
            .insert_component(
                &mut rt.world,
                work,
                "NeedsReview",
                r#"{"payload":"changed but still present"}"#,
            )
            .unwrap();
        let second = collect_facts(&mut rt.world, &rt.component_registry, &scope).unwrap();
        assert_eq!(first[0].source_version, second[0].source_version);
        let bad = InputScope {
            presence: ["TypoMarker".into()].into(),
            ..scope
        };
        assert!(
            collect_facts(&mut rt.world, &rt.component_registry, &bad)
                .unwrap_err()
                .contains("unknown selector component")
        );
    }

    #[test]
    fn source_limit_counts_union_once_and_rejects_before_assigning_identities() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for name in ["A", "B"] {
            rt.component_registry
                .define_dynamic_component(
                    &mut rt.world,
                    name,
                    DynamicSchema {
                        fields: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        let mut entities = vec![];
        for i in 0..257 {
            let entity = rt.world.spawn_empty().id();
            rt.component_registry
                .insert_component(&mut rt.world, entity, "A", "{}")
                .unwrap();
            if i % 2 == 0 {
                rt.component_registry
                    .insert_component(&mut rt.world, entity, "B", "{}")
                    .unwrap();
            }
            entities.push(entity);
        }
        let scope = InputScope {
            reads: ["A".into(), "B".into()].into(),
            presence: ["B".into()].into(),
        };
        assert!(
            collect_facts(&mut rt.world, &rt.component_registry, &scope)
                .unwrap_err()
                .contains("256")
        );
        assert!(entities.iter().all(|e| {
            rt.world
                .get::<big_head_ecs::execution::RuntimeIdentity>(*e)
                .is_none()
        }));
        rt.world.despawn(entities.pop().unwrap());
        let facts = collect_facts(&mut rt.world, &rt.component_registry, &scope).unwrap();
        assert_eq!(facts.len(), 256);
        assert_eq!(facts.iter().filter(|f| f.membership["B"]).count(), 128);
        let again = collect_facts(&mut rt.world, &rt.component_registry, &scope).unwrap();
        assert_eq!(
            serde_json::to_value(facts).unwrap(),
            serde_json::to_value(again).unwrap()
        );
    }

    #[test]
    #[ignore = "manual synthetic performance measurement, not a timing assertion"]
    fn measure_archetype_collection_16600_entities() {
        #[derive(bevy_ecs::component::Component)]
        struct Unrelated;
        let mut rt = crate::AgentRuntime::new_for_test();
        for _ in 0..16400 {
            rt.world.spawn(Unrelated);
        }
        for i in 0..200 {
            rt.world.spawn(big_head_ecs::TickAction {
                enabled: true,
                script: "log(\"sample\");".into(),
                cooldown_secs: 600,
                last_run: i,
            });
        }
        let scope = InputScope {
            reads: ["TickAction".into()].into(),
            presence: ["TickAction".into()].into(),
        };
        let expected = collect_facts(&mut rt.world, &rt.component_registry, &scope).unwrap();
        let cid = rt
            .component_registry
            .lookup_component_id(&rt.world, "TickAction")
            .unwrap();
        let mut legacy = || -> Vec<Fact> {
            let entities: Vec<_> = rt.world.query::<Entity>().iter(&rt.world).collect();
            let mut facts = vec![];
            for entity in entities {
                if !rt.world.entity(entity).contains_id(cid) {
                    continue;
                }
                let value = json!({"TickAction":rt.component_registry.serialize_rust_component(&rt.world,entity,"TickAction").unwrap()});
                let membership = BTreeMap::from([("TickAction".into(), true)]);
                let version = crate::observation::stable_hash(
                    &serde_json::to_vec(&(&value, &membership)).unwrap(),
                );
                facts.push(Fact {
                    source_id: big_head_ecs::execution::identity(&mut rt.world, entity),
                    source_version: format!("{version:016x}"),
                    value,
                    branches: vec![],
                    membership,
                });
            }
            facts.sort_by(|a, b| a.source_id.cmp(&b.source_id));
            facts
        };
        assert_eq!(
            serde_json::to_value(legacy()).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let start = std::time::Instant::now();
        for _ in 0..50 {
            std::hint::black_box(legacy());
        }
        let legacy_us = start.elapsed().as_micros() / 50;
        let start = std::time::Instant::now();
        for _ in 0..50 {
            std::hint::black_box(
                collect_facts(&mut rt.world, &rt.component_registry, &scope).unwrap(),
            );
        }
        let archetype_us = start.elapsed().as_micros() / 50;
        let start = std::time::Instant::now();
        for _ in 0..50 {
            std::hint::black_box(
                crate::world_shape::inspect(
                    &rt.world,
                    &rt.component_registry,
                    &json!({"with":["TickAction"]}),
                )
                .unwrap(),
            );
        }
        eprintln!(
            "synthetic 16600 entities / 200 source facts, 50 iterations: legacy_us={legacy_us} archetype_us={archetype_us} shape_us={}",
            start.elapsed().as_micros() / 50
        );
    }
}

pub(crate) fn collect_facts(
    world: &mut World,
    registry: &ComponentActionRegistry,
    scope: &InputScope,
) -> Result<Vec<Fact>, String> {
    let components: Vec<_> = scope
        .reads
        .iter()
        .map(|name| {
            let cid = registry
                .lookup_component_id(world, name)
                .ok_or_else(|| format!("unknown input component: {name}"))?;
            Ok((name.clone(), cid))
        })
        .collect::<Result<_, String>>()?;
    // Presence checks never serialize marker payloads or expand the read universe.
    let selectors: Vec<_> = scope
        .presence
        .iter()
        .map(|name| {
            registry
                .lookup_component_id(world, name)
                .map(|cid| (name.clone(), cid))
                .ok_or_else(|| {
                    format!(
                        "unknown selector component: {name}; define the component before using it"
                    )
                })
        })
        .collect::<Result<_, _>>()?;
    // Resolve membership once per archetype; materialize only the bounded read universe.
    // Capture before assigning RuntimeIdentity, which may move entities to another archetype.
    let mut sources = Vec::new();
    for archetype in world.archetypes().iter() {
        let present: Vec<_> = components
            .iter()
            .filter(|(_, cid)| archetype.components().contains(cid))
            .cloned()
            .collect();
        if present.is_empty() {
            continue;
        }
        if sources.len() + archetype.entities().len() > 256 {
            return Err("more than 256 source entities; narrow reads before running".into());
        }
        let membership: std::collections::BTreeMap<_, _> = selectors
            .iter()
            .map(|(name, cid)| (name.clone(), archetype.components().contains(cid)))
            .collect();
        for entity in archetype.entities() {
            sources.push((entity.id(), present.clone(), membership.clone()));
        }
    }
    let mut facts = Vec::new();
    for (entity, present, membership) in sources {
        let mut values = serde_json::Map::new();
        for (name, cid) in &present {
            let value = if registry.dynamic_store().is_dynamic(*cid) {
                big_head_ecs::dynamic_component::read(world, entity, *cid)
            } else {
                registry.serialize_rust_component(world, entity, name)
            };
            values.insert(
                name.clone(),
                value.ok_or_else(|| format!("component cannot be read: {name}"))?,
            );
        }
        let value = Value::Object(values);
        let serialized = value.to_string();
        if serialized.len() > 65536 {
            return Err(
                "source fact exceeds 64 KiB; project large sources into shards first".into(),
            );
        }
        let version_data = serde_json::to_vec(&(&value, &membership)).map_err(|e| e.to_string())?;
        let version = crate::observation::stable_hash(&version_data);
        facts.push(Fact {
            source_id: big_head_ecs::execution::identity(world, entity),
            source_version: format!("{version:016x}"),
            value,
            branches: Vec::new(),
            membership,
        });
    }
    facts.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(facts)
}
