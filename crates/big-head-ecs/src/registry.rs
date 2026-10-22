//! 组件操作注册表。
//!
//! 注册表将组件名称映射到 insert/patch/remove 闭包，
//! 同时持有 [`DynamicComponentStore`] 管理运行时定义的组件。

use std::collections::HashMap;

use bevy_ecs::component::{Component, ComponentId};
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::dynamic_component;
use crate::helpers::coerce_json_scalar_strings;
use crate::schema::{
    DynamicComponentEntry, DynamicComponentStore, DynamicSchema, resolve_by_short_name, validate,
    validate_patch,
};

type InsertFn = Box<dyn Fn(&mut World, Entity, &str) -> Result<(), String> + Send + Sync>;
type PatchFn = Box<dyn Fn(&mut World, Entity, &str) -> Result<(), String> + Send + Sync>;
type RemoveFn = Box<dyn Fn(&mut World, Entity) + Send + Sync>;

type SerializeFn = Box<dyn Fn(&World, Entity) -> Option<serde_json::Value> + Send + Sync>;

/// 单个 Rust 组件的操作闭包集合。
pub struct ComponentHandler {
    pub(crate) insert: InsertFn,
    pub(crate) patch: PatchFn,
    pub(crate) remove: RemoveFn,
    pub(crate) serialize: SerializeFn,
}

/// 注册表，支持从 JSON 字符串进行动态组件操作。
pub struct ComponentActionRegistry {
    pub(crate) handlers: HashMap<String, ComponentHandler>,
    pub(crate) dynamic_store: DynamicComponentStore,
}

impl ComponentActionRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            dynamic_store: DynamicComponentStore::new(),
        }
    }

    /// 按名称解析组件（支持短名称匹配）。
    pub(crate) fn resolve_name<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        resolve_by_short_name(name, self.handlers.keys().map(|s| s.as_str()))
    }

    /// 暴露动态组件存储，供读取路径使用。
    pub fn dynamic_store(&self) -> &DynamicComponentStore {
        &self.dynamic_store
    }

    /// Names with registered JSON insert/patch/remove handlers, including unused types.
    pub fn rust_component_names(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(String::as_str)
    }

    /// 按组件名称查找 [`ComponentId`]。
    /// 先查动态存储，再回退到世界组件注册表做短名称匹配。
    pub fn lookup_component_id(&self, world: &World, name: &str) -> Option<ComponentId> {
        // 1. 先查动态存储
        if let Some(id) = self.dynamic_store.get_id(name) {
            return Some(id);
        }
        // 2. 回退到世界组件注册表（Rust 注册的组件）
        let name_short = name.rsplit("::").next().unwrap_or(name);
        for info in world.components().iter_registered() {
            let info_name = info.name();
            let info_str: &str = &info_name;
            let info_short = info_str.rsplit("::").next().unwrap_or(info_str);
            if info_short == name_short {
                return Some(info.id());
            }
        }
        None
    }

    /// 将实体上的 Rust 组件序列化为 JSON（如果 handler 存在的话）。
    pub fn serialize_rust_component(
        &self,
        world: &World,
        entity: Entity,
        name: &str,
    ) -> Option<serde_json::Value> {
        let resolved = self.resolve_name(name)?;
        let handler = &self.handlers[resolved];
        (handler.serialize)(world, entity)
    }

    /// 注册一个 Rust 组件类型，使 agent 能动态增/改/删它。
    pub fn register_rust_component<T>(&mut self, name: &str)
    where
        T: Component + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        self.handlers.insert(
            name.to_string(),
            ComponentHandler {
                insert: Box::new(|world, entity, json| {
                    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
                        format!("Parse JSON for '{}': {}", std::any::type_name::<T>(), e)
                    })?;
                    let val: T = deserialize_rust_component_value(value, "Deserialize")?;
                    world.entity_mut(entity).insert(val);
                    Ok(())
                }),
                patch: Box::new(|world, entity, patch_json| {
                    let patch: serde_json::Value = serde_json::from_str(patch_json)
                        .map_err(|e| format!("Parse patch JSON: {}", e))?;
                    match world.get::<T>(entity) {
                        Some(current) => {
                            let mut base = serde_json::to_value(current).map_err(|e| {
                                format!("Serialize current '{}': {}", std::any::type_name::<T>(), e)
                            })?;
                            if let (
                                serde_json::Value::Object(base_map),
                                serde_json::Value::Object(patch_map),
                            ) = (&mut base, &patch)
                            {
                                for (k, v) in patch_map {
                                    base_map.insert(k.clone(), v.clone());
                                }
                            } else {
                                base = patch;
                            }
                            let merged: T =
                                deserialize_rust_component_value(base, "Deserialize merged")?;
                            world.entity_mut(entity).insert(merged);
                            Ok(())
                        }
                        None => {
                            let val: T =
                                deserialize_rust_component_value(patch, "Deserialize for upsert")?;
                            world.entity_mut(entity).insert(val);
                            Ok(())
                        }
                    }
                }),
                remove: Box::new(|world, entity| {
                    world.entity_mut(entity).remove::<T>();
                }),
                serialize: Box::new(|world, entity| {
                    world
                        .get::<T>(entity)
                        .and_then(|val| serde_json::to_value(val).ok())
                }),
            },
        );
    }

    /// 在运行时定义一个新的组件类型。
    pub fn define_dynamic_component(
        &mut self,
        world: &mut World,
        name: &str,
        schema: DynamicSchema,
    ) -> Result<ComponentId, String> {
        if name.trim().is_empty() {
            return Err("component name cannot be empty".into());
        }
        if self.dynamic_store.entries.contains_key(name) {
            return Err(format!(
                "dynamic component '{}' is already defined — pick a different name",
                name
            ));
        }
        if self.resolve_name(name).is_some() {
            return Err(format!(
                "name '{}' collides with an existing Rust component handler — pick a different name",
                name
            ));
        }
        let name_short = name.rsplit("::").next().unwrap_or(name);
        for info in world.components().iter_registered() {
            let existing = info.name().to_string();
            let existing_short = existing.rsplit("::").next().unwrap_or(&existing);
            if existing_short == name_short {
                return Err(format!(
                    "name '{}' collides with an already-registered component '{}' — pick a different name",
                    name, existing
                ));
            }
        }

        let component_id = dynamic_component::register(world, name.to_string());
        self.dynamic_store.dynamic_ids.insert(component_id);
        self.dynamic_store.entries.insert(
            name.to_string(),
            DynamicComponentEntry {
                component_id,
                schema,
            },
        );
        Ok(component_id)
    }

    /// 注销（软删除）一个运行时定义的动态组件类型。
    ///
    /// 从注册表移除名称映射，使组件不可再被发现。警告如果存在使用此组件的实体。
    /// 注意：bevy_ecs 不支持真正的组件类型注销，底层 ComponentId 保留（但不可达）。
    pub fn undefine_dynamic_component(
        &mut self,
        world: &World,
        name: &str,
    ) -> Result<(ComponentId, DynamicSchema), String> {
        // 先检查是否为 Rust 组件（不可删除）
        if self.handlers.contains_key(name) {
            return Err(format!(
                "Component '{}' is a built-in Rust component and cannot be undefined.                  Only dynamically defined components can be undefined.",
                name
            ));
        }

        // 检查组件是否存在于动态 store
        let resolved = self.dynamic_store.resolve_name(name);
        if resolved.is_none() {
            return Err(format!("Component '{}' is not defined", name));
        }

        // 检查是否有实体使用此组件（警告）
        if let Some(id) = self.dynamic_store.get_id(name) {
            let mut entities_with: Vec<u64> = Vec::new();
            for archetype in world.archetypes().iter() {
                if !archetype.components().iter().any(|c| *c == id) {
                    continue;
                }
                for arch_entity in archetype.entities() {
                    entities_with.push(arch_entity.id().to_bits());
                }
            }
            if !entities_with.is_empty() {
                return Err(format!(
                    "Cannot undefined '{}': {} entities still have this component: {:?}.                      Remove the component from these entities first with remove_component,                      or despawn them with despawn_entity.",
                    name,
                    entities_with.len(),
                    &entities_with[..entities_with.len().min(5)]
                ));
            }
        }

        self.dynamic_store.undefine(name)
    }

    /// 向实体插入动态组件值（完整 schema 校验）。
    pub fn insert_dynamic(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let resolved = self
            .dynamic_store
            .resolve_name(name)
            .ok_or_else(|| format!("No dynamic component '{}'", name))?;
        let entry = &self.dynamic_store.entries[resolved];
        validate(&entry.schema, &value).map_err(|e| format!("schema validation: {}", e))?;
        dynamic_component::insert(world, entity, entry.component_id, value)
    }

    /// Upsert 语义：实体已有该组件时，宽松校验 patch 并合并；
    /// 没有时，将 patch 作为完整值插入。
    pub fn patch_dynamic(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
        patch: serde_json::Value,
    ) -> Result<(), String> {
        let resolved = self
            .dynamic_store
            .resolve_name(name)
            .ok_or_else(|| format!("No dynamic component '{}'", name))?;
        let entry = &self.dynamic_store.entries[resolved];

        match dynamic_component::read(world, entity, entry.component_id) {
            Some(current) => {
                validate_patch(&entry.schema, &patch)
                    .map_err(|e| format!("schema validation: {}", e))?;
                let merged = match (current, patch) {
                    (serde_json::Value::Object(mut base), serde_json::Value::Object(patch_map)) => {
                        for (k, v) in patch_map {
                            base.insert(k, v);
                        }
                        serde_json::Value::Object(base)
                    }
                    (_, patch) => patch,
                };
                dynamic_component::insert(world, entity, entry.component_id, merged)
            }
            None => {
                validate(&entry.schema, &patch)
                    .map_err(|e| format!("schema validation (insert): {}", e))?;
                dynamic_component::insert(world, entity, entry.component_id, patch)
            }
        }
    }

    /// 从实体移除动态组件。
    pub fn remove_dynamic(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
    ) -> Result<(), String> {
        let resolved = self
            .dynamic_store
            .resolve_name(name)
            .ok_or_else(|| format!("No dynamic component '{}'", name))?;
        let entry = &self.dynamic_store.entries[resolved];
        dynamic_component::remove(world, entity, entry.component_id);
        Ok(())
    }

    /// 按名称插入组件（先查 Rust handler，再查动态存储）。
    pub fn insert_component(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
        json: &str,
    ) -> Result<(), String> {
        if let Some(resolved) = self.resolve_name(name) {
            let resolved = resolved.to_string();
            let handler = &self.handlers[&resolved];
            return (handler.insert)(world, entity, json);
        }
        if self.dynamic_store.resolve_name(name).is_some() {
            let value: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| format!("parse JSON for dynamic component '{}': {}", name, e))?;
            return self.insert_dynamic(world, entity, name, value);
        }
        Err(format!(
            "No handler or dynamic component registered for '{}'",
            name
        ))
    }

    /// 按名称 patch 组件（先查 Rust handler，再查动态存储）。
    pub fn patch_component(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
        json: &str,
    ) -> Result<(), String> {
        if let Some(resolved) = self.resolve_name(name) {
            let resolved = resolved.to_string();
            let handler = &self.handlers[&resolved];
            return (handler.patch)(world, entity, json);
        }
        if self.dynamic_store.resolve_name(name).is_some() {
            let value: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| format!("parse JSON for dynamic component '{}': {}", name, e))?;
            return self.patch_dynamic(world, entity, name, value);
        }
        Err(format!(
            "No handler or dynamic component registered for '{}'",
            name
        ))
    }

    /// 按名称移除组件（先查 Rust handler，再查动态存储）。
    pub fn remove_component_by_name(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
    ) -> Result<(), String> {
        if let Some(resolved) = self.resolve_name(name) {
            let resolved = resolved.to_string();
            let handler = &self.handlers[&resolved];
            (handler.remove)(world, entity);
            return Ok(());
        }
        if self.dynamic_store.resolve_name(name).is_some() {
            return self.remove_dynamic(world, entity, name);
        }
        Err(format!(
            "No handler or dynamic component registered for '{}'",
            name
        ))
    }
}

fn deserialize_rust_component_value<T>(value: serde_json::Value, context: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    match serde_json::from_value::<T>(value.clone()) {
        Ok(value) => Ok(value),
        Err(original_error) => {
            let coerced = coerce_json_scalar_strings(&value);
            if coerced == value {
                return Err(format!(
                    "{context} '{}': {original_error}",
                    std::any::type_name::<T>()
                ));
            }
            serde_json::from_value::<T>(coerced).map_err(|_| {
                format!(
                    "{context} '{}': {original_error}",
                    std::any::type_name::<T>()
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use std::collections::BTreeMap;

    use crate::schema::{DynamicSchema, FieldType};

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestHealth(f32);

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestName(String);

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestSession {
        title: String,
        status: String,
        path: String,
    }

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestSchedule {
        name: String,
        cooldown_secs: u64,
        enabled: bool,
    }

    fn review_state_schema() -> DynamicSchema {
        let mut fields = BTreeMap::new();
        fields.insert("reviewer".to_string(), FieldType::String);
        fields.insert("approved".to_string(), FieldType::Boolean);
        fields.insert("notes".to_string(), FieldType::String);
        DynamicSchema { fields }
    }

    fn test_world_with_registry() -> (World, ComponentActionRegistry) {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();

        // 反射相关资源（有些测试会用到）
        use bevy_ecs::reflect::AppTypeRegistry;
        world.insert_resource(AppTypeRegistry::default());

        (world, registry)
    }

    #[test]
    fn test_component_action_registry() {
        let (mut world, registry) = test_world_with_registry();
        // 手动注册（不通过 AgentRuntime）
        let mut registry = registry;
        registry.register_rust_component::<TestHealth>("TestHealth");
        registry.register_rust_component::<TestName>("TestName");

        let entity = world.spawn_empty().id();

        registry
            .insert_component(&mut world, entity, "TestHealth", "100.0")
            .unwrap();
        registry
            .insert_component(&mut world, entity, "TestName", "\"warrior\"")
            .unwrap();

        assert!(world.get::<TestHealth>(entity).is_some());
        assert!(world.get::<TestName>(entity).is_some());
        assert_eq!(world.get::<TestHealth>(entity).unwrap().0, 100.0);
        assert_eq!(world.get::<TestName>(entity).unwrap().0, "warrior");

        registry
            .remove_component_by_name(&mut world, entity, "TestHealth")
            .unwrap();
        assert!(world.get::<TestHealth>(entity).is_none());
        assert!(world.get::<TestName>(entity).is_some());
    }

    #[test]
    fn rust_component_insert_coerces_stringified_scalar_fields_after_failure() {
        let (mut world, mut registry) = test_world_with_registry();
        registry.register_rust_component::<TestSchedule>("TestSchedule");

        let entity = world.spawn_empty().id();
        registry
            .insert_component(
                &mut world,
                entity,
                "TestSchedule",
                r#"{"name":"weather tick","cooldown_secs":"21600","enabled":"true"}"#,
            )
            .unwrap();

        let schedule = world.get::<TestSchedule>(entity).unwrap();
        assert_eq!(schedule.name, "weather tick");
        assert_eq!(schedule.cooldown_secs, 21600);
        assert!(schedule.enabled);
    }

    #[test]
    fn test_define_dynamic_component_then_insert() {
        let (mut world, mut registry) = test_world_with_registry();
        registry
            .define_dynamic_component(&mut world, "ReviewState", review_state_schema())
            .unwrap();

        let entity = world.spawn_empty().id();
        let value = serde_json::json!({
            "reviewer": "alice",
            "approved": true,
            "notes": "looks good"
        });
        registry
            .insert_dynamic(&mut world, entity, "ReviewState", value.clone())
            .unwrap();

        let id = registry.dynamic_store().get_id("ReviewState").unwrap();
        let read_back = dynamic_component::read(&world, entity, id).unwrap();
        assert_eq!(read_back, value);
    }

    #[test]
    fn test_dynamic_component_visible_in_iter_registered() {
        let (mut world, mut registry) = test_world_with_registry();
        registry
            .define_dynamic_component(&mut world, "Note", {
                let mut f = BTreeMap::new();
                f.insert("text".to_string(), FieldType::String);
                DynamicSchema { fields: f }
            })
            .unwrap();

        let found = world
            .components()
            .iter_registered()
            .any(|info| info.name().to_string() == "Note");
        assert!(found);
    }

    #[test]
    fn test_define_twice_errors() {
        let (mut world, mut registry) = test_world_with_registry();
        registry
            .define_dynamic_component(&mut world, "Tag", {
                let mut f = BTreeMap::new();
                f.insert("label".to_string(), FieldType::String);
                DynamicSchema { fields: f }
            })
            .unwrap();

        let second = registry.define_dynamic_component(&mut world, "Tag", {
            let mut f = BTreeMap::new();
            f.insert("label".to_string(), FieldType::String);
            DynamicSchema { fields: f }
        });
        assert!(second.is_err());
    }

    #[test]
    fn test_define_collides_with_rust_handler_errors() {
        let (mut world, mut registry) = test_world_with_registry();
        registry.register_rust_component::<TestHealth>("TestHealth");

        let result = registry.define_dynamic_component(&mut world, "TestHealth", {
            let mut f = BTreeMap::new();
            f.insert("hp".to_string(), FieldType::Number);
            DynamicSchema { fields: f }
        });
        assert!(result.is_err(), "expected collision with Rust handler");
    }

    #[test]
    fn test_patch_dynamic_partial() {
        let (mut world, mut registry) = test_world_with_registry();
        registry
            .define_dynamic_component(&mut world, "ReviewState", review_state_schema())
            .unwrap();
        let entity = world.spawn_empty().id();
        registry
            .insert_dynamic(
                &mut world,
                entity,
                "ReviewState",
                serde_json::json!({
                    "reviewer": "alice",
                    "approved": false,
                    "notes": "pending"
                }),
            )
            .unwrap();

        registry
            .patch_dynamic(
                &mut world,
                entity,
                "ReviewState",
                serde_json::json!({
                    "approved": true
                }),
            )
            .unwrap();

        let id = registry.dynamic_store().get_id("ReviewState").unwrap();
        let read_back = dynamic_component::read(&world, entity, id).unwrap();
        assert_eq!(read_back.get("approved").unwrap(), &serde_json::json!(true));
        assert_eq!(
            read_back.get("reviewer").unwrap(),
            &serde_json::json!("alice")
        );
        assert_eq!(
            read_back.get("notes").unwrap(),
            &serde_json::json!("pending")
        );
    }

    #[test]
    fn test_patch_dynamic_upserts_when_absent() {
        let (mut world, mut registry) = test_world_with_registry();
        registry
            .define_dynamic_component(&mut world, "ReviewState", review_state_schema())
            .unwrap();
        let entity = world.spawn_empty().id();

        registry
            .patch_dynamic(
                &mut world,
                entity,
                "ReviewState",
                serde_json::json!({
                    "reviewer": "alice",
                    "approved": true,
                    "notes": "Looks good"
                }),
            )
            .unwrap();

        let id = registry.dynamic_store().get_id("ReviewState").unwrap();
        let read_back = dynamic_component::read(&world, entity, id).unwrap();
        assert_eq!(
            read_back.get("reviewer").unwrap(),
            &serde_json::json!("alice")
        );
        assert_eq!(read_back.get("approved").unwrap(), &serde_json::json!(true));
        assert_eq!(
            read_back.get("notes").unwrap(),
            &serde_json::json!("Looks good")
        );

        let entity2 = world.spawn_empty().id();
        let err = registry
            .patch_dynamic(
                &mut world,
                entity2,
                "ReviewState",
                serde_json::json!({
                    "approved": true
                }),
            )
            .unwrap_err();
        assert!(
            err.contains("missing field"),
            "expected missing field error, got: {}",
            err
        );
    }

    #[test]
    fn test_patch_rust_component_upserts_when_absent() {
        let (mut world, mut registry) = test_world_with_registry();
        registry.register_rust_component::<TestHealth>("TestHealth");
        let entity = world.spawn_empty().id();

        registry
            .patch_component(&mut world, entity, "TestHealth", "42.0")
            .unwrap();

        assert_eq!(world.get::<TestHealth>(entity).unwrap().0, 42.0);

        registry
            .patch_component(&mut world, entity, "TestHealth", "99.0")
            .unwrap();
        assert_eq!(world.get::<TestHealth>(entity).unwrap().0, 99.0);
    }

    #[test]
    fn test_remove_dynamic() {
        let (mut world, mut registry) = test_world_with_registry();
        registry
            .define_dynamic_component(&mut world, "ReviewState", review_state_schema())
            .unwrap();
        let entity = world.spawn_empty().id();
        registry
            .insert_dynamic(
                &mut world,
                entity,
                "ReviewState",
                serde_json::json!({
                    "reviewer": "alice",
                    "approved": true,
                    "notes": "ok"
                }),
            )
            .unwrap();

        let id = registry.dynamic_store().get_id("ReviewState").unwrap();
        assert!(dynamic_component::read(&world, entity, id).is_some());

        registry
            .remove_dynamic(&mut world, entity, "ReviewState")
            .unwrap();
        assert!(dynamic_component::read(&world, entity, id).is_none());
    }

    #[test]
    fn test_serialize_rust_component() {
        let (mut world, mut registry) = test_world_with_registry();
        registry.register_rust_component::<TestHealth>("TestHealth");
        let entity = world.spawn(TestHealth(42.5)).id();

        let val = registry.serialize_rust_component(&world, entity, "TestHealth");
        assert!(val.is_some());
        assert_eq!(val.unwrap(), serde_json::json!(42.5));
    }
}
