//! Local Lens applications share the ECS persistence and audit lifecycle.
use bevy_ecs::world::World;
use big_head_ai::{ToolCall, ToolDefinition};
use big_head_ecs::{ComponentActionRegistry, lens::LensApplication};
use serde_json::{Value, json};

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    let mut definitions = vec![
        ToolDefinition { name:"define_lens_app".into(), description:"Create/update a persistent, read-only context Lens. definition requires id, version, purpose, reads (component names), stage. Stages: identity; archetype_with {component}; archetype_without {component}; filter {pointer,equals}; project {fields: {name: JSON_pointer}}; take {limit}; sequence {stages}; parallel {branches: {name:stage}}; lens {id,version}. Sequence composes presence tests as AND. Membership is captured from ECS, survives project, and invalidates cache on add/remove. Selector components must be registered; their values are NOT read unless in reads. Input universe is entities with at least one reads component across the composed lenses (max 256); without does not scan the whole world. Update requires expected_version. Source input values map component names to their values. Cycles and stale dependencies are rejected. See run_system('guide_query_dsl') for component composition examples.".into(), input_schema:json!({"type":"object","properties":{"definition":{"type":"object"},"expected_version":{"type":"string"}},"required":["definition"]}) },
        ToolDefinition { name:"run_lens_app".into(), description:"Run a Lens over current ECS facts. No business mutation or Spark creation. Returns observation data and provenance, never an assertion of business completion. Stores a bounded execution receipt. details=true includes the runtime trace; false keeps it out of conversational context.".into(), input_schema:json!({"type":"object","properties":{"id":{"type":"string"},"details":{"type":"boolean"}},"required":["id"]}) },
        ToolDefinition { name:"inspect_lens_apps".into(), description:"List local Lens definitions and latest execution receipts. An optional id returns the complete definition and last receipt.".into(), input_schema:json!({"type":"object","properties":{"id":{"type":"string"}}}) },
    ];
    definitions[0].input_schema = crate::lens_contract::schema();
    definitions.push(ToolDefinition {
        name: "describe_lens_contract".into(),
        description: "Read-only Lens contract doctor. Without input returns full schema and example. With input (the entire define_lens_app request), reports validation/precondition errors and canonical normalized_input; does not publish or execute. Only nonnegative integer versions are normalized to strings; no inferred stages or silently relocated fields.".into(),
        input_schema: json!({"type":"object","properties":{"input":{"type":"object"}},"additionalProperties":false}),
    });
    definitions
}

pub(crate) fn dispatch(
    world: &mut World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> Result<String, String> {
    if call.name == "describe_lens_contract" {
        let catalog = crate::lens_runtime::definition_catalog(world)?;
        return Ok(crate::lens_contract::diagnose(call.input.get("input"), &catalog).to_string());
    }
    let (entries, mut catalog) = crate::lens_runtime::catalog(world)?;
    if call.name == "inspect_lens_apps" {
        return if let Some(id) = call.input.get("id").and_then(Value::as_str) {
            let (_, app) = entries
                .iter()
                .find(|(_, a)| a.id == id)
                .ok_or("unknown Lens")?;
            Ok(json!({"definition":catalog[id],"last_run":serde_json::from_str::<Value>(&app.last_run_json).unwrap_or(Value::Null)}).to_string())
        } else {
            Ok(json!({"applications":catalog.values().map(|d|json!({"id":d.id,"version":d.version,"purpose":d.purpose,"usage":d.usage,"reads":d.reads})).collect::<Vec<_>>()}).to_string())
        };
    }
    if call.name == "define_lens_app" {
        let (request, _, normalizations) = crate::lens_contract::parse(&call.input)?;
        crate::lens_contract::validate_request(&request, &catalog)?;
        let definition = request.definition;
        let id = definition.id.clone();
        catalog.insert(id.clone(), definition);
        let app = LensApplication {
            id: id.clone(),
            definition_json: serde_json::to_string(&catalog[&id]).map_err(|e| e.to_string())?,
            last_run_json: String::new(),
        };
        let entity = if let Some((entity, _)) = entries.iter().find(|(_, a)| a.id == id) {
            world.entity_mut(*entity).insert(app);
            *entity
        } else {
            world.spawn(app).id()
        };
        return Ok(
            json!({"id":id,"entity_id":entity.index().index(),"version":catalog[&id].version,"normalizations":normalizations})
                .to_string(),
        );
    }
    let id = call
        .input
        .get("id")
        .and_then(Value::as_str)
        .ok_or("id required")?;
    let receipt = crate::lens_runtime::observe(world, registry, id, &catalog)?;
    let serialized = receipt.to_string();
    let (entity, _) = entries
        .iter()
        .find(|(_, a)| a.id == id)
        .ok_or("Lens disappeared")?;
    world
        .get_mut::<LensApplication>(*entity)
        .unwrap()
        .last_run_json = serialized.clone();
    if call
        .input
        .get("details")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Ok(serialized)
    } else {
        Ok(json!({"lens_id":id,"kind":"observation","run_id":receipt["run_id"],"observed_at":receipt["observed_at"],"data":receipt["data"]}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(name: &str, input: Value) -> ToolCall {
        ToolCall {
            id: "lens-test".into(),
            name: name.into(),
            input,
        }
    }
    fn app() -> Value {
        json!({"id":"weather.schedule","version":"1","purpose":"Inspect weather refresh schedules","reads":["TickAction"],"stage":{"kind":"sequence","stages":[{"kind":"filter","pointer":"/TickAction/cooldown_secs","equals":21600},{"kind":"project","fields":{"last_run":"/TickAction/last_run","script":"/TickAction/script"}}]}})
    }

    #[test]
    fn doctor_and_publish_agree_without_diagnostic_mutation() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let request = json!({"definition":app()});
        let before = rt.world.entities().len();
        let diagnosis: Value = serde_json::from_str(
            &dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("describe_lens_contract", json!({"input":request})),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(diagnosis["valid"], true);
        assert_eq!(rt.world.entities().len(), before);
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("define_lens_app", diagnosis["normalized_input"].clone()),
        )
        .unwrap();
        let after = rt.world.entities().len();
        let diagnosis: Value = serde_json::from_str(
            &dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("describe_lens_contract", json!({"input":request})),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(diagnosis["valid"], false);
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("define_lens_app", request)
            )
            .is_err()
        );
        assert_eq!(rt.world.entities().len(), after);
    }

    #[test]
    fn marker_tool_lifecycle_matches_query_and_lens_across_restore() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let (_, failed, _) = crate::tools::define::dispatch_define_component(
            &mut rt.world,
            &mut rt.component_registry,
            &call("define_component", json!({"name":"AccidentalMarker"})),
        );
        assert!(failed);
        assert!(
            rt.component_registry
                .lookup_component_id(&rt.world, "AccidentalMarker")
                .is_none()
        );
        for (name, schema) in [
            ("DemoWork", json!({"title":"string"})),
            ("DemoNeedsReview", json!({})),
            ("DemoCompleted", json!({})),
        ] {
            let (output, failed, _) = crate::tools::define::dispatch_define_component(
                &mut rt.world,
                &mut rt.component_registry,
                &call("define_component", json!({"name":name,"schema":schema})),
            );
            assert!(!failed, "{output}");
        }
        let (output, failed, _) = crate::tools::write::dispatch_write_action(
            &mut rt.world,
            &mut rt.component_registry,
            &call(
                "spawn_entity",
                json!({"components":[
                    {"name":"DemoWork","value":{"title":"review"}},
                    {"name":"DemoNeedsReview","value":{}}
                ]}),
            ),
        );
        assert!(!failed, "{output}");
        let query = big_head_query::parse_query(
            &rt.world,
            "DemoWork where WITH(DemoWork) AND WITH(DemoNeedsReview) AND WITHOUT(DemoCompleted)",
        )
        .unwrap();
        let matched = big_head_query::execute_dynamic_query(&rt.world, &query);
        assert_eq!(matched.len(), 1);
        let entity = matched[0].entity;
        dispatch(&mut rt.world, &rt.component_registry, &call("define_lens_app", json!({"definition":{
            "id":"demo.review", "version":"1", "purpose":"review work", "reads":["DemoWork"],
            "stage":{"kind":"sequence","stages":[
                {"kind":"archetype_with","component":"DemoWork"},
                {"kind":"archetype_with","component":"DemoNeedsReview"},
                {"kind":"archetype_without","component":"DemoCompleted"},
                {"kind":"project","fields":{"title":"/DemoWork/title"}}
            ]}
        }}))).unwrap();
        let observe = |rt: &mut crate::AgentRuntime| -> Value {
            serde_json::from_str(
                &dispatch(
                    &mut rt.world,
                    &rt.component_registry,
                    &call("run_lens_app", json!({"id":"demo.review"})),
                )
                .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            observe(&mut rt)["data"][0]["value"],
            json!({"title":"review"})
        );
        for (tool, expected) in [("modify_component", 0), ("remove_component", 1)] {
            let (output, failed, _) = crate::tools::write::dispatch_write_action(
                &mut rt.world,
                &mut rt.component_registry,
                &call(
                    tool,
                    json!({"entity_id":entity.index().index(),"component":"DemoCompleted","value":{}}),
                ),
            );
            assert!(!failed, "{output}");
            assert_eq!(
                big_head_query::execute_dynamic_query(&rt.world, &query).len(),
                expected
            );
            assert_eq!(observe(&mut rt)["data"].as_array().unwrap().len(), expected);
        }
        let before = observe(&mut rt);
        let dir = std::env::temp_dir().join(format!("marker-checkpoint-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("checkpoint.json");
        rt.checkpoint(&path).unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&path).unwrap();
        assert_eq!(observe(&mut restored)["data"], before["data"]);
        assert!(restored.spark_queue.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn current_facts_and_receipts_survive_checkpoint() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let tick = rt
            .world
            .spawn(big_head_ecs::TickAction {
                enabled: true,
                cooldown_secs: 21600,
                last_run: 12,
                script: "log(\"weather\");".into(),
            })
            .id();
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("define_lens_app", json!({"definition":app()})),
        )
        .unwrap();
        let output = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("run_lens_app", json!({"id":"weather.schedule"})),
        )
        .unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["data"][0]["value"]["last_run"], 12);
        assert!(result.get("inputs").is_none());
        assert_eq!(
            rt.world
                .get::<big_head_ecs::TickAction>(tick)
                .unwrap()
                .last_run,
            12
        );
        let version = result["data"][0]["source_version"].clone();
        rt.world
            .get_mut::<big_head_ecs::TickAction>(tick)
            .unwrap()
            .last_run = 34;
        let second: Value = serde_json::from_str(
            &dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("run_lens_app", json!({"id":"weather.schedule"})),
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(version, second["data"][0]["source_version"]);
        let dir = std::env::temp_dir().join(format!("lens-checkpoint-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("checkpoint.json");
        rt.checkpoint(&path).unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&path).unwrap();
        let detail: Value = serde_json::from_str(
            &dispatch(
                &mut restored.world,
                &restored.component_registry,
                &call("inspect_lens_apps", json!({"id":"weather.schedule"})),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(detail["last_run"]["run_id"], second["run_id"]);
        let third: Value = serde_json::from_str(
            &dispatch(
                &mut restored.world,
                &restored.component_registry,
                &call("run_lens_app", json!({"id":"weather.schedule"})),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            third["data"][0]["source_id"],
            second["data"][0]["source_id"]
        );
        assert_eq!(third["data"][0]["value"]["last_run"], 34);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn stale_update_and_unknown_source_are_explicit_errors() {
        let mut rt = crate::AgentRuntime::new_for_test();
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("define_lens_app", json!({"definition":app()})),
        )
        .unwrap();
        let mut changed = app();
        changed["version"] = json!("2");
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("define_lens_app", json!({"definition":changed}))
            )
            .is_err()
        );
        changed["reads"] = json!(["NonexistentWeatherSource"]);
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "define_lens_app",
                json!({"definition":changed,"expected_version":"1"}),
            ),
        )
        .unwrap();
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("run_lens_app", json!({"id":"weather.schedule"}))
            )
            .unwrap_err()
            .contains("unknown input component")
        );
    }
}
