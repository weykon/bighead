use super::*;

#[tokio::test]
async fn large_tool_results_share_body_and_history_keeps_reference() {
    let mut rt = AgentRuntime::new_for_test();
    rt.system_registry
        .register(crate::SystemDefinition {
            id: "large-output".into(),
            description: String::new(),
            system_prompt: "".into(),
            kind: crate::SystemKind::RhaiScript {
                source: format!("log(\"{}\");", "x".repeat(20000)),
            },
            tool_names: vec![],
            max_iterations: 1,
            is_tool: false,
        })
        .unwrap();
    let mut exec = make_executor(&mut rt);
    for id in ["first", "second"] {
        let result = exec
            .execute(&ToolCall {
                id: id.into(),
                name: "run_system".into(),
                input: serde_json::json!({"id":"large-output"}),
            })
            .await;
        assert!(!result.is_error, "{}", result.output);
        assert!(
            result.output.starts_with("[Context Artifact]"),
            "{}",
            result.output
        );
    }
    assert!(
        exec.tool_call_log
            .iter()
            .all(|(_, _, output, _)| output.len() < 2000)
    );
    assert!(exec.last_tool_result.as_ref().unwrap().len() > 20000);
    drop(exec);
    let groups: std::collections::BTreeSet<_> = rt
        .world
        .query::<&big_head_shard::ShardData>()
        .iter(&rt.world)
        .map(|s| s.paradigm.clone())
        .collect();
    assert_eq!(groups.len(), 1);
}

#[tokio::test]
async fn broad_entity_read_is_paged_without_creating_another_shard() {
    let mut rt = AgentRuntime::new_for_test();
    let entity = rt
        .world
        .spawn(crate::TickAction {
            enabled: true,
            script: "x".repeat(200000),
            cooldown_secs: 600,
            last_run: 0,
        })
        .id();
    let mut exec = make_executor(&mut rt);
    let result = exec
        .execute(&ToolCall {
            id: "read".into(),
            name: "get_entity".into(),
            input: serde_json::json!({"entity_id":entity.index().index()}),
        })
        .await;
    assert!(!result.is_error);
    let page: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(page["has_more"], true);
    assert!(result.output.len() < 8000);
    drop(exec);
    assert_eq!(
        rt.world
            .query::<&big_head_shard::ShardData>()
            .iter(&rt.world)
            .count(),
        0
    );
}
#[tokio::test]
async fn test_context_projection_can_be_read_as_shard_window() {
    let mut rt = AgentRuntime::new_for_test();
    let content = "0123456789abcdefghijklmnopqrstuvwxyz".repeat(20);
    let paradigm = {
        let mut exec = make_executor(&mut rt);
        let result = exec
            .project_context_content(big_head_ai::ContextProjectionRequest {
                source_kind: "message_text".into(),
                source_label: "message:0:text:0".into(),
                stable_id: "stable-test-id".into(),
                content: content.clone(),
            })
            .expect("projection should write ShardData");
        assert_eq!(result.component, "ShardData");
        assert_eq!(result.total_shards, 1);
        result.paradigm
    };

    let mut exec = make_executor(&mut rt);
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "get_shard_content".into(),
            input: serde_json::json!({
                "paradigm": paradigm,
                "shard_index": 0,
                "offset": 10,
                "limit": 25
            }),
        })
        .await;

    assert!(!r.is_error, "get_shard_content failed: {}", r.output);
    assert!(r.output.contains("ShardData content window"));
    assert!(r.output.contains("offset: 10"));
    assert!(r.output.contains("returned_chars: 25"));
    assert!(r.output.contains("has_more: true"));
    assert!(
        r.output
            .contains("--- content ---\nabcdefghijklmnopqrstuvwxy")
    );
}

#[test]
fn test_context_projection_records_project_context_action() {
    let mut rt = AgentRuntime::new_for_test();
    let tx = crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "test",
        &[],
        None,
        None,
    );
    let result = {
        let mut exec = make_executor(&mut rt);
        exec.tx_id = Some(tx.tx_id.clone());
        exec.current_wave = 5;
        exec.project_context_content(big_head_ai::ContextProjectionRequest {
            source_kind: "message_text".into(),
            source_label: "messages[0].content[0]".into(),
            stable_id: "projection-test".into(),
            content: "large context payload".repeat(100),
        })
        .expect("projection should write ShardData")
    };

    let mut query = rt.world.query::<&crate::ActionRecord>();
    let records = query.iter(&rt.world).collect::<Vec<_>>();
    let record = records
        .iter()
        .find(|record| record.kind == "ProjectContext")
        .expect("projection action should be recorded");
    assert_eq!(record.tx_id, tx.tx_id);
    assert_eq!(record.wave, 5);
    assert_eq!(record.component.as_deref(), Some("ShardData"));
    let after: serde_json::Value =
        serde_json::from_str(record.after_json.as_deref().unwrap()).unwrap();
    assert_eq!(after["paradigm"], result.paradigm);
    assert_eq!(after["component"], "ShardData");
}
