use super::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn message(text: &str) -> Value {
    json!({"content":[{"type":"text","text":text}],"stop_reason":"end_turn"})
}

fn tool(id: &str, name: &str, input: Value) -> Value {
    json!({"content":[{"type":"tool_use","id":id,"name":name,"input":input}],"stop_reason":"tool_use"})
}

// Exercise the actual provider adapter and agent loop with deterministic responses.
async fn scripted_agent(
    responses: Vec<Value>,
) -> (big_head_ai::Agent, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(20), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut data = Vec::new();
            let body_start;
            loop {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0 && data.len() < 4_000_000);
                data.extend_from_slice(&chunk[..n]);
                if let Some(index) = data.windows(4).position(|b| b == b"\r\n\r\n") {
                    body_start = index + 4;
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&data[..body_start]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while data.len() < body_start + length {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&chunk[..n]);
            }
            requests.push(serde_json::from_slice(&data[body_start..body_start + length]).unwrap());
            let body = response.to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let mut config =
        big_head_ai::ApiConfig::new(big_head_ai::ApiProvider::MiniMax, "local-fixture".into());
    config.base_url = Some(format!("http://{address}"));
    (big_head_ai::Agent::from_api_config(config), server)
}

#[tokio::test]
async fn review_returns_omitted_work_to_real_tools_and_keeps_failure_receipts() {
    let mut rt = AgentRuntime::new_for_test();
    rt.register_component_action::<TestHealth>("TestHealth");
    let entity = rt.world.spawn(TestHealth(0.0)).id();
    let eid = entity.index().index();
    let tx = crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "test:replay",
        &[],
        None,
        None,
    );
    let (mut agent, server) = scripted_agent(vec![
        message("Task remains incomplete."),
        message(r#"{"reply":"正在補齊更新。","continue_with":"Set TestHealth to 7, then independently read it back."}"#),
        tool("bad", "modify_component", json!({"entity_id":eid,"component":"TestHealth","value":"invalid"})),
        tool("fixed", "modify_component", json!({"entity_id":eid,"component":"TestHealth","value":7.0})),
        tool("readback", "get_component", json!({"entity_id":eid,"component":"TestHealth"})),
        message("TestHealth was updated and independently read back as 7."),
        message(r#"{"reply":"已更新為 7，亦已讀返確認。","continue_with":null}"#),
    ]).await;
    let mut exec = make_executor(&mut rt);
    exec.tx_id = Some(tx.tx_id.clone());
    let mut operation = agent
        .reason_with_tools(
            "Please update the probe to 7",
            &mut exec,
            12,
            "Execute authorized work.",
        )
        .await
        .unwrap();
    assert_eq!(operation.total_actions, 0);
    let reply = crate::completion::review_and_continue(
        &mut agent,
        &mut exec,
        &mut operation,
        "Please update the probe to 7",
        "Execute authorized work.",
        12,
    )
    .await
    .unwrap();
    assert!(reply.contains('7'));
    assert_eq!(operation.total_actions, 3);
    assert_eq!(
        exec.tool_call_log.iter().map(|c| c.3).collect::<Vec<_>>(),
        [true, false, false]
    );
    assert_eq!(exec.world.get::<TestHealth>(entity).unwrap().0, 7.0);
    let audit =
        super::super::inspect_transaction::inspect(exec.world, &json!({"tx_id":tx.tx_id})).unwrap();
    assert_eq!(audit["tool_execution_count"], 3);
    assert!(
        audit["tool_executions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["finished_at"].is_number() && t["tool_call_id"].is_string())
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 7);
    assert!(
        requests[3]["messages"]
            .to_string()
            .contains("is_error\":true")
    );
    assert!(
        requests[6]["messages"]
            .to_string()
            .contains("get_component")
    );
}

fn local_transaction(rt: &mut AgentRuntime) -> String {
    rt.enable_user_questions();
    let spark = rt
        .push_spark_front("Interaction Context:\nsource=tui\nUser Spark:\n幫我準備一個項目".into());
    // The fixture declares this input interactive, just as the frontend admission path does.
    rt.world
        .entity_mut(spark)
        .insert(big_head_ecs::execution::SparkDispatch {
            sequence: 1,
            interactive: true,
            source_id: "local-test".into(),
            correlation_id: "question-test".into(),
            attempts: 0,
        });
    crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "test:question",
        &[spark],
        None,
        None,
    )
    .tx_id
}

#[tokio::test]
async fn question_stops_the_current_batch_and_survives_checkpoint_with_single_answer() {
    let mut rt = AgentRuntime::new_for_test();
    let tx_id = local_transaction(&mut rt);
    let mut response = tool(
        "ask",
        "request_user_input",
        json!({"question":"項目名稱？","options":["garden","library"]}),
    );
    response["content"].as_array_mut().unwrap().push(json!({"type":"tool_use","id":"must-not-run","name":"spawn_entity","input":{"components":[]}}));
    let (mut agent, server) = scripted_agent(vec![response]).await;
    {
        let mut exec = make_executor(&mut rt);
        exec.tx_id = Some(tx_id);
        let result = agent
            .reason_with_tools(
                "幫我建立項目",
                &mut exec,
                8,
                "Ask for genuinely missing input.",
            )
            .await
            .unwrap();
        assert_eq!(
            result.termination,
            big_head_ai::ReasoningTermination::AwaitingInput
        );
        assert_eq!(result.total_actions, 1);
        assert_eq!(exec.tool_call_log.len(), 1);
    }
    assert_eq!(server.await.unwrap().len(), 1);
    let path = std::env::temp_dir().join(format!("questions-{}.json", uuid::Uuid::new_v4()));
    rt.checkpoint(&path).unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    restored.enable_user_questions();
    let q = restored.pending_user_questions().pop().unwrap();
    let resumed = restored
        .answer_user_question(&q.id, Some("a custom project"))
        .unwrap()
        .unwrap();
    assert!(resumed.contains("a custom project"));
    assert!(resumed.contains("原本"));
    assert!(
        restored
            .answer_user_question(&q.id, Some("garden"))
            .is_err()
    );
    assert!(restored.pending_user_questions().is_empty());
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn denied_tools_are_traced_and_incomplete_attempts_recover_as_unknown() {
    let mut rt = AgentRuntime::new_for_test();
    {
        let mut exec = make_executor(&mut rt);
        exec.allowed_tools = Some(&[]);
        assert!(
            exec.execute(&ToolCall {
                id: "denied".into(),
                name: "spawn_entity".into(),
                input: json!({"components":[]})
            })
            .await
            .is_error
        );
    }
    let trace = rt
        .world
        .query::<&crate::ToolExecution>()
        .single(&rt.world)
        .unwrap();
    assert_eq!(trace.status, "failed");
    assert!(trace.output_excerpt.contains("capability"));
    crate::tool_tracking::start(
        &mut rt.world,
        "tx-interrupted",
        &ToolCall {
            id: "unfinished".into(),
            name: "execute_cli".into(),
            input: json!({}),
        },
        1,
    );
    crate::tool_tracking::recover(&mut rt.world);
    assert!(
        rt.world
            .query::<&crate::ToolExecution>()
            .iter(&rt.world)
            .any(|t| t.status == "interrupted_unknown" && t.finished_at.is_none())
    );
}

#[tokio::test]
async fn documents_publish_a_persistent_readable_view_without_claiming_user_read_it() {
    let mut rt = AgentRuntime::new_for_test();
    let (channel, sink) = big_head_view::ViewChannel::new();
    rt.view_handler = Some(big_head_view::ViewToolHandler::new(Some(channel), None));
    let mut exec = make_executor(&mut rt);
    let result = exec.execute(&ToolCall { id:"doc".into(), name:"render_document".into(), input:json!({"id":"learning","title":"一次清楚的解釋","summary":"先理解，再落地。","sections":[{"heading":"第一步","body":"從實際例子開始。"}]}) }).await;
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.contains("/#panel=document-learning"));
    assert!(result.output.contains("\"user_read_verified\":false"));
    assert!(matches!(
        sink.recv_command().await.unwrap(),
        big_head_view::ViewCommand::Spawn { .. }
    ));
    assert!(exec.registry.dynamic_store().get_id("ViewState").is_some());
}

#[tokio::test]
async fn question_requires_local_interactive_origin_and_cannot_be_forged_with_write_tool() {
    let mut rt = AgentRuntime::new_for_test();
    let tx = crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "background",
        &[],
        None,
        None,
    );
    rt.enable_user_questions();
    let mut exec = make_executor(&mut rt);
    exec.tx_id = Some(tx.tx_id);
    let result = exec
        .execute(&ToolCall {
            id: "background-ask".into(),
            name: "request_user_input".into(),
            input: json!({"question":"Proceed?","options":["Yes","No"]}),
        })
        .await;
    assert!(result.is_error);
    assert!(!exec.waiting_for_user());
    let forged = exec
        .execute(&ToolCall {
            id: "forged".into(),
            name: "spawn_entity".into(),
            input: json!({"components":[{"name":"UserQuestion","value":{}}]}),
        })
        .await;
    assert!(forged.is_error);
    assert!(forged.output.contains("runtime-owned"));
}

#[tokio::test]
async fn correction_preserves_capability_boundary_and_truncated_calls_do_not_execute() {
    let mut rt = AgentRuntime::new_for_test();
    let allowed = vec!["get_component".to_string()];
    let (mut agent, server) = scripted_agent(vec![
        message(r#"{"reply":"Checking the outstanding work.","continue_with":"Attempt the requested update."}"#),
        tool("denied-write", "spawn_entity", json!({"components":[]})),
        message("The update was denied by the capability boundary."),
        message(r#"{"reply":"目前通道沒有寫入權限，未執行更新。","continue_with":null}"#),
    ]).await;
    let mut exec = make_executor(&mut rt);
    exec.allowed_tools = Some(&allowed);
    let mut operation = big_head_ai::ReasoningResult {
        final_text: Some("Not yet completed".into()),
        iterations: 1,
        total_actions: 0,
        termination: big_head_ai::ReasoningTermination::Completed,
    };
    crate::completion::review_and_continue(
        &mut agent,
        &mut exec,
        &mut operation,
        "Check status",
        "Respect capabilities",
        8,
    )
    .await
    .unwrap();
    assert_eq!(exec.allowed_tools, Some(allowed.as_slice()));
    assert_eq!(exec.tool_call_log.len(), 1);
    assert!(exec.tool_call_log[0].3);
    assert_eq!(server.await.unwrap().len(), 4);
    let mut truncated = tool("truncated", "spawn_entity", json!({"components":[]}));
    truncated["stop_reason"] = json!("max_tokens");
    let (mut agent, server) = scripted_agent(vec![truncated]).await;
    let before = exec.tool_call_log.len();
    let result = agent
        .reason_with_tools("Create a probe", &mut exec, 8, "test")
        .await
        .unwrap();
    assert_eq!(
        result.termination,
        big_head_ai::ReasoningTermination::OutputLimit
    );
    assert_eq!(exec.tool_call_log.len(), before);
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn reviewer_corrections_stop_after_two_returns_without_recursive_sparks() {
    let mut rt = AgentRuntime::new_for_test();
    let review = message(
        r#"{"reply":"Work remains incomplete.","continue_with":"Read the missing evidence."}"#,
    );
    let (mut agent, server) = scripted_agent(vec![
        review.clone(),
        message("Missing evidence."),
        review.clone(),
        message("Missing evidence."),
        review,
    ])
    .await;
    let mut exec = make_executor(&mut rt);
    let mut operation = big_head_ai::ReasoningResult {
        final_text: Some("Missing evidence.".into()),
        iterations: 1,
        total_actions: 0,
        termination: big_head_ai::ReasoningTermination::Completed,
    };
    let reply = crate::completion::review_and_continue(
        &mut agent,
        &mut exec,
        &mut operation,
        "Read evidence",
        "test",
        12,
    )
    .await
    .unwrap();
    assert!(reply.contains("預算已用完"));
    assert_eq!(
        operation.termination,
        big_head_ai::ReasoningTermination::Incomplete
    );
    assert!(exec.spark_queue.is_empty());
    assert_eq!(server.await.unwrap().len(), 5);
}
