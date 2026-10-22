use super::*;

fn setup() -> AgentRuntime {
    let mut rt = AgentRuntime::new_for_test();
    rt.initialize_work_district();
    rt
}
#[test]
fn layout_is_independent_atomic_and_revision_guarded() {
    let mut rt = setup();
    let before = projection(&mut rt.world);
    let p = json!({"placements":[{"id":"scene-runtime","expected_revision":1,"x":-3.5,"z":-1.6,"height":1.5,"style":"tower","color":"#ee9988"}]});
    dispatch(&mut rt.world, "arrange_work", &p).unwrap();
    let after = projection(&mut rt.world);
    for record in before["records"].as_array().unwrap() {
        let current = after["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["record"]["id"] == record["record"]["id"])
            .unwrap();
        assert_eq!(record["record"], current["record"]);
    }
    assert!(dispatch(&mut rt.world, "arrange_work", &p).is_err());
    let invalid = json!({"placements":[{"id":"scene-runtime","expected_revision":2,"x":0.0,"z":-1.6,"height":1.5,"style":"tower","color":"#ee9988"}]});
    assert!(dispatch(&mut rt.world, "arrange_work", &invalid).is_err());
    assert_eq!(projection(&mut rt.world), after);
    assert!(
        dispatch(
            &mut rt.world,
            "arrange_work",
            &json!({"connections":[{"from":"checkpoint","to":"missing","label":"bad"}]})
        )
        .is_err()
    );
}
#[test]
fn completion_requires_evidence_and_book_projection_tracks_fact_changes() {
    let mut rt = setup();
    let mut p = json!({"id":"living-work","expected_revision":1,"title":"工作街区","summary":"还在试验","status":"completed","evidence_ids":["work-request"]});
    assert!(dispatch(&mut rt.world, "record_work", &p).is_err());
    p["evidence_ids"] = json!(["scene-acceptance"]);
    assert!(
        dispatch(&mut rt.world, "record_work", &p).is_err(),
        "Another task's acceptance cannot complete this task"
    );
    p["evidence_ids"] = json!(["work-request"]);
    p["status"] = json!("idea");
    dispatch(&mut rt.world, "record_work", &p).unwrap();
    let e = record_entity(&mut rt.world, "living-work").unwrap();
    assert_eq!(rt.world.get::<WorkRecord>(e).unwrap().revision, 2);
    assert_eq!(rt.world.get::<WorkAppearance>(e).unwrap().revision, 1);
    rt.world.get_mut::<WorkRecord>(e).unwrap().status = "completed".into();
    let snap = projection(&mut rt.world);
    assert_eq!(
        snap["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["record"]["id"] == "living-work")
            .unwrap()["status"],
        "unverified"
    );
}
#[test]
fn entering_runs_script_once_restores_and_does_not_call_a_model() {
    let mut rt = setup();
    let dir = std::env::temp_dir().join(format!("work-world-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("checkpoint.json");
    rt.set_checkpoint_path(&path);
    rt.register_system(crate::SystemDefinition{id:"welcome".into(),description:String::new(),system_prompt:"".into(),kind:crate::SystemKind::RhaiScript{source:"for row in query(\"WorkRecord\") {if row.WorkRecord.id==args.event.target_id {let a=get_component(row._entity_id,\"WorkAppearance\"); set_component(row._entity_id,\"WorkAppearance\",#{glow:a.glow+0.25});}}".into()},tool_names:vec![],max_iterations:1,is_tool:false}).unwrap();
    let trigger = rt
        .world
        .spawn(WorkTrigger {
            system_id: "welcome".into(),
            target_id: "checkpoint".into(),
            ..Default::default()
        })
        .id();
    let input = json!({"action":"work_enter","object_id":"checkpoint","interaction_id":"event-1"});
    rt.work_event(&input).unwrap();
    rt.work_event(&input).unwrap();
    assert_eq!(rt.world.get::<WorkTrigger>(trigger).unwrap().runs, 1);
    assert_eq!(rt.world.get::<WorkTrigger>(trigger).unwrap().error, "");
    let e = record_entity(&mut rt.world, "checkpoint").unwrap();
    assert_eq!(rt.world.get::<WorkAppearance>(e).unwrap().glow, 0.25);
    let mut restored = AgentRuntime::new_for_test();
    restored.set_checkpoint_path(&path);
    restored.restore(&path).unwrap();
    restored.initialize_work_district();
    assert_eq!(
        projection(&mut restored.world)["records"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    restored.work_event(&input).unwrap();
    let e = record_entity(&mut restored.world, "checkpoint").unwrap();
    assert_eq!(restored.world.get::<WorkAppearance>(e).unwrap().glow, 0.25);
    // Use a new interaction ID to run the persisted trigger again.
    let mut input = input;
    input["interaction_id"] = json!("event-2");
    restored.work_event(&input).unwrap();
    assert_eq!(restored.world.get::<WorkAppearance>(e).unwrap().glow, 0.5);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_trigger_keeps_partial_writes_disables_itself_and_bounds_event_history() {
    let mut rt = setup();
    let e = record_entity(&mut rt.world, "checkpoint").unwrap();
    let source = format!(
        "set_component({},\"WorkAppearance\",#{{glow:0.7}}); throw \"expected failure\";",
        e.index().index()
    );
    rt.register_system(crate::SystemDefinition {
        id: "fail-after-write".into(),
        description: String::new(),
        system_prompt: "".into(),
        kind: crate::SystemKind::RhaiScript { source },
        tool_names: vec![],
        max_iterations: 1,
        is_tool: false,
    })
    .unwrap();
    let t = rt
        .world
        .spawn(WorkTrigger {
            target_id: "checkpoint".into(),
            system_id: "fail-after-write".into(),
            ..Default::default()
        })
        .id();
    for i in 0..35 {
        rt.work_event(&json!({"action":"work_enter","object_id":"checkpoint","interaction_id":format!("event-{i}")})).unwrap();
    }
    let trigger = rt.world.get::<WorkTrigger>(t).unwrap();
    assert!(!trigger.enabled);
    assert!(trigger.error.contains("expected failure"));
    assert_eq!(trigger.runs, 0);
    assert_eq!(rt.world.get::<WorkAppearance>(e).unwrap().glow, 0.7);
    let snap = projection(&mut rt.world);
    assert_eq!(snap["events"].as_array().unwrap().len(), 32);
    assert_eq!(snap["events"][0]["sequence"], 4);
    assert_eq!(snap["events"][0]["status"], "no_handler");
}
