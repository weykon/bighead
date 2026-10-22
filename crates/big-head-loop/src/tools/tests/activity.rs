use super::*;
use serde_json::json;

#[tokio::test]
async fn activity_tools_are_discoverable_and_read_only_peers_cannot_schedule_effects() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    let names = exec.dynamic_tool_definitions();
    for name in [
        "create_activity",
        "update_activity_plan",
        "control_activity",
        "inspect_activity",
    ] {
        assert!(names.iter().any(|d| d.name == name));
        assert!(
            crate::system_def::coordinator_system()
                .tool_names
                .contains(&name.to_string())
        );
    }
    let call = |name: &str, input| ToolCall {
        id: name.into(),
        name: name.into(),
        input,
    };
    let defined = exec
        .execute(&call(
            "define_system",
            json!({"id":"activity_noop","source":"log(\"noop\");"}),
        ))
        .await;
    assert!(!defined.is_error, "{}", defined.output);
    let p = json!({"id":"one","scope":"test","goal":"observe goal",
        "steps":[{"id":"step","system_id":"activity_noop","verify":[{"component":"Business","key_pointer":"/id","key":"one","value_pointer":"/ready","equals":true}]}],
        "acceptance":[{"component":"Business","key_pointer":"/id","key":"one","value_pointer":"/ready","equals":true}]});
    let created = exec.execute(&call("create_activity", p.clone())).await;
    assert!(!created.is_error, "{}", created.output);
    let value: serde_json::Value = serde_json::from_str(&created.output).unwrap();
    assert_eq!(value["activity"]["status"], "draft");
    let run_id = value["activity"]["run_id"].clone();
    let updated = exec
        .execute(&call(
            "update_activity_plan",
            json!({"run_id":run_id,"expected_revision":1,"plan":p.clone()}),
        ))
        .await;
    assert!(!updated.is_error, "{}", updated.output);
    let updated: serde_json::Value = serde_json::from_str(&updated.output).unwrap();
    assert_eq!(
        updated["activity"]["plan_history"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let revision = updated["activity"]["revision"].clone();
    for (name, input) in [
        (
            "define_component",
            json!({"name":"ActivityRun","schema":{}}),
        ),
        (
            "spawn_entity",
            json!({"components":[{"name":"ActivityRun","value":{}}]}),
        ),
    ] {
        assert!(exec.execute(&call(name, input)).await.is_error);
    }
    let allowed = crate::a2a_origin::read_only_tools();
    exec.allowed_tools = Some(&allowed);
    for name in ["inspect_activity", "update_activity_plan"] {
        let detail = exec
            .execute(&call(
                "inspect_ecs_capabilities",
                json!({"view":"operations","name":name}),
            ))
            .await;
        assert!(!detail.is_error, "{}", detail.output);
        let value: serde_json::Value = serde_json::from_str(&detail.output).unwrap();
        assert_eq!(
            value["entries"][0]["available_in_current_context"],
            name == "inspect_activity"
        );
        assert!(value["entries"][0]["input_schema"].is_object());
    }
    assert!(
        exec.execute(&call(
            "update_activity_plan",
            json!({"run_id":run_id,"expected_revision":revision,"plan":p.clone()})
        ))
        .await
        .is_error
    );
    assert!(exec.execute(&call("create_activity", p)).await.is_error);
    assert!(
        exec.execute(&call(
            "control_activity",
            json!({"run_id":run_id,"expected_revision":1,"action":"start"})
        ))
        .await
        .is_error
    );
    let inspected = exec
        .execute(&call("inspect_activity", json!({"run_id":run_id})))
        .await;
    assert!(!inspected.is_error, "{}", inspected.output);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&inspected.output).unwrap()["status"],
        "draft"
    );
}
