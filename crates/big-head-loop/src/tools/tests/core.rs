use super::*;

#[tokio::test]
async fn test_spawn_and_modify() {
    let mut rt = AgentRuntime::new_for_test();
    rt.register_component_action::<TestHealth>("TestHealth");
    rt.register_component_action::<TestName>("TestName");

    {
        let mut exec = make_executor(&mut rt);
        let r = exec
            .execute(&ToolCall {
                id: "t1".into(),
                name: "spawn_entity".into(),
                input: serde_json::json!({
                    "components": [
                        {"name": "TestHealth", "value": 42.0},
                        {"name": "TestName", "value": "archer"}
                    ]
                }),
            })
            .await;
        assert!(!r.is_error, "Spawn failed: {}", r.output);
    }

    let mut found = false;
    for entity in rt.world.iter_entities() {
        if let Some(h) = entity.get::<TestHealth>() {
            if h.0 == 42.0 {
                assert_eq!(entity.get::<TestName>().unwrap().0, "archer");
                found = true;
            }
        }
    }
    assert!(found);
}

#[tokio::test]
async fn test_get_world_overview() {
    let mut rt = AgentRuntime::new_for_test();
    rt.register_component_action::<TestHealth>("TestHealth");
    rt.world.spawn(TestHealth(100.0));

    let mut exec = make_executor(&mut rt);
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "get_world_overview".into(),
            input: serde_json::json!({}),
        })
        .await;
    assert!(!r.is_error);
    assert!(r.output.contains("TestHealth"));
}

#[tokio::test]
async fn test_unknown_tool() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "nope".into(),
            input: serde_json::json!({}),
        })
        .await;
    assert!(r.is_error);
    assert!(r.output.contains("Unknown tool"));
}

#[tokio::test]
async fn test_background_cli_registers_and_reconciles_managed_process() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    let result = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "execute_cli".into(),
            input: serde_json::json!({
                "command": "sleep 1",
                "background": true,
                "name": "test-sleeper",
                "purpose": "exercise process supervision",
                "port": 5199
            }),
        })
        .await;
    assert!(
        !result.is_error,
        "background command failed: {}",
        result.output
    );
    assert!(result.output.contains("Managed process:"));
    drop(exec);

    let processes = rt.poll_managed_processes();
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].name, "test-sleeper");
    assert_eq!(processes[0].port, Some(5199));
    assert!(matches!(
        processes[0].state.as_str(),
        "Starting" | "Running"
    ));

    std::thread::sleep(std::time::Duration::from_millis(1_300));
    let processes = rt.poll_managed_processes();
    assert_eq!(processes[0].state, "Exited");
}

#[tokio::test]
async fn test_schema_validation_error_includes_recovery_hint() {
    let mut rt = AgentRuntime::new_for_test();

    {
        let mut exec = make_executor(&mut rt);
        let define = exec
            .execute(&ToolCall {
                id: "toolu_define".into(),
                name: "define_component".into(),
                input: serde_json::json!({
                    "name": "AuditHintItem",
                    "schema": {
                        "title": "string",
                        "status": "string"
                    }
                }),
            })
            .await;
        assert!(
            !define.is_error,
            "define_component failed: {}",
            define.output
        );
    }

    let mut exec = make_executor(&mut rt);
    let result = exec
        .execute(&ToolCall {
            id: "toolu_bad_spawn".into(),
            name: "spawn_entity".into(),
            input: serde_json::json!({
                "components": [
                    {"name": "AuditHintItem", "value": ""}
                ]
            }),
        })
        .await;

    assert!(result.is_error);
    assert!(
        result
            .output
            .contains("component value must be a JSON object")
    );
    assert!(result.output.contains("Recovery hint for spawn_entity"));
    assert!(
        result
            .output
            .contains("components[*].value must include every required field")
    );
    assert!(
        result
            .output
            .contains("number/integer fields must be unquoted JSON numbers")
    );
}

#[test]
fn test_query_entities_error_includes_dsl_hint() {
    let output = append_recovery_hint(
        "query_entities",
        "Query error: Unknown component: ViewState.panels contains 'mind-view'".into(),
    );

    assert!(output.contains("Recovery hint for query_entities"));
    assert!(output.contains("\"ViewState\""));
    assert!(output.contains("get_world_overview"));
}

#[test]
fn test_rhai_error_includes_guide_hint() {
    let output = append_recovery_hint(
        "run_system",
        "System failed: Function not found: read_file (&str)".into(),
    );

    assert!(output.contains("Recovery hint for Rhai systems"));
    assert!(output.contains("guide_rhai_syntax"));
    assert!(output.contains("#{ComponentName"));
}

#[tokio::test]
async fn test_search_tool_dispatch_rejects_bad_input() {
    // 真实网络请求不在单元测试范围。这里只验证 unified `search` tool 嘅
    // dispatch 路由 + 参数校验(query 必须非空,kind 必须系 web/wiki/code/news)。
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);

    // Empty query
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "search".into(),
            input: serde_json::json!({"query": "", "kind": "web"}),
        })
        .await;
    assert!(r.is_error);
    assert!(r.output.contains("Missing or empty 'query'"));

    // Missing query field
    let r = exec
        .execute(&ToolCall {
            id: "t2".into(),
            name: "search".into(),
            input: serde_json::json!({"kind": "web"}),
        })
        .await;
    assert!(r.is_error);
    assert!(r.output.contains("Missing or empty 'query'"));

    // Missing kind field
    let r = exec
        .execute(&ToolCall {
            id: "t3".into(),
            name: "search".into(),
            input: serde_json::json!({"query": "rust"}),
        })
        .await;
    assert!(r.is_error);
    assert!(r.output.contains("Missing 'kind' field"));
}

#[tokio::test]
async fn test_fetch_url_tool_dispatch_rejects_bad_input() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);

    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "fetch_url".into(),
            input: serde_json::json!({"url": "file:///etc/passwd"}),
        })
        .await;

    assert!(r.is_error);
    assert!(r.output.contains("http:// and https://"));
}
