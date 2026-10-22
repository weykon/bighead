//! 纯 ECS 辅助函数 —— 实体解析、值解包、动态组件 reader。
//! 供 registry、scripting、tools 等模块共享。

use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::dynamic_component;
use crate::schema::DynamicComponentStore;

/// 将原始实体索引解析为活跃的 `Entity` 句柄，若 id 超出范围或已
/// despawn 则返回可读的错误信息。
pub fn resolve_entity(world: &World, entity_id: u64) -> Result<Entity, String> {
    let entities = world.entities();
    let index = u32::try_from(entity_id)
        .ok()
        .and_then(bevy_ecs::entity::EntityIndex::from_raw_u32);
    if let Some(index) = index {
        let entity = entities.resolve_from_index(index);
        if world.get_entity(entity).is_ok() {
            return Ok(entity);
        }
    }
    Err(format!("Entity {} not found", entity_id))
}

#[cfg(test)]
mod entity_tests {
    use super::*;

    #[test]
    fn resolves_only_spawned_entities_without_truncating_indices() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let index = u64::from(entity.index().index());
        assert_eq!(resolve_entity(&world, index).unwrap(), entity);
        assert!(resolve_entity(&world, index + (1u64 << 32)).is_err());
        assert!(resolve_entity(&world, 999999).is_err());
        world.despawn(entity);
        assert!(resolve_entity(&world, index).is_err());
    }
}

/// 某些 LLM（尤其是 MiniMax-M2）有时会将工具的 `value`/`patch` 参数包裹为
/// JSON 编码的字符串，而非内联 JSON 对象 —— 例如
/// `"value": "{\"a\": 1}"` 而不是 `"value": {"a": 1}`。
/// 如果此处看到的是能正确解析为 JSON 对象或数组的字符串，就透明地解包一层，
/// 让下游的 schema 校验能正常工作。
pub fn unwrap_stringified_json(value: &serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::String(s) = value {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
            if matches!(
                parsed,
                serde_json::Value::Object(_) | serde_json::Value::Array(_)
            ) {
                return parsed;
            }
        }
    }
    value.clone()
}

/// 将 JSON 对象/数组中的字符串标量尽量转回 JSON 标量。
///
/// 这是强类型 Rust 组件反序列化失败后的 fallback，用来容错 LLM 把
/// `u64`/`bool` 等字段写成 `"21600"`、`"true"` 的情况。调用方应先尝试
/// 原始值，只有失败时再尝试此转换，避免破坏本来应该是字符串的字段。
pub fn coerce_json_scalar_strings(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => coerce_string_scalar(s),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(coerce_json_scalar_strings)
                .collect::<Vec<_>>(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), coerce_json_scalar_strings(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn coerce_string_scalar(s: &str) -> serde_json::Value {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if trimmed.eq_ignore_ascii_case("null") {
        return serde_json::Value::Null;
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return serde_json::Value::Number(integer.into());
    }
    if let Ok(float) = trimmed.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(float) {
            return serde_json::Value::Number(number);
        }
    }
    serde_json::Value::String(s.to_string())
}

/// 构建一个动态 reader 闭包（返回 JSON），用于快照读取路径。
pub fn snapshot_reader(
    store: &DynamicComponentStore,
) -> impl Fn(&World, Entity, ComponentId) -> Option<serde_json::Value> + '_ {
    move |world, entity, id| {
        if !store.is_dynamic(id) {
            return None;
        }
        dynamic_component::read(world, entity, id)
    }
}

/// 构建一个动态 reader 闭包（返回字符串），用于查询路径。
pub fn query_reader(
    store: &DynamicComponentStore,
) -> impl Fn(&World, Entity, ComponentId) -> Option<String> + '_ {
    move |world, entity, id| {
        if !store.is_dynamic(id) {
            return None;
        }
        dynamic_component::read(world, entity, id).map(|v| v.to_string())
    }
}
