use super::*;
use std::borrow::Cow;

fn definition(id: &str, source: &str) -> SystemDefinition {
    SystemDefinition {
        id: id.into(),
        description: "fixture".into(),
        system_prompt: Cow::Borrowed(""),
        kind: SystemKind::RhaiScript {
            source: source.into(),
        },
        tool_names: vec![],
        max_iterations: 1,
        is_tool: false,
    }
}

fn call(name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: "fixture".into(),
        name: name.into(),
        input,
    }
}

#[test]
fn every_available_tool_is_discoverable_without_a_semantics_allowlist() {
    let world = World::new();
    let components = ComponentActionRegistry::new();
    let systems = SystemRegistry::new();
    let mut tools = crate::activity::definitions();
    tools.push(ToolDefinition {
        name: "new_dynamic_tool".into(),
        description: "A new domain tool".into(),
        input_schema: json!({"type":"object"}),
    });
    let result = inspect(
        &world,
        &components,
        &systems,
        &tools,
        &json!({"view":"operations","limit":32}),
    )
    .unwrap();
    assert_eq!(result["matching_total"], tools.len());
    for tool in &tools {
        let detail = inspect(
            &world,
            &components,
            &systems,
            &tools,
            &json!({"view":"operations","name":tool.name}),
        )
        .unwrap();
        assert_eq!(detail["entries"][0]["input_schema"], tool.input_schema);
    }
    let allowed: Vec<_> = tools
        .into_iter()
        .filter(|t| t.name == "inspect_activity")
        .collect();
    let result = inspect(
        &world,
        &components,
        &systems,
        &allowed,
        &json!({"view":"operations"}),
    )
    .unwrap();
    assert_eq!(result["matching_total"], 1);
    assert_eq!(result["entries"][0]["name"], "inspect_activity");
}

#[test]
fn hot_update_validates_before_mutation_and_preserves_metadata() {
    let mut reg = SystemRegistry::new();
    let original = definition("behavior", "41 + 1");
    let hash = definition_hash(&original);
    reg.register(original).unwrap();
    let engine = scripting::build_ecs_engine();
    for (expected, source) in [("stale", "123"), (hash.as_str(), "let = ;")] {
        assert!(mutate(&mut reg, &engine, &call("update_system", json!({
            "id":"behavior","expected_definition_hash":expected,"source":source,"reason":"test"
        })), &[]).is_err());
        assert_eq!(definition_hash(reg.get("behavior").unwrap()), hash);
    }
    let (receipt, _) = mutate(&mut reg, &engine, &call("update_system", json!({
        "id":"behavior","expected_definition_hash":hash,"source":"throw \"not executed during update\";","reason":"publish only"
    })), &[]).unwrap();
    assert_eq!(receipt["executed"], false);
    assert_eq!(receipt["behavior_verified"], false);
    let updated = reg.get("behavior").unwrap();
    assert_eq!(updated.description, "fixture");
    assert_eq!(updated.max_iterations, 1);
    assert!(!updated.is_tool);
    assert_ne!(definition_hash(updated), hash);
    let mut metadata_change = updated.clone();
    metadata_change.is_tool = true;
    assert_ne!(definition_hash(&metadata_change), definition_hash(updated));
}

#[test]
fn removal_checks_version_builtins_and_explicit_callers() {
    let mut reg = SystemRegistry::new();
    let mut target = definition("target", "42");
    target.is_tool = true;
    let hash = definition_hash(&target);
    reg.register(target).unwrap();
    let mut caller = definition("caller", "0");
    caller.tool_names = vec!["target".into()];
    reg.register(caller).unwrap();
    let engine = scripting::build_ecs_engine();
    let request = call(
        "remove_system",
        json!({"id":"target","expected_definition_hash":hash,"reason":"retire"}),
    );
    assert!(
        mutate(&mut reg, &engine, &request, &[])
            .unwrap_err()
            .contains("caller")
    );
    assert!(mutate(&mut reg, &engine, &call("update_system",json!({
        "id":"target","expected_definition_hash":hash,"source":"43","is_tool":false,"reason":"hide"
    })), &[]).is_err());
    reg.remove("caller").unwrap();
    assert!(
        mutate(
            &mut reg,
            &engine,
            &call(
                "remove_system",
                json!({
                    "id":"target","expected_definition_hash":"old","reason":"retire"
                })
            ),
            &[]
        )
        .is_err()
    );
    mutate(&mut reg, &engine, &request, &[]).unwrap();
    assert!(reg.get("target").is_none());
    assert!(reg.tool_definitions().is_empty());
    for mut def in [
        big_head_ai::guides::guide_rhai_syntax(),
        definition("llm-stage", ""),
    ] {
        if def.id == "llm-stage" {
            def.kind = SystemKind::Llm;
        }
        let id = def.id.clone();
        let hash = definition_hash(&def);
        reg.register(def).unwrap();
        let err = mutate(
            &mut reg,
            &engine,
            &call(
                "remove_system",
                json!({
                    "id":id,"expected_definition_hash":hash,"reason":"retire"
                }),
            ),
            &[],
        )
        .unwrap_err();
        assert!(err.contains("cannot be hot-edited"));
    }
}

#[test]
fn catalogs_show_real_zero_instance_schemas_and_page_source_without_execution() {
    let mut world = World::new();
    let mut components = ComponentActionRegistry::new();
    components
        .define_dynamic_component(
            &mut world,
            "Marker",
            big_head_ecs::DynamicSchema {
                fields: Default::default(),
            },
        )
        .unwrap();
    let mut systems = SystemRegistry::new();
    systems
        .register(definition("hello", "log(\"你好世界\");"))
        .unwrap();
    let tools = definitions();
    let before_entities = world.entities().len();
    let component = inspect(
        &world,
        &components,
        &systems,
        &tools,
        &json!({"view":"components","name":"Marker"}),
    )
    .unwrap();
    assert_eq!(component["entries"][0]["instances"], 0);
    assert_eq!(component["entries"][0]["presence_only"], true);
    let hash = definition_hash(systems.get("hello").unwrap());
    let page = inspect(&world,&components,&systems,&tools,&json!({
        "view":"systems","name":"hello","source_offset":5,"source_limit":2,"expected_definition_hash":hash
    })).unwrap();
    assert_eq!(page["entries"][0]["source"]["text"], "你好");
    assert_eq!(page["entries"][0]["source"]["next_offset"], 7);
    assert_eq!(page["entries"][0]["source"]["complete"], false);
    assert_eq!(page["entries"][0]["can_run_system"], false);
    assert_eq!(page["entries"][0]["can_update"], true);
    let page = inspect(
        &world,
        &components,
        &systems,
        &[],
        &json!({"view":"systems","name":"hello"}),
    )
    .unwrap();
    assert_eq!(page["entries"][0]["can_update"], false);
    assert_eq!(world.entities().len(), before_entities);
    assert!(
        inspect(
            &world,
            &components,
            &systems,
            &tools,
            &json!({
                "view":"systems","name":"hello","expected_definition_hash":"stale"
            })
        )
        .is_err()
    );
    assert!(
        inspect(
            &world,
            &components,
            &systems,
            &tools,
            &json!({"view":"operations","limit":0})
        )
        .is_err()
    );
}

#[test]
fn tool_exposure_cannot_shadow_runtime_operations() {
    let mut reg = SystemRegistry::new();
    reg.register(definition("query_entities", "42")).unwrap();
    let hash = definition_hash(reg.get("query_entities").unwrap());
    let err = mutate(&mut reg,&scripting::build_ecs_engine(),&call("update_system",json!({
        "id":"query_entities","expected_definition_hash":hash,"source":"0","is_tool":true,"reason":"expose"
    })), &["query_entities".into()]).unwrap_err();
    assert!(err.contains("collides"));
    assert_eq!(definition_hash(reg.get("query_entities").unwrap()), hash);
}
