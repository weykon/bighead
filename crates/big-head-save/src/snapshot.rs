//! 世界快照 —— 为 LLM 工具提供结构化的世界只读视图。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::world::World;
use bevy_reflect::ReflectFromPtr;
use bevy_reflect::serde::TypedReflectSerializer;
use serde::{Deserialize, Serialize};

/// 单个组件的快照：名称 + 可选的 JSON 字符串值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentSnapshot {
    pub name: String,
    pub value: Option<String>,
}

/// 单个实体的快照：ID + 其所有组件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySnapshot {
    pub id: u64,
    pub components: Vec<ComponentSnapshot>,
}

/// 完整世界快照：组件注册表 + 所有实体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub components_registry: Vec<ComponentRegistryEntry>,
    pub entities: Vec<EntitySnapshot>,
}

/// 组件注册表条目（仅名称）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentRegistryEntry {
    pub name: String,
}

/// 构建世界快照，`reader` 用于读取反射无法处理的动态组件。
pub fn snapshot_with_dynamic_reader(
    world: &World,
    reader: &dyn Fn(&World, Entity, ComponentId) -> Option<serde_json::Value>,
) -> WorldSnapshot {
    build_snapshot(world, Some(reader))
}

/// 构建世界的 Markdown 表示，`reader` 用于动态组件。
pub fn snapshot_markdown_with_dynamic_reader(
    world: &World,
    reader: &dyn Fn(&World, Entity, ComponentId) -> Option<serde_json::Value>,
) -> String {
    let snap = snapshot_with_dynamic_reader(world, reader);
    format_markdown(&snap)
}

fn build_snapshot(
    world: &World,
    dynamic_reader: Option<&dyn Fn(&World, Entity, ComponentId) -> Option<serde_json::Value>>,
) -> WorldSnapshot {
    let mut components_registry = Vec::new();
    let components = world.components();

    let type_registry = world.get_resource::<AppTypeRegistry>();
    let registry_guard = type_registry.map(|r| r.read());

    let mut component_names: BTreeMap<ComponentId, String> = BTreeMap::new();
    for archetype in world.archetypes().iter() {
        for &comp_id in archetype.components() {
            if component_names.contains_key(&comp_id) {
                continue;
            }
            if let Some(info) = components.get_info(comp_id) {
                let name = info.name().to_string();
                component_names.insert(comp_id, name.clone());
                components_registry.push(ComponentRegistryEntry { name });
            }
        }
    }

    let mut entities = Vec::new();
    for archetype in world.archetypes().iter() {
        let arch_components: Vec<ComponentId> = archetype.components().to_vec();

        for arch_entity in archetype.entities() {
            let entity = arch_entity.id();
            let mut comp_snapshots = Vec::new();

            for &comp_id in &arch_components {
                let info = match components.get_info(comp_id) {
                    Some(i) => i,
                    None => continue,
                };
                let name = info.name().to_string();

                let value = try_read_component_value(world, entity, comp_id, info, &registry_guard)
                    .or_else(|| {
                        dynamic_reader
                            .and_then(|r| r(world, entity, comp_id))
                            .map(|v| v.to_string())
                    });

                comp_snapshots.push(ComponentSnapshot { name, value });
            }

            if !comp_snapshots.is_empty() {
                entities.push(EntitySnapshot {
                    id: entity.index().index() as u64,
                    components: comp_snapshots,
                });
            }
        }
    }

    components_registry.sort_by(|a, b| a.name.cmp(&b.name));
    components_registry.dedup_by(|a, b| a.name == b.name);

    WorldSnapshot {
        components_registry,
        entities,
    }
}

/// 读取单个已注册组件的反射值。
///
/// 快照构建与实体点查共用这一条读取路径，保证两个入口输出一致。
pub(crate) fn try_read_component_value(
    world: &World,
    entity: Entity,
    component_id: ComponentId,
    info: &bevy_ecs::component::ComponentInfo,
    registry_guard: &Option<std::sync::RwLockReadGuard<'_, bevy_reflect::TypeRegistry>>,
) -> Option<String> {
    let type_id = info.type_id()?;
    let registry = registry_guard.as_ref()?;
    let reflect_from_ptr = registry.get_type_data::<ReflectFromPtr>(type_id)?;

    let ptr = world.get_by_id(entity, component_id)?;

    // Safety: type_id 已通过 ReflectFromPtr 注册验证
    let reflected = unsafe { reflect_from_ptr.as_reflect(ptr) };
    // Use TypedReflectSerializer for proper JSON serialization instead of debug format
    let serializer = TypedReflectSerializer::new(reflected.as_partial_reflect(), registry);
    serde_json::to_string(&serializer).ok()
}

fn format_markdown(snap: &WorldSnapshot) -> String {
    let mut out = String::new();

    writeln!(out, "## Components").unwrap();
    for entry in &snap.components_registry {
        writeln!(out, "- {}", entry.name).unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "## Entities ({} total)", snap.entities.len()).unwrap();
    for entity in &snap.entities {
        writeln!(out, "### Entity {}", entity.id).unwrap();
        for comp in &entity.components {
            let val = comp.value.as_deref().unwrap_or("(opaque)");
            writeln!(out, "- **{}**: {}", comp.name, val).unwrap();
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use bevy_reflect::{FromType, Reflect};

    #[derive(Component, Reflect, Debug)]
    struct Health(f32);

    #[derive(Component, Reflect, Debug)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Component, Debug)]
    struct Enemy;

    #[test]
    fn test_snapshot_markdown() {
        let mut world = World::new();

        world.register_component::<Health>();
        world.register_component::<Position>();

        let registry = AppTypeRegistry::default();
        {
            let mut reg = registry.write();
            reg.register::<Health>();
            reg.register_type_data::<Health, ReflectFromPtr>();
            reg.register::<Position>();
            reg.register_type_data::<Position, ReflectFromPtr>();
        }
        world.insert_resource(registry);

        world.spawn((Health(100.0), Position { x: 1.0, y: 2.0 }));
        world.spawn((Health(50.0), Enemy));

        let md = snapshot_markdown_with_dynamic_reader(&world, &|_, _, _| None);
        assert!(md.contains("Health"));
        assert!(md.contains("Position"));
        assert!(md.contains("100"));
        assert!(md.contains("50"));
    }

    #[test]
    fn test_snapshot_json() {
        let mut world = World::new();
        world.register_component::<Health>();
        world.spawn(Health(42.0));

        let snap = snapshot_with_dynamic_reader(&world, &|_, _, _| None);
        let _json = serde_json::to_string_pretty(&snap).unwrap();
        assert!(
            snap.components_registry
                .iter()
                .any(|c| c.name.contains("Health"))
        );
    }

    #[test]
    fn test_snapshot_with_dynamic_reader() {
        use bevy_ecs::component::{ComponentCloneBehavior, ComponentDescriptor, StorageType};
        use bevy_ecs::ptr::OwningPtr;
        use std::alloc::Layout;
        use std::sync::{Arc, Mutex};

        unsafe fn drop_boxed_string(ptr: OwningPtr<'_>) {
            let _boxed: Box<String> = unsafe { ptr.read::<Box<String>>() };
        }

        let mut world = World::new();

        let descriptor = unsafe {
            ComponentDescriptor::new_with_layout(
                "DynName",
                StorageType::Table,
                Layout::new::<Box<String>>(),
                Some(drop_boxed_string),
                true,
                ComponentCloneBehavior::Ignore,
                None,
            )
        };
        let id = world.register_component_with_descriptor(descriptor);

        let entity = world.spawn_empty().id();
        let val: Box<String> = Box::new("alice".to_string());
        OwningPtr::make(val, |ptr| unsafe {
            world.entity_mut(entity).insert_by_id(id, ptr);
        });

        let dyn_id = id;
        let reader_calls: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let reader_calls_clone = reader_calls.clone();
        let reader = move |w: &World, e: Entity, cid: ComponentId| -> Option<serde_json::Value> {
            *reader_calls_clone.lock().unwrap() += 1;
            if cid != dyn_id {
                return None;
            }
            let ptr = w.get_by_id(e, cid)?;
            let s: &Box<String> = unsafe { ptr.deref::<Box<String>>() };
            Some(serde_json::Value::String((**s).clone()))
        };

        let snap = snapshot_with_dynamic_reader(&world, &reader);
        let entity_snap = snap
            .entities
            .iter()
            .find(|e| e.id == entity.index().index() as u64)
            .expect("entity should be in snapshot");
        let comp = entity_snap
            .components
            .iter()
            .find(|c| c.name.contains("DynName"))
            .expect("DynName component should be present");
        assert!(comp.value.is_some());
        assert!(comp.value.as_ref().unwrap().contains("alice"));
        assert!(*reader_calls.lock().unwrap() > 0);
    }
}
