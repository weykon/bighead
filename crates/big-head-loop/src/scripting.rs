//! Rhai 脚本引擎 —— 从 [`big_head_script`] 重新导出。
pub use big_head_script::scripting::*;

/// Bounded synchronous programs used by scene and business activity runners.
pub(crate) fn bounded_ecs_engine() -> std::sync::Arc<rhai::Engine> {
    let mut engine = build_ecs_engine();
    engine.set_max_operations(100_000);
    engine.set_max_array_size(4096);
    engine.set_max_map_size(1024);
    engine.set_max_string_size(65536);
    engine.set_max_expr_depths(64, 32);
    std::sync::Arc::new(engine)
}

/// Runtime services share their tool contract with Rhai, without a crate cycle.
pub fn build_ecs_engine() -> rhai::Engine {
    let mut engine = big_head_script::build_ecs_engine();
    for name in [
        "define_lens_app",
        "run_lens_app",
        "inspect_lens_apps",
        "describe_lens_contract",
        "define_lens_binding",
        "inspect_lens_bindings",
        "discover_lenses",
        "inspect_world_shape",
    ] {
        register_world_function(&mut engine, name, move |world, registry, input| {
            if name == "inspect_world_shape" {
                return crate::world_shape::inspect(world, registry, &input);
            }
            let call = big_head_ai::ToolCall {
                id: "rhai-host".into(),
                name: name.into(),
                input,
            };
            let output = match name {
                "define_lens_binding" | "inspect_lens_bindings" => {
                    crate::lens_bindings::dispatch(world, registry, &call)
                }
                "discover_lenses" => crate::lens_learning::dispatch(world, registry, &call),
                _ => crate::tools::lens_app::dispatch(world, registry, &call),
            }?;
            serde_json::from_str(&output).map_err(|e| format!("invalid service response: {e}"))
        });
    }
    engine
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentRuntime;
    use bevy_ecs::component::Component;
    use big_head_ecs::schema::{DynamicSchema, FieldType};
    use rhai::{Array, Dynamic, Engine, Map};
    use std::collections::BTreeMap;

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestHealth(f32);

    fn setup_runtime_with_health() -> AgentRuntime {
        let mut runtime = AgentRuntime::new_for_test();
        runtime.register_component_action::<TestHealth>("TestHealth");
        runtime
    }

    fn engine() -> Engine {
        build_ecs_engine()
    }

    fn eval(rt: &mut AgentRuntime, script: &str) -> Result<Dynamic, Box<rhai::EvalAltResult>> {
        eval_with_world(
            &rt.rhai_engine,
            script,
            &mut rt.world,
            &mut rt.component_registry,
            &mut rt.system_registry,
            None,
            None,
        )
    }

    #[test]
    fn failed_rhai_writes_stop_execution_and_spawn_does_not_leave_partial_entities() {
        let mut rt = AgentRuntime::new_for_test();
        let _ = eval(&mut rt, r#"define_component("Good", #{value:"integer"});"#).unwrap();
        let before = rt.world.entities().len();
        let failed = eval(
            &mut rt,
            r#"spawn_entity(#{Good:#{value:1}, Unknown:#{}}); define_component("ShouldNotExist", #{});"#,
        );
        assert!(failed.unwrap_err().to_string().contains("rolled back"));
        assert_eq!(before, rt.world.entities().len());
        assert!(
            rt.component_registry
                .lookup_component_id(&rt.world, "ShouldNotExist")
                .is_none()
        );
        let id = eval(&mut rt, r#"spawn_entity(#{Good:#{value:1}})"#)
            .unwrap()
            .as_int()
            .unwrap();
        assert!(
            eval(
                &mut rt,
                &format!(r#"set_component({id}, "Good", #{{value:"bad"}});"#)
            )
            .is_err()
        );
        assert_eq!(
            eval(&mut rt, &format!(r#"get_component({id}, "Good").value"#))
                .unwrap()
                .as_int()
                .unwrap(),
            1
        );
        for script in [
            r#"query("Unknown")"#,
            r#"entities_with("Unknown")"#,
            r#"remove_component(999999, "Good")"#,
            r#"define_component("Good", #{})"#,
            r#"push_spark("work")"#,
            r#"despawn(999999)"#,
        ] {
            assert!(eval(&mut rt, script).is_err(), "{script}");
        }
    }

    #[test]
    fn rhai_lens_services_share_contract_and_named_composition_without_nested_eval() {
        let mut rt = AgentRuntime::new_for_test();
        let result = eval(&mut rt, r#"
            define_component("Topic", #{title:"string"});
            define_component("TopicInsight", #{});
            let e = spawn_entity(#{Topic:#{title:"orphan"}});
            define_lens_app(#{definition:#{id:"orphans",version:1,purpose:"Find orphan topics",reads:["Topic"],stage:#{kind:"archetype_without",component:"TopicInsight"}}});
            define_lens_app(#{definition:#{id:"titles",version:"1",purpose:"Project orphan titles",reads:[],stage:#{kind:"sequence",stages:[#{kind:"lens",id:"orphans",version:1},#{kind:"project",fields:#{title:"/Topic/title"}}]}}});
            let result = run_lens_app(#{id:"titles"});
            if result.data.len() != 1 { throw "missing orphan"; }
            set_component(e, "TopicInsight", #{});
            run_lens_app(#{id:"titles"})
        "#).unwrap();
        assert_eq!(dynamic_to_json(result)["data"], serde_json::json!([]));
        assert!(
            eval(
                &mut rt,
                r#"define_lens_app(#{definition:#{id:"bad",version:1}})"#
            )
            .is_err()
        );
        assert!(rt.spark_queue.is_empty());
    }

    #[test]
    fn tick_return_state_reaches_report_and_invalid_contract_becomes_error() {
        let mut rt = AgentRuntime::new_for_test();
        let entity = rt
            .world
            .spawn(big_head_ecs::TickAction {
                enabled: true,
                script:
                    r#"#{tick_observation:#{state_key:"healthy-v1",summary:"stable",urgent:false}}"#
                        .into(),
                cooldown_secs: 0,
                last_run: 0,
            })
            .id();
        let reports = rt.evaluate_tick_actions();
        let report = reports.iter().find(|r| r.entity == entity).unwrap();
        assert!(report.error.is_none());
        assert_eq!(report.state.as_ref().unwrap().state_key, "healthy-v1");
        rt.world
            .get_mut::<big_head_ecs::TickAction>(entity)
            .unwrap()
            .script = r#"#{tick_observation:#{state_key:""}}"#.into();
        let reports = rt.evaluate_tick_actions();
        assert!(
            reports
                .iter()
                .find(|r| r.entity == entity)
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("state_key")
        );
        rt.world
            .get_mut::<big_head_ecs::TickAction>(entity)
            .unwrap()
            .script = "log(\"legacy\");".into();
        let reports = rt.evaluate_tick_actions();
        let report = reports.iter().find(|r| r.entity == entity).unwrap();
        assert!(report.error.is_none());
        assert!(report.state.is_none());
    }

    #[test]
    fn test_all_entities() {
        let mut runtime = setup_runtime_with_health();
        runtime.world.spawn(TestHealth(100.0));
        runtime.world.spawn(TestHealth(50.0));

        let eng = engine();
        let result = eval_with_world(
            &eng,
            "all_entities()",
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let arr: Array = result.cast();
        assert!(arr.len() >= 2);
    }

    #[test]
    fn test_get_set_component() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "Score", {
                let mut f = BTreeMap::new();
                f.insert("value".to_string(), FieldType::Number);
                DynamicSchema { fields: f }
            })
            .unwrap();

        let entity = runtime.world.spawn_empty().id();
        runtime
            .component_registry
            .insert_dynamic(
                &mut runtime.world,
                entity,
                "Score",
                serde_json::json!({"value": 42.0}),
            )
            .unwrap();
        let eid = entity.index().index() as i64;

        let eng = engine();
        let script = format!(r#"get_component({}, "Score")"#, eid);
        let result = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let map: Map = result.cast();
        assert_eq!(map.get("value").unwrap().as_float().unwrap(), 42.0);

        let script = format!(r#"set_component({}, "Score", #{{"value": 99.0}})"#, eid);
        let _ = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        let score_id = runtime
            .component_registry
            .dynamic_store()
            .get_id("Score")
            .unwrap();
        let val = big_head_ecs::dynamic_component::read(&runtime.world, entity, score_id).unwrap();
        assert_eq!(val["value"], 99.0);
    }

    #[test]
    fn test_spawn_from_rhai() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "Tag", {
                let mut f = BTreeMap::new();
                f.insert("label".to_string(), FieldType::String);
                DynamicSchema { fields: f }
            })
            .unwrap();

        let eng = engine();
        let result = eval_with_world(
            &eng,
            r#"spawn_entity(#{"Tag": #{"label": "hero"}})"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let eid = result.as_int().unwrap();
        assert!(eid >= 0);

        let tag_id = runtime
            .component_registry
            .dynamic_store()
            .get_id("Tag")
            .unwrap();
        let entity = big_head_ecs::resolve_entity(&runtime.world, eid as u64).unwrap();
        let val = big_head_ecs::dynamic_component::read(&runtime.world, entity, tag_id).unwrap();
        assert_eq!(val["label"], "hero");
    }

    #[test]
    fn test_despawn_from_rhai() {
        let mut runtime = setup_runtime_with_health();
        let entity = runtime.world.spawn(TestHealth(10.0)).id();
        let eid = entity.index().index() as i64;

        let eng = engine();
        let script = format!("despawn({})", eid);
        let _ = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        assert!(!runtime.world.entities().contains(entity));
    }

    #[test]
    fn test_entities_with() {
        let mut runtime = setup_runtime_with_health();
        runtime.world.spawn(TestHealth(100.0));

        let eng = engine();
        let result = eval_with_world(
            &eng,
            r#"entities_with("TestHealth")"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let arr: Array = result.cast();
        assert!(!arr.is_empty());
    }

    #[test]
    fn test_has_component() {
        let mut runtime = setup_runtime_with_health();
        let entity = runtime.world.spawn(TestHealth(50.0)).id();
        let eid = entity.index().index() as i64;

        let eng = engine();
        let script = format!(r#"has_component({}, "TestHealth")"#, eid);
        let result = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        assert!(result.as_bool().unwrap());

        let script2 = format!(r#"has_component({}, "NonExistent")"#, eid);
        let result2 = eval_with_world(
            &eng,
            &script2,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        assert!(!result2.as_bool().unwrap());
    }

    #[test]
    fn test_query_dsl() {
        let mut runtime = setup_runtime_with_health();
        runtime.world.spawn(TestHealth(77.0));

        let eng = engine();
        let result = eval_with_world(
            &eng,
            r#"query("TestHealth")"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let arr: Array = result.cast();
        assert!(!arr.is_empty());
    }

    #[test]
    fn test_dynamic_component_access() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .component_registry
            .define_dynamic_component(&mut runtime.world, "Note", {
                let mut f = BTreeMap::new();
                f.insert("text".to_string(), FieldType::String);
                f.insert("priority".to_string(), FieldType::Integer);
                DynamicSchema { fields: f }
            })
            .unwrap();

        let entity = runtime.world.spawn_empty().id();
        runtime
            .component_registry
            .insert_dynamic(
                &mut runtime.world,
                entity,
                "Note",
                serde_json::json!({"text": "hello", "priority": 1}),
            )
            .unwrap();
        let eid = entity.index().index() as i64;

        let eng = engine();

        let script = format!(r#"get_component({}, "Note")"#, eid);
        let result = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let map: Map = result.cast();
        assert_eq!(
            map.get("text").unwrap().clone().into_string().unwrap(),
            "hello"
        );
        assert_eq!(map.get("priority").unwrap().as_int().unwrap(), 1);

        let script = format!(r#"set_component({}, "Note", #{{"priority": 5}})"#, eid);
        let _ = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        let note_id = runtime
            .component_registry
            .dynamic_store()
            .get_id("Note")
            .unwrap();
        let val = big_head_ecs::dynamic_component::read(&runtime.world, entity, note_id).unwrap();
        assert_eq!(val["priority"], 5);
        assert_eq!(val["text"], "hello");
    }

    #[test]
    fn test_syntax_error() {
        let eng = engine();
        let result = check_syntax(&eng, "fn foo( { }");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("syntax error"));
    }

    #[test]
    fn test_define_component_from_rhai() {
        let mut runtime = AgentRuntime::new_for_test();
        let eng = engine();

        let _ = eval_with_world(
            &eng,
            r#"define_component("Priority", #{"level": "integer", "label": "string"})"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        assert!(
            runtime
                .component_registry
                .dynamic_store()
                .get_id("Priority")
                .is_some()
        );

        let result = eval_with_world(
            &eng,
            r#"spawn_entity(#{"Priority": #{"level": 3, "label": "high"}})"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let eid = result.as_int().unwrap();

        let script = format!(r#"get_component({}, "Priority")"#, eid);
        let result = eval_with_world(
            &eng,
            &script,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();
        let map: Map = result.cast();
        assert_eq!(map.get("level").unwrap().as_int().unwrap(), 3);
        assert_eq!(
            map.get("label").unwrap().clone().into_string().unwrap(),
            "high"
        );
    }

    #[test]
    fn test_define_system_from_rhai() {
        let mut runtime = AgentRuntime::new_for_test();
        let eng = engine();

        let _ = eval_with_world(
            &eng,
            r#"define_system("greeter", "log(\"hello from greeter\")")"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        assert!(runtime.system_registry.get("greeter").is_some());
    }

    #[test]
    fn test_define_tool_from_rhai() {
        let mut runtime = AgentRuntime::new_for_test();
        let eng = engine();

        let _ = eval_with_world(
            &eng,
            r#"define_tool("avg_health", "Calculate average health", "let total = 0.0; for e in entities_with(\"TestHealth\") { total += get_component(e, \"TestHealth\").value; } log(\"avg: \" + (total / all_entities().len()));")"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        let sys = runtime.system_registry.get("avg_health").unwrap();
        assert!(sys.is_tool);
        assert_eq!(sys.description, "Calculate average health");

        let tools = runtime.system_registry.tool_definitions();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "avg_health");
        assert_eq!(tools[0].description, "Calculate average health");

        let _ = eval_with_world(
            &eng,
            r#"define_system("hidden", "log(\"hidden\")")"#,
            &mut runtime.world,
            &mut runtime.component_registry,
            &mut runtime.system_registry,
            None,
            None,
        )
        .unwrap();

        let tools = runtime.system_registry.tool_definitions();
        assert_eq!(tools.len(), 1);
    }
}
