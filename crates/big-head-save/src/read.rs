//! 实体/组件读取辅助函数 —— 供 Rhai 脚本引擎和工具分发使用。
//!
//! 点查路径：只读取目标实体自己的 archetype 组件，不再为一次读取重建整个世界快照。

use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::world::World;

use big_head_ecs::{DynamicComponentStore, resolve_entity};

use crate::snapshot::try_read_component_value;

/// 读取某个组件在当前实体上的值。
///
/// 动态组件走 `DynamicComponentStore`，静态/反射组件走 `AppTypeRegistry` 反射读取；
/// 反射不可用或未注册时返回 `None`（调用方以 `(opaque)` 呈现）。
fn component_value(
    world: &World,
    store: &DynamicComponentStore,
    entity: Entity,
    component_id: ComponentId,
) -> Option<String> {
    if store.is_dynamic(component_id) {
        return big_head_ecs::dynamic_component::read(world, entity, component_id)
            .map(|value| value.to_string());
    }
    let type_registry = world.get_resource::<AppTypeRegistry>()?;
    let guard = type_registry.read();
    let info = world.components().get_info(component_id)?;
    try_read_component_value(world, entity, component_id, info, &Some(guard))
}

/// 实体当前挂载的组件（ComponentId + 短名），按 archetype 顺序返回。
fn entity_components(world: &World, entity: Entity) -> Vec<(ComponentId, String)> {
    let Ok(entity_ref) = world.get_entity(entity) else {
        return Vec::new();
    };
    let components = world.components();
    entity_ref
        .archetype()
        .components()
        .iter()
        .filter_map(|id| {
            components.get_info(*id).map(|info| {
                let name = info.name();
                let short = name.rsplit("::").next().unwrap_or(&name);
                (*id, short.to_string())
            })
        })
        .collect()
}

/// Read structured data for scripts without parsing human-facing output.
pub fn read_component_value(
    world: &World,
    store: &DynamicComponentStore,
    entity: Entity,
    name: &str,
) -> Option<serde_json::Value> {
    let short = name.rsplit("::").next().unwrap_or(name);
    let (id, _) = entity_components(world, entity)
        .into_iter()
        .find(|(_, n)| n == short)?;
    serde_json::from_str(&component_value(world, store, entity, id)?).ok()
}

/// 获取单个实体的所有组件值。
pub fn execute_get_entity(
    world: &World,
    store: &DynamicComponentStore,
    entity_id: u64,
) -> (String, bool) {
    let entity = match resolve_entity(world, entity_id) {
        Ok(entity) => entity,
        Err(e) => return (e, true),
    };
    let components = entity_components(world, entity);
    if components.is_empty() {
        return (
            format!(
                "Entity {} exists but has no readable components.",
                entity_id
            ),
            false,
        );
    }
    use std::fmt::Write;
    let mut out = format!("Entity {}:\n", entity_id);
    for (id, short_name) in components {
        let value = component_value(world, store, entity, id);
        writeln!(
            out,
            "  {}: {}",
            short_name,
            value.as_deref().unwrap_or("(opaque)")
        )
        .unwrap();
    }
    (out, false)
}

/// 获取实体上单个组件的值。
pub fn execute_get_component(
    world: &World,
    store: &DynamicComponentStore,
    entity_id: u64,
    component_name: &str,
) -> (String, bool) {
    let entity = match resolve_entity(world, entity_id) {
        Ok(entity) => entity,
        Err(e) => return (e, true),
    };
    let components = entity_components(world, entity);
    if components.is_empty() {
        return (
            format!("Entity {} has no readable components", entity_id),
            true,
        );
    }
    let short_target = component_name.rsplit("::").next().unwrap_or(component_name);
    for (id, short_name) in components {
        if short_name == short_target {
            let value = component_value(world, store, entity, id);
            return (
                format!("{}: {}", short_name, value.as_deref().unwrap_or("(opaque)")),
                false,
            );
        }
    }
    (
        format!("Entity {} has no component '{}'", entity_id, component_name),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::snapshot_with_dynamic_reader;
    use bevy_ecs::world::World;
    use big_head_ecs::schema::{DynamicSchema, FieldType};
    use big_head_ecs::{ComponentActionRegistry, snapshot_reader};
    use std::collections::BTreeMap;

    /// 旧实现：为一次读取重建整个世界快照，再从中挑目标实体。
    /// 保留在此仅用于对照，防止点查路径的输出格式漂移。
    fn legacy_get_entity(world: &World, store: &DynamicComponentStore, id: u64) -> (String, bool) {
        match resolve_entity(world, id) {
            Ok(_) => {
                use std::fmt::Write;
                let snap = snapshot_with_dynamic_reader(world, &snapshot_reader(store));
                match snap.entities.iter().find(|e| e.id == id) {
                    Some(es) => {
                        let mut out = format!("Entity {}:\n", id);
                        for comp in &es.components {
                            let short = comp.name.rsplit("::").next().unwrap_or(&comp.name);
                            writeln!(
                                out,
                                "  {}: {}",
                                short,
                                comp.value.as_deref().unwrap_or("(opaque)")
                            )
                            .unwrap();
                        }
                        (out, false)
                    }
                    None => (
                        format!("Entity {} exists but has no readable components.", id),
                        false,
                    ),
                }
            }
            Err(e) => (e, true),
        }
    }

    fn legacy_get_component(
        world: &World,
        store: &DynamicComponentStore,
        id: u64,
        name: &str,
    ) -> (String, bool) {
        match resolve_entity(world, id) {
            Ok(_) => {
                let snap = snapshot_with_dynamic_reader(world, &snapshot_reader(store));
                match snap.entities.iter().find(|e| e.id == id) {
                    Some(es) => {
                        let target = name.rsplit("::").next().unwrap_or(name);
                        for comp in &es.components {
                            let short = comp.name.rsplit("::").next().unwrap_or(&comp.name);
                            if short == target {
                                return (
                                    format!(
                                        "{}: {}",
                                        short,
                                        comp.value.as_deref().unwrap_or("(opaque)")
                                    ),
                                    false,
                                );
                            }
                        }
                        (format!("Entity {} has no component '{}'", id, name), true)
                    }
                    None => (format!("Entity {} has no readable components", id), true),
                }
            }
            Err(e) => (e, true),
        }
    }

    fn world_with_two_dynamic_components() -> (World, ComponentActionRegistry, Entity, Entity) {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        for (name, fields) in [
            (
                "Note",
                BTreeMap::from([("title".to_string(), FieldType::String)]),
            ),
            ("Flag", BTreeMap::new()),
        ] {
            registry
                .define_dynamic_component(&mut world, name, DynamicSchema { fields })
                .unwrap();
        }
        let both = world.spawn_empty().id();
        registry
            .insert_component(&mut world, both, "Note", r#"{"title":"hi"}"#)
            .unwrap();
        registry
            .insert_component(&mut world, both, "Flag", "{}")
            .unwrap();
        let single = world.spawn_empty().id();
        registry
            .insert_component(&mut world, single, "Note", r#"{"title":"solo"}"#)
            .unwrap();
        (world, registry, both, single)
    }

    #[test]
    fn point_lookup_matches_full_snapshot_output() {
        let (mut world, registry, both, single) = world_with_two_dynamic_components();
        // 一个非动态（反射）组件，覆盖反射分支；未注册类型时呈现 (opaque)。
        let reflected = world.spawn(big_head_ecs::Spark::default()).id();
        let blank = world.spawn_empty().id();
        let store = registry.dynamic_store();

        for entity in [both, single, reflected, blank] {
            let id = u64::from(entity.index().index());
            assert_eq!(
                legacy_get_entity(&world, store, id),
                execute_get_entity(&world, store, id),
                "get_entity output drifted for {id}"
            );
            for name in ["Note", "Flag", "Spark", "Missing"] {
                assert_eq!(
                    legacy_get_component(&world, store, id, name),
                    execute_get_component(&world, store, id, name),
                    "get_component output drifted for {id}/{name}"
                );
            }
        }
        assert_eq!(
            legacy_get_entity(&world, store, 999_999),
            execute_get_entity(&world, store, 999_999)
        );
        let types = AppTypeRegistry::default();
        types.write().register::<big_head_ecs::Spark>();
        world.insert_resource(types);
        let id = u64::from(reflected.index().index());
        let actual = execute_get_component(&world, store, id, "Spark");
        assert!(!actual.0.contains("(opaque)"));
        assert_eq!(actual, legacy_get_component(&world, store, id, "Spark"));
    }

    /// 点查成本应只与目标实体有关；旧路径会随世界实体数线性增长。
    /// 这里用 20k 个无关实体把差距放到数量级上，阈值取足够宽松以避免机器噪声抖动。
    #[test]
    #[ignore = "manual timing measurement; wall-clock assertions are not regression tests"]
    fn point_lookup_does_not_scale_with_world_size() {
        let (mut world, registry, both, _) = world_with_two_dynamic_components();
        for _ in 0..20_000 {
            world.spawn_empty();
        }
        let store = registry.dynamic_store();
        let id = u64::from(both.index().index());

        let expected = legacy_get_entity(&world, store, id);
        let start = std::time::Instant::now();
        let actual = execute_get_entity(&world, store, id);
        let point = start.elapsed();
        assert_eq!(expected, actual);

        let start = std::time::Instant::now();
        let _ = legacy_get_entity(&world, store, id);
        let legacy = start.elapsed();
        eprintln!(
            "[read-path] 20k unrelated entities: point_lookup={point:?} full_snapshot={legacy:?} ratio={:.0}x",
            legacy.as_secs_f64() / point.as_secs_f64().max(f64::EPSILON)
        );

        // 点查必须是世界规模无关的微秒级操作，而不是每次重建整个快照。
        assert!(
            point < std::time::Duration::from_millis(2),
            "point lookup took {point:?}; it likely rebuilds the world snapshot again"
        );
        assert!(
            legacy > point,
            "legacy snapshot path should be slower: point={point:?} legacy={legacy:?}"
        );
    }
}
