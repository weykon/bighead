use super::*;
use big_head_world::LensBucket;

#[test]
fn test_lens_tools_are_dynamic_when_world_manifest_attached() {
    let mut rt = AgentRuntime::new_for_test();
    rt.set_world_manifest(temp_manifest("defs"));
    let exec = make_executor(&mut rt);
    let names = exec
        .dynamic_tool_definitions()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert!(names.contains(&"create_lens".into()));
    assert!(names.contains(&"list_lens_inbox".into()));
    assert!(names.contains(&"read_lens".into()));
    assert!(names.contains(&"accept_lens".into()));
    assert!(names.contains(&"reject_lens".into()));
}

#[tokio::test]
async fn test_create_lens_writes_to_outbox_bucket() {
    let mut rt = AgentRuntime::new_for_test();
    let manifest = temp_manifest("create");
    let root = manifest.paths.root_dir.clone();
    let outbox = manifest.paths.outbox_dir.clone();
    rt.set_world_manifest(manifest);

    let mut exec = make_executor(&mut rt);
    let r = exec
        .execute(&ToolCall {
            id: "toolu_create_lens".into(),
            name: "create_lens".into(),
            input: serde_json::json!({
                "to_world": "peer:alice",
                "kind": "request",
                "intent": "review",
                "summary": "Review ambient dwelling notes",
                "refs": [{
                    "world_id": "test:create",
                    "object_id": "topic-1",
                    "object_type": "Topic",
                    "version": "v1"
                }],
                "payload": {"question": "what is missing?"},
                "permissions": {
                    "can_store": true,
                    "can_derive": true,
                    "can_forward": false
                }
            }),
        })
        .await;

    assert!(!r.is_error, "create_lens failed: {}", r.output);
    assert!(r.output.contains("Lens created in outbox"));
    let lenses = LensBucket::new(&outbox).read_pending().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0].to_world.as_deref(), Some("peer:alice"));
    assert_eq!(lenses[0].kind, "request");
    assert_eq!(lenses[0].refs.len(), 1);
    let _ = std::fs::remove_dir_all(root);
}
