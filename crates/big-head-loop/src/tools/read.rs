//! 只读 ECS 工具分发：`get_world_overview`、`query_entities`、
//! `get_entity`、`get_component`。
//!
//! 每个分发函数接收解析后的 [`ToolCall`] 和运行时状态引用，
//! 返回 `(output, is_error)` 对。

use bevy_ecs::world::World;
use big_head_ai::ToolCall;

use crate::registry::ComponentActionRegistry;

use super::helpers::{
    execute_get_component, execute_get_entity, execute_get_world_overview, execute_query_entities,
};

/// Paging regenerates a read result, never archives its omitted suffix.
pub(crate) fn page_output(output: String, input: &serde_json::Value) -> Result<String, String> {
    let offset = input
        .get("output_offset")
        .map(|v| v.as_u64().ok_or("invalid output_offset"))
        .transpose()?
        .unwrap_or(0) as usize;
    let limit = input
        .get("output_limit")
        .map(|v| {
            v.as_u64()
                .filter(|n| *n > 0 && *n <= 8000)
                .ok_or("output_limit must be 1..8000")
        })
        .transpose()?
        .unwrap_or(4000) as usize;
    let total = output.chars().count();
    if total <= limit && offset == 0 && input.get("expected_sha256").is_none() {
        return Ok(output);
    }
    let hash = big_head_save::persistence::digest(output.as_bytes());
    if let Some(expected) = input.get("expected_sha256") {
        if expected.as_str() != Some(hash.as_str()) {
            return Err("read result changed; restart paging at output_offset=0".into());
        }
    }
    let text: String = output.chars().skip(offset).take(limit).collect();
    let next = offset.saturating_add(text.chars().count()).min(total);
    Ok(serde_json::json!({"output":text,"output_offset":offset,"total_chars":total,"next_offset":next,"has_more":next<total,"sha256":hash,"paging":"Repeat the same read with output_offset=next_offset and expected_sha256=sha256. Omitted text is not stored as a new artifact."}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_pages_are_complete_and_detect_changes() {
        let source = "中🙂abc".repeat(2000);
        let mut restored = String::new();
        let mut input = serde_json::json!({"output_limit":37});
        loop {
            let page: serde_json::Value =
                serde_json::from_str(&page_output(source.clone(), &input).unwrap()).unwrap();
            restored.push_str(page["output"].as_str().unwrap());
            if page["has_more"] == false {
                break;
            }
            input["output_offset"] = page["next_offset"].clone();
            input["expected_sha256"] = page["sha256"].clone();
        }
        assert_eq!(restored, source);
        assert!(
            page_output("changed".into(), &input)
                .unwrap_err()
                .contains("changed")
        );
        assert!(page_output(source, &serde_json::json!({"output_limit":0})).is_err());
    }
}

pub(crate) fn dispatch_get_world_overview(
    world: &World,
    registry: &ComponentActionRegistry,
    _call: &ToolCall,
) -> (String, bool) {
    execute_get_world_overview(world, registry.dynamic_store())
}

pub(crate) fn dispatch_query_entities(
    world: &World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool) {
    let qs = call
        .input
        .get("query_string")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    execute_query_entities(world, registry.dynamic_store(), qs)
}

pub(crate) fn dispatch_get_entity(
    world: &World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool) {
    let id = call
        .input
        .get("entity_id")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    execute_get_entity(world, registry.dynamic_store(), id)
}

pub(crate) fn dispatch_get_component(
    world: &World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool) {
    let id = call
        .input
        .get("entity_id")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let comp = call
        .input
        .get("component")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    execute_get_component(world, registry.dynamic_store(), id, comp)
}
