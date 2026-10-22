use super::*;

#[test]
fn test_render_view_error_includes_protocol_hint() {
    let output = append_recovery_hint(
        "render_view",
        "render_view: deserialize failed: missing field `patch`".into(),
    );

    assert!(output.contains("Recovery hint for render_view"));
    assert!(output.contains("\"op\":\"update\""));
    assert!(output.contains("\"patch\":{\"content\""));
    assert!(output.contains("guide_render_view"));
}

#[tokio::test]
async fn test_render_view_tool_without_channel() {
    // No view channel attached → error message tells user to start server.
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "render_view".into(),
            input: serde_json::json!({
                "op": "spawn",
                "id": "p1",
                "kind": "card",
                "title": "Test",
                "position": [0.0, 0.0, 0.0],
                "content": {"kind": "card", "label": "x", "value": 1}
            }),
        })
        .await;
    assert!(r.is_error);
    assert!(r.output.contains("View server not running"));
}

#[tokio::test]
async fn test_render_view_tool_forwards_to_channel() {
    use big_head_view::ViewChannel;

    let mut rt = AgentRuntime::new_for_test();
    let (channel, sink) = ViewChannel::new();
    rt.view_handler = Some(big_head_view::ViewToolHandler::new(Some(channel), None));
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
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "render_view".into(),
            input: serde_json::json!({
                "op": "spawn",
                "id": "hello-1",
                "surface": "html2d",
                "kind": "card",
                "title": "Hello",
                "position": [1.0, 2.0, 3.0],
                "content": {"kind": "card", "label": "Count", "value": 42}
            }),
        })
        .await;
    assert!(
        !r.is_error,
        "render_view should succeed when channel attached: {}",
        r.output
    );

    // Drain sink — should have the spawn command
    let cmd = sink.recv_command().await.expect("got command");
    match cmd {
        big_head_view::ViewCommand::Spawn {
            id,
            surface,
            kind,
            position,
            ..
        } => {
            assert_eq!(id, "hello-1");
            assert_eq!(surface, big_head_view::ViewSurface::Html2D);
            assert_eq!(kind, big_head_view::PanelKind::Card);
            assert_eq!(position, [1.0, 2.0, 3.0]);
        }
        other => panic!("expected spawn, got {:?}", other),
    }

    drop(exec);
    let mut query = rt.world.query::<&crate::ActionRecord>();
    let records = query.iter(&rt.world).collect::<Vec<_>>();
    let record = records
        .iter()
        .find(|record| record.tool_call_id == "t1" && record.kind == "RenderView")
        .expect("render_view action should be recorded");
    assert_eq!(record.component.as_deref(), Some("ViewState"));
    assert!(record.entity.is_some());
    let after: serde_json::Value =
        serde_json::from_str(record.after_json.as_deref().unwrap()).unwrap();
    assert!(after["panels"]["hello-1"].is_object());
    assert_eq!(after["panels"]["hello-1"]["surface"], "html2d");
}

#[tokio::test]
async fn test_render_mind_view_materializes_publishes_and_persists() {
    let mut rt = AgentRuntime::new_for_test();
    let (channel, sink) = big_head_view::ViewChannel::new();
    rt.view_handler = Some(big_head_view::ViewToolHandler::new(Some(channel), None));

    let mut exec = make_executor(&mut rt);
    assert!(
        exec.dynamic_tool_definitions()
            .iter()
            .any(|definition| definition.name == "render_mind_view")
    );
    let result = exec
        .execute(&ToolCall {
            id: "mind-1".into(),
            name: "render_mind_view".into(),
            input: serde_json::json!({
                "title": "Live world",
                "markup": "<canvas id=\"world\"></canvas>"
            }),
        })
        .await;
    assert!(!result.is_error, "mind render failed: {}", result.output);
    assert!(result.output.contains("panel=mind-view"));

    let command = sink.recv_command().await.expect("mind view command");
    match command {
        big_head_view::ViewCommand::Spawn {
            id,
            surface,
            kind,
            title,
            content,
            ..
        } => {
            assert_eq!(id, "mind-view");
            assert_eq!(surface, big_head_view::ViewSurface::Html2D);
            assert_eq!(kind, big_head_view::PanelKind::Html);
            assert_eq!(title, "Live world");
            let big_head_view::PanelContent::Html { markup } = content else {
                panic!("expected HTML mind content");
            };
            assert!(markup.contains("data-big-head-mind-runtime=\"raw\""));
        }
        other => panic!("expected mind view spawn, got {other:?}"),
    }

    drop(exec);
    let view_state_id = rt
        .component_registry
        .dynamic_store()
        .get_id("ViewState")
        .expect("ViewState registered");
    let view_entity = rt
        .world
        .iter_entities()
        .find(|entity| entity.contains_id(view_state_id))
        .map(|entity| entity.id())
        .expect("ViewState entity");
    let view_state = big_head_ecs::dynamic_component::read(&rt.world, view_entity, view_state_id)
        .expect("ViewState value");
    assert_eq!(view_state["panels"]["mind-view"]["title"], "Live world");
    assert_eq!(view_state["panels"]["mind-view"]["surface"], "html2d");
}

#[tokio::test]
async fn test_inspect_component_view_materializes_and_persists_stable_panel() {
    let mut rt = AgentRuntime::new_for_test();
    rt.world.spawn(crate::TickAction {
        enabled: true,
        script: "// weather_query_tickaction\nlet now = timestamp();".into(),
        last_run: 0,
        cooldown_secs: 21_600,
    });
    let (channel, sink) = big_head_view::ViewChannel::new();
    rt.view_handler = Some(big_head_view::ViewToolHandler::new(Some(channel), None));

    let mut exec = make_executor(&mut rt);
    assert!(
        exec.dynamic_tool_definitions()
            .iter()
            .any(|definition| definition.name == "inspect_component_view")
    );
    let result = exec
        .execute(&ToolCall {
            id: "inspect-1".into(),
            name: "inspect_component_view".into(),
            input: serde_json::json!({
                "component": "TickAction",
                "limit": 10
            }),
        })
        .await;
    assert!(!result.is_error, "inspector failed: {}", result.output);
    assert!(result.output.contains("1 of 1 entities"));

    let command = sink.recv_command().await.expect("inspector command");
    match command {
        big_head_view::ViewCommand::Spawn {
            id,
            surface,
            kind,
            content,
            ..
        } => {
            assert_eq!(id, "inspector-tickactions");
            assert_eq!(surface, big_head_view::ViewSurface::Html2D);
            assert_eq!(kind, big_head_view::PanelKind::Inspector);
            match content {
                big_head_view::PanelContent::Inspector {
                    component, entries, ..
                } => {
                    assert_eq!(component, "TickAction");
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].label, "weather_query_tickaction");
                }
                other => panic!("expected inspector content, got {other:?}"),
            }
        }
        other => panic!("expected inspector spawn, got {other:?}"),
    }

    drop(exec);
    let view_state_id = rt
        .component_registry
        .dynamic_store()
        .get_id("ViewState")
        .expect("ViewState registered");
    let view_entity = rt
        .world
        .iter_entities()
        .find(|entity| entity.contains_id(view_state_id))
        .map(|entity| entity.id())
        .expect("ViewState entity");
    let view_state = big_head_ecs::dynamic_component::read(&rt.world, view_entity, view_state_id)
        .expect("ViewState value");
    assert!(view_state["panels"]["inspector-tickactions"].is_object());
    assert!(view_state["panels"].get("").is_none());
    assert_eq!(
        view_state["panels"]["inspector-tickactions"]["surface"],
        "html2d"
    );
}

#[tokio::test]
async fn test_render_view_intent_persists_materialized_panel_metadata() {
    let mut rt = AgentRuntime::new_for_test();
    let (channel, _sink) = big_head_view::ViewChannel::new();
    rt.view_handler = Some(big_head_view::ViewToolHandler::new(Some(channel), None));

    let mut exec = make_executor(&mut rt);
    let result = exec
        .execute(&ToolCall {
            id: "intent-1".into(),
            name: "render_view".into(),
            input: serde_json::json!({
                "op": "spawn",
                "intent": "inspect_sparks",
                "content": {
                    "kind": "inspector",
                    "component": "Spark",
                    "entries": []
                }
            }),
        })
        .await;
    assert!(!result.is_error, "render_view failed: {}", result.output);

    drop(exec);
    let view_state_id = rt
        .component_registry
        .dynamic_store()
        .get_id("ViewState")
        .expect("ViewState registered");
    let view_entity = rt
        .world
        .iter_entities()
        .find(|entity| entity.contains_id(view_state_id))
        .map(|entity| entity.id())
        .expect("ViewState entity");
    let view_state = big_head_ecs::dynamic_component::read(&rt.world, view_entity, view_state_id)
        .expect("ViewState value");
    let panel = &view_state["panels"]["inspector-sparks"];
    assert_eq!(panel["surface"], "html2d");
    assert_eq!(panel["kind"], "inspector");
    assert_eq!(panel["title"], "Spark Queue Inspector");
}

#[tokio::test]
async fn test_render_view_tool_rejects_bad_input() {
    let mut rt = AgentRuntime::new_for_test();
    let (channel, _sink) = big_head_view::ViewChannel::new();
    rt.view_handler = Some(big_head_view::ViewToolHandler::new(Some(channel), None));

    let mut exec = make_executor(&mut rt);
    // Missing required 'id' field
    let r = exec
        .execute(&ToolCall {
            id: "t1".into(),
            name: "render_view".into(),
            input: serde_json::json!({"op": "spawn", "kind": "card"}),
        })
        .await;
    assert!(r.is_error);
    assert!(r.output.contains("render_view: deserialize failed"));
}
