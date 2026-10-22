//! 运行时动态组件。
//!
//! 本模块允许运行时（由 `define_component` 工具调用驱动）分配全新的
//! 组件类型，然后通过 bevy_ecs 的常规存储来增/读/删其值。
//! 所有动态组件共享统一的内存布局：`Box<serde_json::Value>`。
//! 这是保持 unsafe 面小且可审计的关键。
//!
//! ## 为什么用统一的 `Box<serde_json::Value>` 布局？
//!
//! bevy_ecs 要求在注册时为每个动态组件提供固定 `Layout` 和 drop 函数。
//! 我们在注册时不知道组件会有什么字段 —— AI 可以发明任意 schema。
//! 与其尝试生成 per-schema 布局（需要运行时代码生成或每次调用 unsafe），
//! 我们交给 bevy 一个单一稳定的布局：`Layout::new::<Box<serde_json::Value>>()`。
//! Box 给了我们间接引用（无论 payload 多大，archetype 行中都是一个固定的
//! 8 字节指针），drop 函数也始终相同：丢掉这个 box。
//!
//! ## 为什么用 `ComponentCloneBehavior::Ignore`？
//!
//! bevy_ecs 对通过 descriptor 注册的组件的默认克隆行为是逐字节复制。
//! 逐字节复制 `Box<T>` 会产生两个指向同一内存的 box，当任意一个实体
//! 被 despawn 时都会导致 double free。`Ignore` 告诉 bevy 不要通过
//! `EntityCloner` 携带此组件，这是安全的选择 —— 动态组件是会话级别的
//! 元数据，不需要克隆。
//!
//! ## 保证 unsafe 健全的不变式
//!
//! 1. 每个动态 `ComponentId` 都由本模块的 `register()` 创建，
//!    因此其内部必定是 `Box<serde_json::Value>`。
//! 2. `insert()` 总是通过 `OwningPtr::make` 写入新分配的
//!    `Box<serde_json::Value>`，因此 bevy 存储的值与布局匹配。
//! 3. `read()` 只在调用方通过 `DynamicComponentStore::is_dynamic(id)`
//!    验证该 id 属于本模块后才调用。`AgentRuntime::snapshot` 中的
//!    运行时代码强制执行此检查。
//! 4. `drop_boxed_json` 是安装在动态 descriptor 上的唯一 drop 函数，
//!    它的编写与上述布局精确匹配。

use std::alloc::Layout;
use std::borrow::Cow;
use std::collections::BTreeMap;

use bevy_ecs::component::{ComponentCloneBehavior, ComponentDescriptor, ComponentId, StorageType};
use bevy_ecs::entity::Entity;
use bevy_ecs::ptr::OwningPtr;
use bevy_ecs::world::World;
use serde_json::Value;

use crate::{DynamicSchema, FieldType};

/// 动态组件的 drop 函数。
///
/// # Safety
///
/// 调用方（bevy_ecs）必须仅在指向有效 `Box<serde_json::Value>` 的
/// `OwningPtr` 上调用此函数 —— 即由下面的 `insert` 生成的值，
/// `insert` 是本模块中唯一向 bevy 存储写入动态组件数据的函数。
unsafe fn drop_boxed_json(ptr: OwningPtr<'_>) {
    // SAFETY: `ptr` 持有一个由 `insert` 写入的 `Box<serde_json::Value>`。
    // 在此读出它会将 box 移到栈上；其析构函数恰好释放底层的
    // `serde_json::Value` 一次。
    let _boxed: Box<Value> = unsafe { ptr.read::<Box<Value>>() };
}

/// 为具有给定名称的新动态组件构建 `ComponentDescriptor`。
/// 所有动态组件共享相同的布局和 drop 函数。
pub fn make_descriptor(name: impl Into<Cow<'static, str>>) -> ComponentDescriptor {
    // SAFETY:
    // - `drop_boxed_json` 可以在指向 `Box<serde_json::Value>` 的指针上
    //   正确调用，这正是 `insert()` 写入此槽的内容。
    // - `Box<serde_json::Value>` 是 `Send + Sync`。
    // - 我们传 `None` 作为关系访问器，因为动态组件不参与关系。
    unsafe {
        ComponentDescriptor::new_with_layout(
            name,
            StorageType::Table,
            Layout::new::<Box<Value>>(),
            Some(drop_boxed_json),
            true,
            ComponentCloneBehavior::Ignore,
            None,
        )
    }
}

/// 在 `world` 上注册一个新的动态组件并返回其 id。
pub fn register(world: &mut World, name: impl Into<Cow<'static, str>>) -> ComponentId {
    let descriptor = make_descriptor(name);
    world.register_component_with_descriptor(descriptor)
}

/// 在 `entity` 上插入（或替换）一个动态组件值。
pub fn insert(
    world: &mut World,
    entity: Entity,
    component_id: ComponentId,
    value: Value,
) -> Result<(), String> {
    if !world.entities().contains(entity) {
        return Err(format!("Entity {:?} does not exist", entity));
    }
    let boxed: Box<Value> = Box::new(value);
    OwningPtr::make(boxed, |ptr| {
        // SAFETY:
        // - `component_id` 由本模块的 `register()` 产生，因此描述的是
        //   `Box<serde_json::Value>` 槽。
        // - `ptr` 指向我们刚创建的 `Box<serde_json::Value>`，与布局完全匹配。
        // - `component_id` 属于 `world`（调用方获取它的唯一途径是通过
        //   本运行时持有的 `DynamicComponentStore`，它在此 world 上注册了它）。
        unsafe {
            world.entity_mut(entity).insert_by_id(component_id, ptr);
        }
    });
    Ok(())
}

/// 从 `entity` 读取动态组件值。若实体没有该组件则返回 `None`。
///
/// # 调用方契约
///
/// `component_id` 必须是之前从本模块的 `register` 返回的动态 id。
/// 传入 Rust 组件的 `ComponentId` 将是未定义行为，因为我们会将任意
/// 字节解释为 `Box<serde_json::Value>`。运行时代码通过
/// `DynamicComponentStore::is_dynamic` 门控此调用。
pub fn read(world: &World, entity: Entity, component_id: ComponentId) -> Option<Value> {
    let ptr = world.get_by_id(entity, component_id)?;
    // SAFETY: 参见上方调用方契约。id 是动态的，因此此槽中的存储
    // 是由 `insert` 写入的 `Box<serde_json::Value>`。
    let boxed_ref: &Box<Value> = unsafe { ptr.deref::<Box<Value>>() };
    Some((**boxed_ref).clone())
}

/// 从 `entity` 移除动态组件，若不存在则为 no-op。
pub fn remove(world: &mut World, entity: Entity, component_id: ComponentId) {
    if !world.entities().contains(entity) {
        return;
    }
    world.entity_mut(entity).remove_by_id(component_id);
}

/// 将 JSON 对象解析为 `DynamicSchema`。
///
/// 期望格式：
/// ```json
/// { "field_a": "string", "field_b": "integer", "field_c": "boolean" }
/// ```
pub fn parse_schema_from_json(value: &Value) -> Result<DynamicSchema, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "schema must be a JSON object".to_string())?;

    let mut fields = BTreeMap::new();
    let mut errors = Vec::new();

    for (name, field_value) in obj {
        let type_str = match field_value.as_str() {
            Some(s) => s,
            None => {
                errors.push(format!(
                    "field '{}': type must be a string, got {}",
                    name, field_value
                ));
                continue;
            }
        };
        let field_type = match type_str {
            "string" => FieldType::String,
            "integer" => FieldType::Integer,
            "number" => FieldType::Number,
            "boolean" => FieldType::Boolean,
            "any" => FieldType::Any,
            other => {
                errors.push(format!(
                    "field '{}': unknown type '{}' (expected one of string, integer, number, boolean, any)",
                    name, other
                ));
                continue;
            }
        };
        fields.insert(name.clone(), field_type);
    }

    // An explicit empty object defines a presence-only marker component.
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(DynamicSchema { fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_register_descriptor_visible() {
        let mut world = World::new();
        let id = register(&mut world, "DynNote");
        let found = world
            .components()
            .iter_registered()
            .any(|info| info.id() == id && info.name().to_string() == "DynNote");
        assert!(found, "Dynamic component should appear in iter_registered");
    }

    #[test]
    fn test_insert_read_roundtrip() {
        let mut world = World::new();
        let id = register(&mut world, "DynA");
        let entity = world.spawn_empty().id();

        let value = serde_json::json!({"text": "hi", "count": 3});
        insert(&mut world, entity, id, value.clone()).unwrap();

        let read_back = read(&world, entity, id).unwrap();
        assert_eq!(read_back, value);
    }

    #[test]
    fn test_insert_replace() {
        let mut world = World::new();
        let id = register(&mut world, "DynB");
        let entity = world.spawn_empty().id();

        insert(&mut world, entity, id, serde_json::json!({"n": 1})).unwrap();
        insert(&mut world, entity, id, serde_json::json!({"n": 2})).unwrap();

        let read_back = read(&world, entity, id).unwrap();
        assert_eq!(read_back, serde_json::json!({"n": 2}));
    }

    #[test]
    fn test_remove_then_read_returns_none() {
        let mut world = World::new();
        let id = register(&mut world, "DynC");
        let entity = world.spawn_empty().id();

        insert(&mut world, entity, id, serde_json::json!({"x": 1})).unwrap();
        assert!(read(&world, entity, id).is_some());

        remove(&mut world, entity, id);
        assert!(read(&world, entity, id).is_none());
    }

    // 计算包裹在 Box 中的 payload 的 drop 次数，以验证 drop 函数
    // 在每次值插入时恰好运行一次，无论是替换还是 despawn。
    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct DropCounted;
    impl Drop for DropCounted {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    unsafe fn drop_boxed_counter(ptr: OwningPtr<'_>) {
        // SAFETY: 此 drop 函数仅安装在其布局匹配 `Box<DropCounted>` 的
        // descriptor 上，测试仅插入 `Box<DropCounted>` 值。
        let _boxed: Box<DropCounted> = unsafe { ptr.read::<Box<DropCounted>>() };
    }

    fn make_counter_descriptor() -> ComponentDescriptor {
        // SAFETY:
        // - drop_boxed_counter 匹配布局 Box<DropCounted>
        // - Box<DropCounted> 是 Send+Sync（DropCounted 是零大小类型）
        unsafe {
            ComponentDescriptor::new_with_layout(
                "DropCounted",
                StorageType::Table,
                Layout::new::<Box<DropCounted>>(),
                Some(drop_boxed_counter),
                true,
                ComponentCloneBehavior::Ignore,
                None,
            )
        }
    }

    #[test]
    fn test_drop_counter() {
        DROP_COUNT.store(0, Ordering::SeqCst);

        let mut world = World::new();
        let descriptor = make_counter_descriptor();
        let id = world.register_component_with_descriptor(descriptor);

        let e1 = world.spawn_empty().id();

        // 插入一次
        let val = Box::new(DropCounted);
        OwningPtr::make(val, |ptr| unsafe {
            world.entity_mut(e1).insert_by_id(id, ptr);
        });
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);

        // 替换：旧值应恰好 drop 一次
        let val2 = Box::new(DropCounted);
        OwningPtr::make(val2, |ptr| unsafe {
            world.entity_mut(e1).insert_by_id(id, ptr);
        });
        assert_eq!(
            DROP_COUNT.load(Ordering::SeqCst),
            1,
            "replace should drop old value once"
        );

        // 移除：当前值应 drop
        world.entity_mut(e1).remove_by_id(id);
        assert_eq!(
            DROP_COUNT.load(Ordering::SeqCst),
            2,
            "remove should drop current value"
        );

        // 再次插入，然后 despawn：应 drop
        let val3 = Box::new(DropCounted);
        OwningPtr::make(val3, |ptr| unsafe {
            world.entity_mut(e1).insert_by_id(id, ptr);
        });
        world.despawn(e1);
        assert_eq!(
            DROP_COUNT.load(Ordering::SeqCst),
            3,
            "despawn should drop remaining value"
        );
    }

    #[test]
    fn test_two_separate_descriptors_get_separate_ids() {
        // 记录 bevy_ecs 的行为，以说明 DynamicComponentStore::define 中
        // 名称唯一性检查的必要性：使用相同名称重新注册会分配全新的
        // ComponentId，不会去重。
        let mut world = World::new();
        let id1 = register(&mut world, "SameName");
        let id2 = register(&mut world, "SameName");
        assert_ne!(id1, id2, "each registration allocates a distinct id");
    }

    #[test]
    fn test_parse_schema_basic() {
        let json = serde_json::json!({"text": "string", "urgent": "boolean", "count": "integer"});
        let schema = parse_schema_from_json(&json).unwrap();
        assert_eq!(schema.fields.len(), 3);
        assert_eq!(schema.fields.get("text"), Some(&FieldType::String));
        assert_eq!(schema.fields.get("urgent"), Some(&FieldType::Boolean));
        assert_eq!(schema.fields.get("count"), Some(&FieldType::Integer));
    }

    #[test]
    fn test_parse_schema_unknown_type_errors() {
        let json = serde_json::json!({"when": "datetime"});
        let result = parse_schema_from_json(&json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("datetime"));
    }

    #[test]
    fn test_parse_schema_marker_accepts_only_empty_object_values() {
        let schema = parse_schema_from_json(&serde_json::json!({})).unwrap();
        assert!(schema.fields.is_empty());
        assert!(crate::schema::validate(&schema, &serde_json::json!({})).is_ok());
        for value in [
            serde_json::json!({"enabled":true}),
            serde_json::json!(null),
            serde_json::json!([]),
        ] {
            assert!(crate::schema::validate(&schema, &value).is_err());
        }
        assert!(parse_schema_from_json(&serde_json::Value::Null).is_err());
    }
}
