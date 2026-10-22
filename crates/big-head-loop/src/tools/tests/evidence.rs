use super::*;
use crate::audit;

#[tokio::test]
async fn experience_evidence_cannot_be_changed_by_generic_tools() {
    let mut rt = AgentRuntime::new_for_test();
    let entity = rt
        .world
        .spawn(crate::experience::tests::sample(
            "protected",
            "experience:local-user",
            "Maybe",
        ))
        .id();
    let mut exec = make_executor(&mut rt);
    for (name, input) in [
        (
            "modify_component",
            serde_json::json!({"entity_id":entity.index().index(),"component":"InteractionExperience","value":{}}),
        ),
        (
            "despawn_entity",
            serde_json::json!({"entity_id":entity.index().index()}),
        ),
        (
            "define_component",
            serde_json::json!({"name":"ExperienceReading","schema":{}}),
        ),
    ] {
        let result = exec
            .execute(&ToolCall {
                id: name.into(),
                name: name.into(),
                input,
            })
            .await;
        assert!(result.is_error, "{name}");
    }
    assert_eq!(
        exec.world
            .get::<crate::experience::InteractionExperience>(entity)
            .unwrap()
            .request_excerpt,
        "Maybe"
    );
}

#[tokio::test]
async fn a2a_read_only_capability_is_enforced_before_dispatch() {
    let mut rt = AgentRuntime::new_for_test();
    let allowed = crate::a2a_origin::read_only_tools();
    let mut exec = make_executor(&mut rt);
    exec.allowed_tools = Some(&allowed);
    exec.tx_id = Some("a2a-denied-tools".into());
    for name in ["execute_cli", "spawn_entity", "record_fact", "push_spark"] {
        let denied = exec
            .execute(&ToolCall {
                id: format!("deny-{name}"),
                name: name.into(),
                input: serde_json::json!({}),
            })
            .await;
        assert!(denied.is_error, "{name} was allowed");
        assert!(
            denied.output.contains("capability set"),
            "{}",
            denied.output
        );
    }
    let observed = exec
        .execute(&ToolCall {
            id: "inspect".into(),
            name: "inspect_runtime".into(),
            input: serde_json::json!({"limit":1}),
        })
        .await;
    assert!(!observed.is_error, "{}", observed.output);
    let denied_actions: Vec<_> = exec
        .world
        .query::<&crate::ActionRecord>()
        .iter(exec.world)
        .filter(|action| action.tx_id == "a2a-denied-tools" && action.is_error)
        .collect();
    assert_eq!(denied_actions.len(), 4);
}

#[tokio::test]
async fn dream_tools_are_discoverable_executable_audited_and_capability_scoped() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    exec.tx_id = Some("dream-transaction".into());
    let definitions = exec.dynamic_tool_definitions();
    for name in ["dream_task", "dream_policy"] {
        assert!(definitions.iter().any(|d| d.name == name));
        assert!(
            crate::system_def::coordinator_system()
                .tool_names
                .iter()
                .any(|n| n == name)
        );
    }
    let created = exec.execute(&ToolCall {id:"dream-create".into(),name:"dream_task".into(),input:serde_json::json!({"action":"create","id":"one","task":{"description":"Identity","cases_json":"[{\"input\":1,\"expected\":1}]"}})}).await;
    assert!(!created.is_error, "{}", created.output);
    let actions: Vec<_> = exec
        .world
        .query::<&crate::ActionRecord>()
        .iter(exec.world)
        .collect();
    assert!(
        actions
            .iter()
            .any(|a| a.tool_name == "dream_task" && !a.is_error)
    );
    let blocked = exec
        .execute(&ToolCall {
            id: "rewrite".into(),
            name: "modify_component".into(),
            input: serde_json::json!({"component":"DreamExperiment","entity_id":0,"data":{}}),
        })
        .await;
    assert!(blocked.is_error);
    assert!(blocked.output.contains("runtime-owned"));
    let allowed = vec!["inspect_runtime".into()];
    exec.allowed_tools = Some(&allowed);
    assert!(
        !exec
            .dynamic_tool_definitions()
            .iter()
            .any(|d| d.name == "dream_task")
    );
    let blocked = exec
        .execute(&ToolCall {
            id: "restricted".into(),
            name: "dream_task".into(),
            input: serde_json::json!({"action":"inspect"}),
        })
        .await;
    assert!(blocked.is_error);
    assert!(blocked.output.contains("capability set"));
}

#[tokio::test]
async fn interpretation_records_cannot_be_forged_through_generic_ecs_writes() {
    let mut rt = AgentRuntime::new_for_test();
    let fact = rt
        .world
        .spawn(crate::interpretation::FactRecord {
            fact_id: "protected".into(),
            statement: "observed".into(),
            payload: serde_json::json!({}),
            source: "test".into(),
            observed_at: 1,
            epistemic_status: "observed".into(),
            version: "v1".into(),
        })
        .id();
    let mut exec = make_executor(&mut rt);
    for call in [
        ToolCall {
            id: "forge".into(),
            name: "spawn_entity".into(),
            input: serde_json::json!({"components":[{"name":"FactRecord","value":{}}]}),
        },
        ToolCall {
            id: "rewrite".into(),
            name: "modify_component".into(),
            input: serde_json::json!({
                "entity_id":fact.index().index(),
                "component":"FactRecord",
                "value":{}
            }),
        },
        ToolCall {
            id: "erase".into(),
            name: "despawn_entity".into(),
            input: serde_json::json!({"entity_id":fact.index().index()}),
        },
    ] {
        let result = exec.execute(&call).await;
        assert!(result.is_error);
        assert!(result.output.contains("runtime-owned"));
    }
    assert!(
        exec.world
            .get::<crate::interpretation::FactRecord>(fact)
            .is_some()
    );
}

#[tokio::test]
async fn steward_requires_real_matching_write_receipts() {
    use crate::steward::{ObserverContract, validate};
    let mut rt = AgentRuntime::new_for_test();
    let tick = rt
        .world
        .spawn(crate::TickAction {
            enabled: true,
            script: "missing_helper();".into(),
            ..Default::default()
        })
        .id();
    let reports = rt.evaluate_tick_actions();
    let mut pipeline = crate::observation::TickObservationPipeline::<16>::default();
    pipeline.ingest(&reports);
    rt.world
        .insert_resource(ObserverContract(pipeline.lease(u64::MAX, 8).unwrap().items));
    let tx = audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "test-observer",
        &[],
        None,
        None,
    );
    let allowed = crate::system_def::tick_steward_system().tool_names;
    let mut executor = make_executor(&mut rt);
    executor.allowed_tools = Some(&allowed);
    executor.tx_id = Some(tx.tx_id.clone());
    let response = executor
        .execute(&ToolCall {
            id: "repair".into(),
            name: "update_tick_action".into(),
            input: serde_json::json!({
                "entity_id":tick.index().index(), "expected_script_hash":reports[0].script_hash,
                "script":"log(\"fixed\");", "reason":"Remove unavailable helper"
            }),
        })
        .await;
    assert!(!response.is_error, "{}", response.output);
    assert!(response.output.contains("Evidence action:"));
    let action_id = response
        .output
        .split("Evidence action: ")
        .last()
        .unwrap()
        .trim()
        .to_owned();
    let disposition = serde_json::json!({"dispositions":[{"event":1,"disposition":"repair","reason":"Removed unavailable helper","evidence_action_id":action_id}]}).to_string();
    assert!(executor.validate_final_response(&disposition).is_ok());
    let forged = executor
        .execute(&ToolCall {
            id: "forged".into(),
            name: "modify_component".into(),
            input: serde_json::json!({
                "entity_id":tick.index().index(), "component":"ActionRecord", "value":{}
            }),
        })
        .await;
    assert!(forged.is_error);
    drop(executor);
    assert!(
        validate(
            &mut rt.world,
            &rt.component_registry,
            "unrelated-tx",
            &disposition
        )
        .is_err()
    );
    rt.world.get_mut::<crate::TickAction>(tick).unwrap().script = "changed_again();".into();
    assert!(
        validate(
            &mut rt.world,
            &rt.component_registry,
            &tx.tx_id,
            &disposition
        )
        .is_err()
    );
}

#[tokio::test]
async fn runtime_inspection_reports_orphans_and_stable_correlations() {
    let mut rt = AgentRuntime::new_for_test();
    rt.push_spark("queued".into());
    rt.world.spawn(Spark {
        text: "orphan".into(),
        ..Default::default()
    });
    let mut executor = make_executor(&mut rt);
    let response = executor
        .execute(&ToolCall {
            id: "inspect".into(),
            name: "inspect_runtime".into(),
            input: serde_json::json!({}),
        })
        .await;
    assert!(!response.is_error);
    let value: serde_json::Value = serde_json::from_str(&response.output).unwrap();
    assert_eq!(value["queued_count"], 1);
    assert!(
        value["sparks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["orphan_pending"] == true)
    );
    assert_eq!(value["events"][0]["kind"], "SparkQueued");
}

#[tokio::test]
async fn pipeline_tool_allowlist_is_enforced_by_the_executor() {
    let mut rt = AgentRuntime::new_for_test();
    let tick = rt
        .world
        .spawn(big_head_ecs::TickAction {
            enabled: true,
            script: "log(\"stable\");".into(),
            ..Default::default()
        })
        .id();
    let allowed = vec![
        "get_entity".to_string(),
        "modify_component".to_string(),
        "update_tick_action".to_string(),
    ];
    let mut exec = make_executor(&mut rt);
    exec.allowed_tools = Some(&allowed);

    let result = exec
        .execute(&ToolCall {
            id: "blocked-push".into(),
            name: "push_spark".into(),
            input: serde_json::json!({"description": "must not queue"}),
        })
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("outside this pipeline"));

    let bypass = exec
        .execute(&ToolCall {
            id: "blocked-script-bypass".into(),
            name: "modify_component".into(),
            input: serde_json::json!({
                "entity_id": tick.index().index(),
                "component": "TickAction",
                "value": {"script": "let = ;"}
            }),
        })
        .await;
    assert!(bypass.is_error);
    assert!(bypass.output.contains("must use update_tick_action"));
    drop(exec);
    assert_eq!(rt.pending_spark_count(), 0);
    assert_eq!(
        rt.world
            .get::<big_head_ecs::TickAction>(tick)
            .unwrap()
            .script,
        "log(\"stable\");"
    );
}

#[tokio::test]
async fn test_executor_records_modify_action_before_after() {
    let mut rt = AgentRuntime::new_for_test();
    rt.register_component_action::<TestHealth>("TestHealth");
    let entity = rt.world.spawn(TestHealth(1.0)).id();
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
        exec.current_wave = 3;
        let r = exec
            .execute(&ToolCall {
                id: "toolu_modify".into(),
                name: "modify_component".into(),
                input: serde_json::json!({
                    "entity_id": entity.index().index(),
                    "component": "TestHealth",
                    "value": 2.0
                }),
            })
            .await;
        assert!(!r.is_error, "modify_component failed: {}", r.output);
    }

    let mut query = rt.world.query::<&crate::ActionRecord>();
    let records = query.iter(&rt.world).collect::<Vec<_>>();
    let record = records
        .iter()
        .find(|record| record.tool_call_id == "toolu_modify")
        .expect("modify action should be recorded");
    assert_eq!(record.tx_id, tx.tx_id);
    assert_eq!(record.wave, 3);
    assert_eq!(record.kind, "ModifyComponent");
    assert_eq!(record.component.as_deref(), Some("TestHealth"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(record.before_json.as_deref().unwrap()).unwrap(),
        serde_json::json!(1.0)
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(record.after_json.as_deref().unwrap()).unwrap(),
        serde_json::json!(2.0)
    );
}

#[tokio::test]
async fn test_executor_records_failed_tool_action() {
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
        let r = exec
            .execute(&ToolCall {
                id: "toolu_fail".into(),
                name: "nope".into(),
                input: serde_json::json!({}),
            })
            .await;
        assert!(r.is_error);
    }

    let mut query = rt.world.query::<&crate::ActionRecord>();
    let records = query.iter(&rt.world).collect::<Vec<_>>();
    let record = records
        .iter()
        .find(|record| record.tool_call_id == "toolu_fail")
        .expect("failed tool action should be recorded");
    assert_eq!(record.tx_id, tx.tx_id);
    assert_eq!(record.kind, "ToolCall");
    assert!(record.is_error);
    assert!(record.output_summary.contains("Unknown tool"));
}

#[tokio::test]
async fn test_executor_records_push_spark_action_target() {
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
        let r = exec
            .execute(&ToolCall {
                id: "toolu_push_spark".into(),
                name: "push_spark".into(),
                input: serde_json::json!({"description": "follow-up task"}),
            })
            .await;
        assert!(!r.is_error, "push_spark failed: {}", r.output);
    }

    let mut query = rt.world.query::<&crate::ActionRecord>();
    let records = query.iter(&rt.world).collect::<Vec<_>>();
    let record = records
        .iter()
        .find(|record| record.tool_call_id == "toolu_push_spark")
        .expect("push_spark action should be recorded");
    assert_eq!(record.kind, "PushSpark");
    assert_eq!(record.component.as_deref(), Some("Spark"));
    assert!(record.entity.is_some());
    let after: serde_json::Value =
        serde_json::from_str(record.after_json.as_deref().unwrap()).unwrap();
    assert_eq!(after["entity"], record.entity.as_ref().unwrap().as_str());
    assert!(
        after["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["name"].as_str().unwrap_or("").ends_with("Spark"))
    );
}
