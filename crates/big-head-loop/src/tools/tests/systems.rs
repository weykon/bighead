use super::*;

#[tokio::test]
async fn hot_system_lifecycle_runs_new_code_and_survives_checkpoint() {
    let mut rt = AgentRuntime::new_for_test();
    rt.component_registry
        .define_dynamic_component(
            &mut rt.world,
            "HotCounter",
            big_head_ecs::dynamic_component::parse_schema_from_json(
                &serde_json::json!({"count":"integer"}),
            )
            .unwrap(),
        )
        .unwrap();
    let entity = rt.world.spawn_empty().id();
    rt.component_registry
        .insert_component(&mut rt.world, entity, "HotCounter", r#"{"count":0}"#)
        .unwrap();
    let source = |count| {
        format!(
            "set_component({}, \"HotCounter\", #{{count: {count}}});",
            entity.index().index()
        )
    };
    let tx = crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "test",
        &[],
        None,
        None,
    );
    let mut exec = make_executor(&mut rt);
    exec.tx_id = Some(tx.tx_id.clone());
    let call = |name: &str, input| ToolCall {
        id: name.into(),
        name: name.into(),
        input,
    };
    let result = exec
        .execute(&call(
            "define_tool",
            serde_json::json!({
                "id":"hot_counter","source":source(1),"description":"set fixture value"
            }),
        ))
        .await;
    assert!(!result.is_error, "{}", result.output);
    let inspect = exec
        .execute(&call(
            "inspect_ecs_capabilities",
            serde_json::json!({
                "view":"systems","name":"hot_counter"
            }),
        ))
        .await;
    let detail: serde_json::Value = serde_json::from_str(&inspect.output).unwrap();
    let hash = detail["entries"][0]["definition_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let result = exec
        .execute(&call("run_system", serde_json::json!({"id":"hot_counter"})))
        .await;
    assert!(!result.is_error, "{}", result.output);
    let cid = exec.registry.dynamic_store().get_id("HotCounter").unwrap();
    assert_eq!(
        big_head_ecs::dynamic_component::read(exec.world, entity, cid).unwrap()["count"],
        1
    );
    let result = exec.execute(&call("update_system",serde_json::json!({
        "id":"hot_counter","expected_definition_hash":hash,"source":source(2).replace("count: 2", "count: args.count"),"reason":"exercise new behavior"
    }))).await;
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(
        big_head_ecs::dynamic_component::read(exec.world, entity, cid).unwrap()["count"],
        1,
        "publishing code must not execute it"
    );
    let result = exec
        .execute(&call(
            "hot_counter",
            serde_json::json!({"args":{"count":2}}),
        ))
        .await;
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(
        big_head_ecs::dynamic_component::read(exec.world, entity, cid).unwrap()["count"],
        2
    );
    let updated_hash =
        ecs_capabilities::definition_hash(exec.system_registry.get("hot_counter").unwrap());
    drop(exec);

    let root = std::env::temp_dir().join(format!("big-head-hot-system-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("checkpoint.json");
    crate::persistence::checkpoint(
        &path,
        &rt.world,
        &rt.component_registry,
        &rt.system_registry,
    )
    .unwrap();
    let mut restored = AgentRuntime::new_for_test();
    crate::persistence::restore(
        &path,
        &mut restored.world,
        &mut restored.component_registry,
        &mut restored.system_registry,
    )
    .unwrap();
    assert_eq!(
        ecs_capabilities::definition_hash(restored.system_registry.get("hot_counter").unwrap()),
        updated_hash
    );
    let mut exec = make_executor(&mut rt);
    exec.tx_id = Some(tx.tx_id.clone());
    let result = exec
        .execute(&call(
            "remove_system",
            serde_json::json!({
                "id":"hot_counter","expected_definition_hash":updated_hash,"reason":"end fixture"
            }),
        ))
        .await;
    assert!(!result.is_error, "{}", result.output);
    assert!(
        !exec
            .dynamic_tool_definitions()
            .iter()
            .any(|tool| tool.name == "hot_counter")
    );
    assert!(
        exec.execute(&call("hot_counter", serde_json::json!({"args":{}})))
            .await
            .is_error
    );
    assert_eq!(
        big_head_ecs::dynamic_component::read(exec.world, entity, cid).unwrap()["count"],
        2,
        "removal must not undo prior effects"
    );
    drop(exec);
    crate::persistence::checkpoint(
        &path,
        &rt.world,
        &rt.component_registry,
        &rt.system_registry,
    )
    .unwrap();
    let mut restored = AgentRuntime::new_for_test();
    crate::persistence::restore(
        &path,
        &mut restored.world,
        &mut restored.component_registry,
        &mut restored.system_registry,
    )
    .unwrap();
    assert!(restored.system_registry.get("hot_counter").is_none());
    let mut query = rt.world.query::<&crate::ActionRecord>();
    let update = query
        .iter(&rt.world)
        .find(|record| record.tool_name == "update_system")
        .unwrap();
    assert_eq!(update.kind, "UpdateSystem");
    assert!(update.before_json.is_some());
    assert!(update.after_json.is_some());
    let removal = query
        .iter(&rt.world)
        .find(|record| record.tool_name == "remove_system")
        .unwrap();
    assert_eq!(removal.kind, "RemoveSystem");
    assert!(removal.before_json.is_some());
    assert!(removal.after_json.is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn tool_arguments_are_per_call_and_tool_visibility_is_enforced() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    let call = |name: &str, input| ToolCall {
        id: name.into(),
        name: name.into(),
        input,
    };
    let source =
        "if !args.enabled { throw \"disabled\"; } log((args.n + args.items[0]).to_string());";
    assert!(
        !exec
            .execute(&call(
                "define_tool",
                serde_json::json!({
                    "id":"with_args","source":source
                })
            ))
            .await
            .is_error
    );
    let result = exec
        .execute(&call(
            "with_args",
            serde_json::json!({"args":{"enabled":true,"n":4,"items":[2]}}),
        ))
        .await;
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.contains("6"));
    let invalid = exec
        .execute(&call("with_args", serde_json::json!({"args":"invalid"})))
        .await;
    assert!(invalid.is_error);
    assert!(invalid.output.contains("args must be a JSON object"));
    assert!(
        exec.execute(&call("with_args", serde_json::json!({})))
            .await
            .is_error,
        "args must not leak from the previous invocation"
    );
    let hash = ecs_capabilities::definition_hash(exec.system_registry.get("with_args").unwrap());
    assert!(!exec.execute(&call("update_system",serde_json::json!({
        "id":"with_args","expected_definition_hash":hash,"source":source,"is_tool":false,"reason":"use explicit invocation"
    }))).await.is_error);
    assert!(
        !exec
            .dynamic_tool_definitions()
            .iter()
            .any(|d| d.name == "with_args")
    );
    assert!(
        exec.execute(&call(
            "with_args",
            serde_json::json!({"args":{"enabled":true,"n":4,"items":[2]}})
        ))
        .await
        .is_error
    );
    let result = exec
        .execute(&call(
            "run_system",
            serde_json::json!({"id":"with_args","args":{"enabled":true,"n":7,"items":[3]}}),
        ))
        .await;
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.contains("10"));
}

#[tokio::test]
async fn capability_catalog_respects_stage_tools_and_rejects_shadowing() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    let result = exec
        .execute(&ToolCall {
            id: "collision".into(),
            name: "define_tool".into(),
            input: serde_json::json!({
                "id":"inspect_ecs_capabilities","source":"42"
            }),
        })
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("collides"));
    let allowed = vec!["inspect_ecs_capabilities".into()];
    exec.allowed_tools = Some(&allowed);
    let result = exec
        .execute(&ToolCall {
            id: "catalog".into(),
            name: "inspect_ecs_capabilities".into(),
            input: serde_json::json!({"name":"inspect_ecs_capabilities"}),
        })
        .await;
    assert!(!result.is_error, "{}", result.output);
    let result: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(result["matching_total"], 1);
    assert_eq!(result["entries"][0]["name"], "inspect_ecs_capabilities");
    assert_eq!(result["entries"][0]["available_in_current_context"], true);
    let contract = exec
        .execute(&ToolCall {
            id: "unavailable-contract".into(),
            name: "inspect_ecs_capabilities".into(),
            input: serde_json::json!({"name":"update_system"}),
        })
        .await;
    assert!(!contract.is_error);
    let contract: serde_json::Value = serde_json::from_str(&contract.output).unwrap();
    assert_eq!(
        contract["entries"][0]["available_in_current_context"],
        false
    );
    assert!(
        exec.execute(&ToolCall {
            id: "still-denied".into(),
            name: "update_system".into(),
            input: serde_json::json!({}),
        })
        .await
        .is_error
    );
    assert_eq!(exec.dynamic_tool_definitions().len(), 1);
}

#[tokio::test]
async fn test_executor_records_system_registry_actions() {
    let mut rt = AgentRuntime::new_for_test();
    let tx = crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "test",
        &[],
        None,
        None,
    );

    {
        let mut exec = make_executor(&mut rt);
        exec.tx_id = Some(tx.tx_id.clone());

        let define = exec
            .execute(&ToolCall {
                id: "toolu_define_system".into(),
                name: "define_system".into(),
                input: serde_json::json!({
                    "id": "audit_system",
                    "description": "audit system",
                    "source": "log(\"audit system\");",
                    "max_iterations": 1,
                    "tool_names": []
                }),
            })
            .await;
        assert!(!define.is_error, "define_system failed: {}", define.output);

        let run = exec
            .execute(&ToolCall {
                id: "toolu_run_system".into(),
                name: "run_system".into(),
                input: serde_json::json!({"id": "audit_system"}),
            })
            .await;
        assert!(!run.is_error, "run_system failed: {}", run.output);

        let define_tool = exec
            .execute(&ToolCall {
                id: "toolu_define_tool".into(),
                name: "define_tool".into(),
                input: serde_json::json!({
                    "id": "audit_tool",
                    "description": "audit tool",
                    "source": "log(\"audit tool\");"
                }),
            })
            .await;
        assert!(
            !define_tool.is_error,
            "define_tool failed: {}",
            define_tool.output
        );
    }

    let mut query = rt.world.query::<&crate::ActionRecord>();
    let records = query.iter(&rt.world).collect::<Vec<_>>();
    let define_record = records
        .iter()
        .find(|record| record.tool_call_id == "toolu_define_system")
        .expect("define_system action should be recorded");
    assert_eq!(define_record.kind, "DefineSystem");
    assert_eq!(define_record.component.as_deref(), Some("SystemDefinition"));
    assert!(define_record.before_json.is_none());
    let define_after: serde_json::Value =
        serde_json::from_str(define_record.after_json.as_deref().unwrap()).unwrap();
    assert_eq!(define_after["id"], "audit_system");
    assert_eq!(define_after["is_tool"], false);

    let run_record = records
        .iter()
        .find(|record| record.tool_call_id == "toolu_run_system")
        .expect("run_system action should be recorded");
    assert_eq!(run_record.kind, "RunSystem");
    assert_eq!(run_record.component.as_deref(), Some("SystemDefinition"));
    assert!(run_record.before_json.is_some());
    assert!(run_record.after_json.is_some());

    let tool_record = records
        .iter()
        .find(|record| record.tool_call_id == "toolu_define_tool")
        .expect("define_tool action should be recorded");
    assert_eq!(tool_record.kind, "DefineTool");
    assert_eq!(tool_record.component.as_deref(), Some("SystemDefinition"));
    let tool_after: serde_json::Value =
        serde_json::from_str(tool_record.after_json.as_deref().unwrap()).unwrap();
    assert_eq!(tool_after["id"], "audit_tool");
    assert_eq!(tool_after["is_tool"], true);
}

#[tokio::test]
async fn test_rhai_tool_dispatch() {
    let mut rt = AgentRuntime::new_for_test();

    // 通过脚本引擎定义 Rhai 工具
    crate::scripting::eval_with_world(
        &rt.rhai_engine,
        r#"define_tool("greet", "A greeting tool", "log(\"hello from tool\")")"#,
        &mut rt.world,
        &mut rt.component_registry,
        &mut rt.system_registry,
        None,
        None,
    )
    .unwrap();

    // 验证工具出现在 dynamic_tool_definitions 中
    let mut exec = make_executor(&mut rt);
    let defs = exec.dynamic_tool_definitions();
    assert_eq!(defs.iter().filter(|d| d.name == "greet").count(), 1);
    assert!(defs.iter().any(|d| d.name == "run_lens_app"));

    // 通过执行器调用工具
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "greet".into(),
            input: serde_json::json!({"args": {}}),
        })
        .await;
    assert!(!r.is_error, "Rhai tool dispatch failed: {}", r.output);
    assert!(r.output.contains("hello from tool"));
}
