//! Persist materialized view commands in the world's ViewState component.

use crate::registry::ComponentActionRegistry;
use bevy_ecs::{entity::Entity, world::World};

/// 持久化 render_view 命令到 ECS 的 ViewState 组件
pub(super) fn persist_view_state(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    input: &serde_json::Value,
) -> Result<Entity, String> {
    use big_head_ecs::DynamicSchema;
    use big_head_ecs::dynamic_component;
    use big_head_ecs::schema::FieldType;

    let op = input.get("op").and_then(|v| v.as_str()).unwrap_or("spawn");
    let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");

    // 确保 ViewState 组件已定义
    let view_state_id = match registry.dynamic_store().get_id("ViewState") {
        Some(cid) => cid,
        None => {
            let schema = DynamicSchema {
                fields: std::collections::BTreeMap::from([("panels".into(), FieldType::Any)]),
            };
            registry
                .define_dynamic_component(world, "ViewState", schema)
                .map_err(|e| format!("define ViewState: {}", e))?;
            registry.dynamic_store().get_id("ViewState").unwrap()
        }
    };

    // 查找现有 ViewState 实体
    let view_entity = {
        let mut found: Option<bevy_ecs::entity::Entity> = None;
        for archetype in world.archetypes().iter() {
            if !archetype.components().contains(&view_state_id) {
                continue;
            }
            for arch_entity in archetype.entities() {
                found = Some(arch_entity.id());
                break;
            }
            if found.is_some() {
                break;
            }
        }
        found.unwrap_or_else(|| world.spawn_empty().id())
    };

    // 获取当前面板状态
    let mut panels: serde_json::Map<String, serde_json::Value> =
        dynamic_component::read(world, view_entity, view_state_id)
            .and_then(|v| v.as_object().cloned())
            .map(|obj| {
                obj.get("panels")
                    .and_then(|p| p.as_object().cloned())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

    match op {
        "spawn" => {
            // 添加新面板
            let panel_data = serde_json::json!({
                "surface": input.get("surface"),
                "kind": input.get("kind"),
                "title": input.get("title"),
                "position": input.get("position"),
                "rotation": input.get("rotation"),
                "content": input.get("content"),
            });
            panels.insert(id.to_string(), panel_data);
        }
        "despawn" => {
            // 移除面板
            panels.remove(id);
        }
        "update" | "move" | "scale" => {
            // 更新现有面板的特定字段
            if let Some(panel) = panels.get_mut(id) {
                if let Some(content) = view_update_content(input) {
                    if let Some(obj) = panel.as_object_mut() {
                        obj.insert("content".to_string(), content.clone());
                    }
                }
                if let Some(pos) = input.get("position") {
                    if let Some(obj) = panel.as_object_mut() {
                        obj.insert("position".to_string(), pos.clone());
                    }
                }
                if let Some(rotation) = input.get("rotation") {
                    if let Some(obj) = panel.as_object_mut() {
                        obj.insert("rotation".to_string(), rotation.clone());
                    }
                }
                if let Some(scale) = input.get("scale") {
                    if let Some(obj) = panel.as_object_mut() {
                        obj.insert("scale".to_string(), scale.clone());
                    }
                }
            }
        }
        "batch" => {
            // 批量处理
            if let Some(cmds) = input.get("commands").and_then(|v| v.as_array()) {
                for cmd in cmds {
                    if let (Some(cmd_op), Some(cmd_id)) = (
                        cmd.get("op").and_then(|v| v.as_str()),
                        cmd.get("id").and_then(|v| v.as_str()),
                    ) {
                        match cmd_op {
                            "despawn" => {
                                panels.remove(cmd_id);
                            }
                            "spawn" => {
                                let panel_data = serde_json::json!({
                                    "surface": cmd.get("surface"),
                                    "kind": cmd.get("kind"),
                                    "title": cmd.get("title"),
                                    "position": cmd.get("position"),
                                    "rotation": cmd.get("rotation"),
                                    "content": cmd.get("content"),
                                });
                                panels.insert(cmd_id.to_string(), panel_data);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        _ => {}
    }

    // 保存到 ECS
    let json_str = serde_json::to_string(&serde_json::json!({"panels": panels}))
        .map_err(|e| format!("serialize: {}", e))?;
    registry.insert_component(world, view_entity, "ViewState", &json_str)?;

    Ok(view_entity)
}

fn view_update_content(input: &serde_json::Value) -> Option<&serde_json::Value> {
    input
        .get("patch")
        .and_then(|patch| patch.get("content"))
        .or_else(|| input.get("content"))
}
