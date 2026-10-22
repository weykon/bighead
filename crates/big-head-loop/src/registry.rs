//! [`apply_action`] 桥接 —— 接收解析后的 [`AgentAction`] 并通过
//! [`ComponentActionRegistry`] 路由到世界变更。
//!
//! [`ComponentActionRegistry`] 定义在 [`big_head_ecs`] 中。

use bevy_ecs::world::World;

use big_head_ai::AgentAction;
use big_head_ecs::{resolve_entity, unwrap_stringified_json};

pub use big_head_ecs::ComponentActionRegistry;

/// 将一个 agent action 应用到世界上。成功时返回 Ok(Some(日志消息))。
pub(crate) fn apply_action(
    world: &mut World,
    action: &AgentAction,
    registry: &mut ComponentActionRegistry,
) -> Result<Option<String>, String> {
    match action {
        AgentAction::SpawnEntity {
            components,
            description: _,
        } => {
            let entity = world.spawn_empty().id();

            let mut inserted = Vec::new();
            for comp in components {
                let unwrapped = unwrap_stringified_json(&comp.value);
                let json_str = serde_json::to_string(&unwrapped)
                    .map_err(|e| format!("Serialize component value: {}", e))?;
                match registry.insert_component(world, entity, &comp.name, &json_str) {
                    Ok(()) => {
                        inserted.push(comp.name.clone());
                    }
                    Err(e) => {
                        world.despawn(entity);
                        return Err(format!(
                            "SpawnEntity: failed to insert '{}' on {:?}; rolled back entity: {}",
                            comp.name, entity, e
                        ));
                    }
                }
            }
            Ok(Some(format!(
                "Spawned {:?} with [{}]",
                entity,
                inserted.join(", ")
            )))
        }

        AgentAction::ModifyComponent {
            entity_id,
            component,
            value,
        } => {
            let entity = resolve_entity(world, *entity_id)?;
            let unwrapped = unwrap_stringified_json(value);
            let json_str = serde_json::to_string(&unwrapped)
                .map_err(|e| format!("Serialize component value: {}", e))?;
            registry.patch_component(world, entity, component, &json_str)?;

            Ok(Some(format!(
                "Modified entity {} component '{}'",
                entity_id, component
            )))
        }

        AgentAction::RemoveComponent {
            entity_id,
            component,
        } => {
            let entity = resolve_entity(world, *entity_id)?;
            registry.remove_component_by_name(world, entity, component)?;
            Ok(Some(format!(
                "Removed '{}' from entity {}",
                component, entity_id
            )))
        }

        AgentAction::DespawnEntity { entity_id } => {
            let entity = resolve_entity(world, *entity_id)?;
            world.despawn(entity);
            Ok(Some(format!("Despawned entity {}", entity_id)))
        }

        AgentAction::DefineComponent { name, schema } => {
            let dyn_schema = big_head_ecs::dynamic_component::parse_schema_from_json(schema)
                .map_err(|e| format!("DefineComponent: invalid schema: {}", e))?;
            registry.define_dynamic_component(world, name, dyn_schema)?;
            Ok(Some(format!("Defined dynamic component '{}'", name)))
        }

        AgentAction::UndefineComponent { name } => {
            registry.undefine_dynamic_component(world, name)?;
            Ok(Some(format!("Undefined dynamic component '{}'", name)))
        }

        AgentAction::DefineSystem { .. } => Ok(None),

        AgentAction::ExecuteSystem { .. } => Ok(None),

        AgentAction::UpdateSystem { .. } | AgentAction::RemoveSystem { .. } => Ok(None),

        AgentAction::ExecuteCli { .. } => Ok(None),

        AgentAction::ShardContent { .. } => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::AgentRuntime;
    use bevy_ecs::component::Component;
    use big_head_ecs::dynamic_component;
    use big_head_ecs::schema::{DynamicSchema, FieldType};

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

    fn review_state_schema() -> DynamicSchema {
        let mut fields = BTreeMap::new();
        fields.insert("reviewer".to_string(), FieldType::String);
        fields.insert("approved".to_string(), FieldType::Boolean);
        fields.insert("notes".to_string(), FieldType::String);
        DynamicSchema { fields }
    }

    #[test]
    fn test_apply_modify_action() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime.register_component_action::<TestHealth>("TestHealth");

        let entity = runtime.world.spawn(TestHealth(50.0)).id();
        let entity_id = entity.index().index() as u64;

        let action = AgentAction::ModifyComponent {
            entity_id,
            component: "TestHealth".into(),
            value: serde_json::json!(75.0),
        };

        let result = apply_action(&mut runtime.world, &action, &mut runtime.component_registry);
        assert!(result.is_ok());
        assert_eq!(runtime.world.get::<TestHealth>(entity).unwrap().0, 75.0);
    }

    #[test]
    fn test_apply_spawn_action() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime.register_component_action::<TestHealth>("TestHealth");
        runtime.register_component_action::<TestName>("TestName");

        let action = AgentAction::SpawnEntity {
            components: vec![
                big_head_ai::ComponentValue {
                    name: "TestHealth".into(),
                    value: serde_json::json!(100.0),
                },
                big_head_ai::ComponentValue {
                    name: "TestName".into(),
                    value: serde_json::json!("hero"),
                },
            ],
            description: Some("A brave hero".into()),
        };

        let result = apply_action(&mut runtime.world, &action, &mut runtime.component_registry);
        assert!(result.is_ok());

        let mut found = false;
        for entity in runtime.world.iter_entities() {
            if let Some(health) = entity.get::<TestHealth>() {
                assert_eq!(health.0, 100.0);
                assert_eq!(entity.get::<TestName>().unwrap().0, "hero");
                found = true;
            }
        }
        assert!(found, "Spawned entity with TestHealth not found");
    }

    #[test]
    fn test_partial_update() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime.register_component_action::<TestSession>("TestSession");

        let entity = runtime
            .world
            .spawn(TestSession {
                title: "my-session".into(),
                status: "idle".into(),
                path: "/tmp/test".into(),
            })
            .id();
        let entity_id = entity.index().index() as u64;

        let action = AgentAction::ModifyComponent {
            entity_id,
            component: "TestSession".into(),
            value: serde_json::json!({"status": "running"}),
        };

        let result = apply_action(&mut runtime.world, &action, &mut runtime.component_registry);
        assert!(result.is_ok(), "Partial update failed: {:?}", result);

        let session = runtime.world.get::<TestSession>(entity).unwrap();
        assert_eq!(session.status, "running");
        assert_eq!(session.title, "my-session");
        assert_eq!(session.path, "/tmp/test");
    }

    #[test]
    fn test_modify_component_accepts_stringified_json_value() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "ReviewState", review_state_schema())
            .unwrap();
        let entity = runtime.world.spawn_empty().id();
        let entity_id = entity.index().index() as u64;

        let action = AgentAction::ModifyComponent {
            entity_id,
            component: "ReviewState".into(),
            value: serde_json::Value::String(
                r#"{"reviewer":"alice","approved":true,"notes":"Looks good"}"#.into(),
            ),
        };

        apply_action(&mut runtime.world, &action, &mut runtime.component_registry).unwrap();

        let id = runtime
            .component_registry
            .dynamic_store()
            .get_id("ReviewState")
            .unwrap();
        let read_back = dynamic_component::read(&runtime.world, entity, id).unwrap();
        assert_eq!(
            read_back.get("reviewer").unwrap(),
            &serde_json::json!("alice")
        );
        assert_eq!(read_back.get("approved").unwrap(), &serde_json::json!(true));
        assert_eq!(
            read_back.get("notes").unwrap(),
            &serde_json::json!("Looks good")
        );
    }

    #[test]
    fn test_apply_action_dispatches_to_dynamic() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "ReviewState", review_state_schema())
            .unwrap();

        let action = AgentAction::SpawnEntity {
            components: vec![big_head_ai::ComponentValue {
                name: "ReviewState".into(),
                value: serde_json::json!({
                    "reviewer": "alice",
                    "approved": true,
                    "notes": "ok"
                }),
            }],
            description: None,
        };
        apply_action(&mut runtime.world, &action, &mut runtime.component_registry).unwrap();

        let id = runtime
            .component_registry
            .dynamic_store()
            .get_id("ReviewState")
            .unwrap();
        let mut found = false;
        for archetype in runtime.world.archetypes().iter() {
            if !archetype.components().iter().any(|c| *c == id) {
                continue;
            }
            for arch_entity in archetype.entities() {
                let e = arch_entity.id();
                if let Some(v) = dynamic_component::read(&runtime.world, e, id) {
                    assert_eq!(v.get("reviewer").unwrap(), &serde_json::json!("alice"));
                    found = true;
                }
            }
        }
        assert!(
            found,
            "SpawnEntity should have inserted a dynamic ReviewState"
        );
    }

    #[test]
    fn test_spawn_entity_rolls_back_when_dynamic_insert_fails() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "ReviewState", review_state_schema())
            .unwrap();
        let before = runtime.world.iter_entities().count();

        let action = AgentAction::SpawnEntity {
            components: vec![big_head_ai::ComponentValue {
                name: "ReviewState".into(),
                value: serde_json::json!({
                    "reviewer": "alice",
                    "approved": "true"
                }),
            }],
            description: None,
        };

        let err = apply_action(&mut runtime.world, &action, &mut runtime.component_registry)
            .expect_err("invalid dynamic component should fail");

        assert!(err.contains("rolled back entity"));
        assert!(err.contains("missing field 'notes'"));
        assert!(err.contains("field 'approved': expected boolean"));
        assert_eq!(runtime.world.iter_entities().count(), before);
    }

    #[test]
    fn test_apply_action_define_component() {
        let mut runtime = AgentRuntime::new_for_test();
        let action = AgentAction::DefineComponent {
            name: "Deadline".into(),
            schema: serde_json::json!({"date": "string", "urgent": "boolean"}),
        };
        let result = apply_action(&mut runtime.world, &action, &mut runtime.component_registry);
        assert!(result.is_ok());
        let msg = result.unwrap().unwrap();
        assert!(msg.contains("Deadline"));

        assert!(
            runtime
                .component_registry
                .dynamic_store()
                .get_id("Deadline")
                .is_some()
        );
    }

    #[test]
    fn test_mixed_rust_and_dynamic_in_one_spawn() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime.register_component_action::<TestHealth>("TestHealth");
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "Tag", {
                let mut f = BTreeMap::new();
                f.insert("label".to_string(), FieldType::String);
                DynamicSchema { fields: f }
            })
            .unwrap();

        let action = AgentAction::SpawnEntity {
            components: vec![
                big_head_ai::ComponentValue {
                    name: "TestHealth".into(),
                    value: serde_json::json!(42.0),
                },
                big_head_ai::ComponentValue {
                    name: "Tag".into(),
                    value: serde_json::json!({"label": "hero"}),
                },
            ],
            description: None,
        };
        apply_action(&mut runtime.world, &action, &mut runtime.component_registry).unwrap();

        let tag_id = runtime
            .component_registry
            .dynamic_store()
            .get_id("Tag")
            .unwrap();
        let mut found = false;
        for archetype in runtime.world.archetypes().iter() {
            if !archetype.components().iter().any(|c| *c == tag_id) {
                continue;
            }
            for arch_entity in archetype.entities() {
                let e = arch_entity.id();
                assert!(runtime.world.get::<TestHealth>(e).is_some());
                let tag = dynamic_component::read(&runtime.world, e, tag_id).unwrap();
                assert_eq!(tag.get("label").unwrap(), &serde_json::json!("hero"));
                found = true;
            }
        }
        assert!(found);
    }
}
