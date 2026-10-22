//! Explicit retention for temporary artifacts; unknown legacy groups stay permanent.
use bevy_ecs::{component::Component, entity::Entity, world::World};
use big_head_ecs::ComponentActionRegistry;
use big_head_shard::ShardData;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_TTL: u64 = 7 * 86400;
#[derive(Component, Clone, Serialize, Deserialize)]
pub struct ShardRetention {
    pub paradigm: String,
    pub retention_secs: u64,
    pub last_accessed_at: u64,
    pub pinned: bool,
    #[serde(default)]
    pub last_checked_at: u64,
    #[serde(default)]
    pub reclaimed_at: u64,
    #[serde(default)]
    pub reclaimed_bytes: u64,
}

pub fn touch(world: &mut World, paradigm: &str, now: u64) {
    for mut policy in world.query::<&mut ShardRetention>().iter_mut(world) {
        if policy.paradigm == paradigm && policy.reclaimed_at == 0 {
            policy.last_accessed_at = now;
        }
    }
}
pub fn manage_new(world: &mut World, paradigm: &str) {
    if !paradigm.starts_with("context:") {
        return;
    }
    let now = big_head_ecs::execution::now_secs();
    for mut p in world.query::<&mut ShardRetention>().iter_mut(world) {
        if p.paradigm == paradigm {
            p.last_accessed_at = now;
            p.reclaimed_at = 0;
            p.reclaimed_bytes = 0;
            return;
        }
    }
    world.spawn(ShardRetention {
        paradigm: paradigm.into(),
        retention_secs: DEFAULT_TTL,
        last_accessed_at: now,
        pinned: false,
        last_checked_at: 0,
        reclaimed_at: 0,
        reclaimed_bytes: 0,
    });
}

struct Group {
    entities: Vec<Entity>,
    bytes: u64,
    indices: BTreeSet<u32>,
    total: u32,
    complete: bool,
}

pub fn inspect(
    world: &mut World,
    registry: &ComponentActionRegistry,
    systems: &big_head_ecs::SystemRegistry,
    now: u64,
) -> Result<Value, String> {
    let mut policies: Vec<_> = world
        .query::<(Entity, &ShardRetention)>()
        .iter(world)
        .map(|(e, p)| (e, p.clone()))
        .collect();
    let mut groups = BTreeMap::<String, Group>::new();
    for (e, s) in world.query::<(Entity, &ShardData)>().iter(world) {
        let g = groups.entry(s.paradigm.clone()).or_insert(Group {
            entities: vec![],
            bytes: 0,
            indices: BTreeSet::new(),
            total: s.total_shards,
            complete: true,
        });
        g.complete &= g.total == s.total_shards && g.indices.insert(s.shard_index);
        g.entities.push(e);
        g.bytes += s.content.len() as u64;
    }
    let mut names = BTreeSet::new();
    for (_, p) in &policies {
        if !names.insert(p.paradigm.clone()) {
            return Err("duplicate shard retention policy; collection blocked".into());
        }
    }
    policies.sort_by_key(|(_, p)| (p.last_checked_at, p.last_accessed_at));
    let candidates: Vec<_> = policies
        .iter()
        .filter(|(_, p)| {
            !p.pinned
                && p.reclaimed_at == 0
                && p.retention_secs >= 86400
                && now.saturating_sub(p.last_accessed_at) >= p.retention_secs
        })
        .take(128)
        .collect();
    let mut references: BTreeMap<String, Vec<String>> = candidates
        .iter()
        .map(|(_, p)| (p.paradigm.clone(), vec![]))
        .collect();
    for system in systems.iter() {
        let text = serde_json::to_string(system).map_err(|e| e.to_string())?;
        for (paradigm, roots) in &mut references {
            if text.contains(paradigm.as_str()) && roots.len() < 8 {
                roots.push("SystemDefinition".into());
            }
        }
    }
    // All persistent components are conservative roots, including embedded JSON strings.
    // Projection creation receipts describe provenance, not a live retention lease.
    for archetype in world.archetypes().iter() {
        for entry in archetype.entities() {
            let e = entry.id();
            for &cid in archetype.components() {
                let Some(info) = world.components().get_info(cid) else {
                    continue;
                };
                let full = info.name().to_string();
                let name = full.rsplit("::").next().unwrap();
                if name == "ShardRetention" {
                    continue;
                }
                if name == "ShardData" {
                    if let Some(s) = world.get::<ShardData>(e) {
                        for (paradigm, roots) in &mut references {
                            if s.paradigm != *paradigm
                                && s.content.contains(paradigm.as_str())
                                && roots.len() < 8
                            {
                                roots.push(format!("ShardData:{}", s.paradigm));
                            }
                        }
                    }
                    continue;
                }
                let value = if registry.dynamic_store().is_dynamic(cid) {
                    big_head_ecs::dynamic_component::read(world, e, cid)
                } else {
                    registry.serialize_rust_component(world, e, name)
                };
                let Some(value) = value else { continue };
                if name == "ActionRecord"
                    && value.get("kind").and_then(Value::as_str) == Some("ProjectContext")
                {
                    continue;
                }
                let text = value.to_string();
                for (paradigm, roots) in &mut references {
                    if text.contains(paradigm.as_str()) && roots.len() < 8 {
                        roots.push(format!("{}:{}", e.index().index(), name));
                    }
                }
            }
        }
    }
    policies.sort_by_key(|(_, p)| (!references.contains_key(&p.paradigm), p.last_accessed_at));
    let rows:Vec<_>=policies.iter().filter(|(_,p)|p.reclaimed_at==0).take(256).map(|(_,p)| {
        let g=groups.get(&p.paradigm);
        let complete=g.is_some_and(|g|g.complete && g.indices.len()==g.total as usize && g.indices.iter().copied().eq(0..g.total));
        let roots=references.get(&p.paradigm);
        let eligible=complete && roots.is_some_and(|r|r.is_empty());
        json!({"paradigm":p.paradigm,"pinned":p.pinned,"retention_secs":p.retention_secs,"last_accessed_at":p.last_accessed_at,
            "bytes":g.map(|g|g.bytes).unwrap_or(0),"complete":complete,"references_checked":roots.is_some(),"reference_examples":roots,"eligible":eligible})
    }).collect();
    Ok(
        json!({"groups":rows,"managed_groups":policies.len(),"unmanaged_groups":groups.keys().filter(|n|!names.contains(*n)).count(),"candidate_limit":128,"group_limit":256,"legacy_default":"retain"}),
    )
}

pub fn collect(
    world: &mut World,
    registry: &ComponentActionRegistry,
    systems: &big_head_ecs::SystemRegistry,
    now: u64,
) -> Result<Value, String> {
    let report = inspect(world, registry, systems, now)?;
    let checked: BTreeSet<_> = report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["references_checked"] == true)
        .map(|r| r["paradigm"].as_str().unwrap().to_owned())
        .collect();
    let eligible: BTreeSet<_> = report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["eligible"] == true)
        .map(|r| r["paradigm"].as_str().unwrap().to_owned())
        .collect();
    let targets: Vec<_> = world
        .query::<(Entity, &ShardData)>()
        .iter(world)
        .filter(|(_, s)| eligible.contains(&s.paradigm))
        .map(|(e, s)| (e, s.paradigm.clone(), s.content.len() as u64))
        .collect();
    let bytes: u64 = targets.iter().map(|(_, _, bytes)| bytes).sum();
    for (e, _, _) in &targets {
        world.entity_mut(*e).remove::<ShardData>();
        if world.entity(*e).archetype().components().is_empty() {
            world.despawn(*e);
        }
    }
    for mut p in world.query::<&mut ShardRetention>().iter_mut(world) {
        if checked.contains(&p.paradigm) {
            p.last_checked_at = now;
        }
        if eligible.contains(&p.paradigm) {
            p.reclaimed_at = now;
            p.reclaimed_bytes = targets
                .iter()
                .filter(|(_, name, _)| name == &p.paradigm)
                .map(|(_, _, b)| b)
                .sum();
        }
    }
    Ok(
        json!({"checked_groups":checked.len(),"reclaimed_groups":eligible.len(),"reclaimed_shards":targets.len(),"reclaimed_bytes":bytes}),
    )
}

pub fn definitions() -> Vec<big_head_ai::ToolDefinition> {
    vec![
    big_head_ai::ToolDefinition{name:"inspect_shard_retention".into(),description:"Read-only retention and reference report. Legacy groups without policies are permanent. At most 128 expired candidates are reference-scanned; unchecked groups are never collected.".into(),input_schema:json!({"type":"object","properties":{},"additionalProperties":false})},
    big_head_ai::ToolDefinition{name:"set_shard_retention".into(),description:"Set an explicit retention policy for one existing complete shard paradigm. pinned=true prevents collection. TTL is 1..3650 days in seconds from last access. Legacy groups are enrolled only by this explicit call. Does not immediately delete content.".into(),input_schema:json!({"type":"object","properties":{"paradigm":{"type":"string"},"pinned":{"type":"boolean"},"retention_secs":{"type":"integer","minimum":86400,"maximum":315360000}},"required":["paradigm","pinned","retention_secs"],"additionalProperties":false})}
]
}

pub fn configure(world: &mut World, input: &Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        paradigm: String,
        pinned: bool,
        retention_secs: u64,
    }
    let r: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if !(86400..=315360000).contains(&r.retention_secs) {
        return Err("retention_secs must be 86400..315360000".into());
    }
    if !world
        .query::<&ShardData>()
        .iter(world)
        .any(|s| s.paradigm == r.paradigm)
    {
        return Err("unknown shard paradigm".into());
    }
    let shards: Vec<_> = world
        .query::<&ShardData>()
        .iter(world)
        .filter(|s| s.paradigm == r.paradigm)
        .collect();
    let total = shards[0].total_shards;
    let indices: BTreeSet<_> = shards.iter().map(|s| s.shard_index).collect();
    if total == 0
        || shards.len() != total as usize
        || !indices.iter().copied().eq(0..total)
        || shards.iter().any(|s| s.total_shards != total)
    {
        return Err("incomplete shard group; retention policy unchanged".into());
    }
    let matches: Vec<_> = world
        .query::<(Entity, &ShardRetention)>()
        .iter(world)
        .filter(|(_, p)| p.paradigm == r.paradigm)
        .map(|(e, _)| e)
        .collect();
    if matches.len() > 1 {
        return Err("duplicate retention policy".into());
    }
    let p = ShardRetention {
        paradigm: r.paradigm,
        retention_secs: r.retention_secs,
        pinned: r.pinned,
        last_accessed_at: big_head_ecs::execution::now_secs(),
        last_checked_at: 0,
        reclaimed_at: 0,
        reclaimed_bytes: 0,
    };
    let result = serde_json::to_value(&p).unwrap();
    if let Some(e) = matches.first() {
        world.entity_mut(*e).insert(p);
    } else {
        world.spawn(p);
    }
    Ok(result)
}

pub fn expired_notice(world: &mut World, paradigm: &str) -> Option<String> {
    world.query::<&ShardRetention>().iter(world).find(|p|p.paradigm==paradigm && p.reclaimed_at>0)
        .map(|p|format!("Shard content expired under retention policy at {}; paradigm={}; reclaimed_bytes={}",p.reclaimed_at,p.paradigm,p.reclaimed_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_script_references_protect_expired_groups() {
        let mut rt = crate::AgentRuntime::new_for_test();
        add(&mut rt, "context:script", false);
        rt.system_registry
            .register(big_head_ecs::SystemDefinition {
                id: "retention-test".into(),
                description: String::new(),
                system_prompt: "".into(),
                kind: big_head_ecs::SystemKind::RhaiScript {
                    source: "get_shards(\"context:script\")".into(),
                },
                tool_names: vec![],
                max_iterations: 1,
                is_tool: false,
            })
            .unwrap();
        assert_eq!(
            collect(
                &mut rt.world,
                &rt.component_registry,
                &rt.system_registry,
                90001
            )
            .unwrap()["reclaimed_shards"],
            0
        );
        let report = inspect(
            &mut rt.world,
            &rt.component_registry,
            &rt.system_registry,
            90001,
        )
        .unwrap();
        assert_eq!(
            report["groups"][0]["reference_examples"][0],
            "SystemDefinition"
        );
    }
    #[test]
    fn protected_candidates_do_not_starve_later_groups() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let mut references = String::new();
        for i in 0..129 {
            let name = format!("context:fair:{i:03}");
            add(&mut rt, &name, false);
            if i < 128 {
                references.push_str(&name);
                references.push(' ');
            }
        }
        rt.world.spawn(big_head_ecs::TickAction {
            enabled: true,
            script: references,
            cooldown_secs: 600,
            last_run: 0,
        });
        assert_eq!(
            collect(
                &mut rt.world,
                &rt.component_registry,
                &rt.system_registry,
                90001
            )
            .unwrap()["reclaimed_shards"],
            0
        );
        assert_eq!(
            collect(
                &mut rt.world,
                &rt.component_registry,
                &rt.system_registry,
                93601
            )
            .unwrap()["reclaimed_shards"],
            1
        );
    }
    fn add(rt: &mut crate::AgentRuntime, name: &str, pinned: bool) -> Entity {
        rt.world.spawn(ShardRetention {
            paradigm: name.into(),
            retention_secs: 86400,
            last_accessed_at: 1,
            pinned,
            last_checked_at: 0,
            reclaimed_at: 0,
            reclaimed_bytes: 0,
        });
        rt.world
            .spawn(ShardData {
                paradigm: name.into(),
                shard_index: 0,
                total_shards: 1,
                content: "payload".into(),
                source_label: "test".into(),
                created_at: 1,
            })
            .id()
    }
    #[test]
    fn expiry_protects_references_pins_recent_access_and_legacy() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let expired = add(&mut rt, "context:expired", false);
        let pinned = add(&mut rt, "context:pinned", true);
        let referenced = add(&mut rt, "context:referenced", false);
        let recent = add(&mut rt, "context:recent", false);
        rt.world.spawn(big_head_ecs::TickAction {
            enabled: true,
            script: "log(\"context:referenced\");".into(),
            cooldown_secs: 600,
            last_run: 0,
        });
        touch(&mut rt.world, "context:recent", 90000);
        let legacy = rt
            .world
            .spawn(ShardData {
                paradigm: "legacy".into(),
                shard_index: 0,
                total_shards: 1,
                content: "history".into(),
                source_label: "legacy".into(),
                created_at: 1,
            })
            .id();
        let report = collect(
            &mut rt.world,
            &rt.component_registry,
            &rt.system_registry,
            90001,
        )
        .unwrap();
        assert_eq!(report["reclaimed_shards"], 1);
        assert!(rt.world.get::<ShardData>(expired).is_none());
        for e in [pinned, referenced, recent, legacy] {
            assert!(rt.world.get::<ShardData>(e).is_some());
        }
        assert!(
            expired_notice(&mut rt.world, "context:expired")
                .unwrap()
                .contains("expired")
        );
        assert_eq!(
            collect(
                &mut rt.world,
                &rt.component_registry,
                &rt.system_registry,
                90002
            )
            .unwrap()["reclaimed_shards"],
            0
        );
    }
    #[test]
    fn incomplete_groups_and_cross_shard_references_are_retained() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let incomplete = add(&mut rt, "context:incomplete", false);
        rt.world
            .get_mut::<ShardData>(incomplete)
            .unwrap()
            .total_shards = 2;
        let target = add(&mut rt, "context:target", false);
        let source = add(&mut rt, "context:source", true);
        rt.world.get_mut::<ShardData>(source).unwrap().content = "context:target".into();
        assert_eq!(
            collect(
                &mut rt.world,
                &rt.component_registry,
                &rt.system_registry,
                90001
            )
            .unwrap()["reclaimed_shards"],
            0
        );
        assert!(rt.world.get::<ShardData>(target).is_some());
    }
}
