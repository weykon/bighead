use super::*;
use crate::{AgentRuntime, system_def::SystemDefinition};
use std::{borrow::Cow, path::PathBuf};

fn fixture() -> (AgentRuntime, PathBuf) {
    let mut rt = AgentRuntime::new_for_test();
    rt.define_dynamic_component(
        "ActivityProbe",
        crate::dynamic_component::parse_schema_from_json(
            &json!({"id":"string","n":"integer","approved":"boolean"}),
        )
        .unwrap(),
    )
    .unwrap();
    let entity = rt.world.spawn_empty().id();
    rt.component_registry
        .insert_component(
            &mut rt.world,
            entity,
            "ActivityProbe",
            r#"{"id":"one","n":0,"approved":false}"#,
        )
        .unwrap();
    register(
        &mut rt,
        "increment",
        r#"for e in query("ActivityProbe") { let c=get_component(e._entity_id,"ActivityProbe"); if c.id=="one" { c.n+=1; set_component(e._entity_id,"ActivityProbe",c); } }"#,
    );
    let root = std::env::temp_dir().join(format!("bighead-activity-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    (rt, root.join("checkpoint.json"))
}

fn register(rt: &mut AgentRuntime, id: &str, source: &str) {
    rt.system_registry
        .register(SystemDefinition {
            id: id.into(),
            description: "test".into(),
            system_prompt: Cow::Borrowed(""),
            kind: SystemKind::RhaiScript {
                source: source.into(),
            },
            tool_names: vec![],
            max_iterations: 1,
            is_tool: false,
        })
        .unwrap();
}

fn check(field: &str, equals: Value) -> Value {
    json!({"component":"ActivityProbe","key_pointer":"/id","key":"one","value_pointer":field,"equals":equals})
}

fn plan() -> Value {
    json!({"id":"demo","scope":"test:project","goal":"two increments after approval",
        "steps":[{"id":"first","system_id":"increment","verify":[check("/n",json!(1))]},
            {"id":"second","system_id":"increment","wait_for":[check("/approved",json!(true))],"verify":[check("/n",json!(2))]}],
        "acceptance":[check("/n",json!(2))]})
}

fn tool(rt: &mut AgentRuntime, name: &str, input: Value) -> Result<Value, String> {
    dispatch(
        &mut rt.world,
        &rt.component_registry,
        &rt.system_registry,
        name,
        &input,
    )
}
fn create(rt: &mut AgentRuntime, p: Value) -> String {
    tool(rt, "create_activity", p).unwrap()["activity"]["run_id"]
        .as_str()
        .unwrap()
        .into()
}
fn state(rt: &mut AgentRuntime, id: &str) -> ActivityRun {
    let e = find(&mut rt.world, id).unwrap();
    rt.world.get::<ActivityRun>(e).unwrap().clone()
}
fn control(rt: &mut AgentRuntime, id: &str, action: &str) -> Result<Value, String> {
    let revision = state(rt, id).revision;
    tool(
        rt,
        "control_activity",
        json!({"run_id":id,"expected_revision":revision,"action":action}),
    )
}
fn domain(rt: &mut AgentRuntime, n: i64, approved: bool) {
    let cid = rt
        .component_registry
        .dynamic_store()
        .get_id("ActivityProbe")
        .unwrap();
    let e = rt
        .world
        .iter_entities()
        .find(|e| crate::dynamic_component::read(&rt.world, e.id(), cid).is_some())
        .unwrap()
        .id();
    rt.component_registry
        .insert_component(
            &mut rt.world,
            e,
            "ActivityProbe",
            &json!({"id":"one","n":n,"approved":approved}).to_string(),
        )
        .unwrap();
}

#[test]
fn two_steps_wait_restore_and_accept_goal_without_llm() {
    let (mut rt, path) = fixture();
    let id = create(&mut rt, plan());
    control(&mut rt, &id, "start").unwrap();
    runner::advance(&mut rt, &path, 10).unwrap();
    let first = state(&mut rt, &id);
    assert_eq!(first.cursor, 1, "{first:?}");
    runner::advance(&mut rt, &path, 20).unwrap();
    assert_eq!(state(&mut rt, &id).status, Status::Waiting);
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    assert_eq!(state(&mut restored, &id).status, Status::Waiting);
    domain(&mut restored, 1, true);
    runner::advance(&mut restored, &path, 30).unwrap();
    assert_eq!(state(&mut restored, &id).status, Status::Ready);
    runner::advance(&mut restored, &path, 40).unwrap();
    let run = state(&mut restored, &id);
    assert_eq!(run.status, Status::Completed, "{}", run.reason);
    assert_eq!(run.records.len(), 2);
    assert!(run.records.iter().all(|r| r.outcome == "verified"));
    assert!(all_pass(&run.evidence));
    assert!(runner::advance(&mut restored, &path, 50).unwrap().is_none());
    println!(
        "activity {}: 2 verified steps, waiting survived restore, goal independently accepted",
        id
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn dedup_revision_pause_cancel_and_hash_guard() {
    let (mut rt, path) = fixture();
    let id = create(&mut rt, plan());
    assert_eq!(create(&mut rt, plan()), id);
    let mut different = plan();
    different["goal"] = json!("different");
    assert!(tool(&mut rt, "create_activity", different).is_err());
    control(&mut rt, &id, "start").unwrap();
    assert!(
        tool(
            &mut rt,
            "control_activity",
            json!({"run_id":id,"expected_revision":1,"action":"cancel"})
        )
        .is_err()
    );
    control(&mut rt, &id, "pause").unwrap();
    assert!(runner::advance(&mut rt, &path, 10).unwrap().is_none());
    control(&mut rt, &id, "resume").unwrap();
    let mut updated = rt.system_registry.get("increment").unwrap().clone();
    updated.kind = SystemKind::RhaiScript {
        source: "log(\"changed\");".into(),
    };
    rt.system_registry.replace(updated).unwrap();
    runner::advance(&mut rt, &path, 20).unwrap();
    let run = state(&mut rt, &id);
    assert_eq!(run.status, Status::Blocked);
    assert!(run.records.is_empty());
    control(&mut rt, &id, "cancel").unwrap();
    assert!(control(&mut rt, &id, "resume").is_err());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn revising_draft_or_blocked_work_does_not_authorize_execution() {
    for initial in [Status::Draft, Status::Blocked] {
        let (mut rt, path) = fixture();
        let id = create(&mut rt, plan());
        if initial == Status::Blocked {
            let e = find(&mut rt.world, &id).unwrap();
            rt.world.get_mut::<ActivityRun>(e).unwrap().change(
                Status::Blocked,
                1,
                "review required",
            );
        }
        let before = state(&mut rt, &id);
        tool(
            &mut rt,
            "update_activity_plan",
            json!({"run_id":id,"expected_revision":before.revision,"plan":plan()}),
        )
        .unwrap();
        runner::advance(&mut rt, &path, 10).unwrap();
        let after = state(&mut rt, &id);
        assert_eq!(after.status, initial);
        assert_eq!(after.cursor, 0);
        assert!(after.records.is_empty());
        control(
            &mut rt,
            &id,
            if initial == Status::Draft {
                "start"
            } else {
                "resume"
            },
        )
        .unwrap();
        runner::advance(&mut rt, &path, 20).unwrap();
        assert_eq!(state(&mut rt, &id).cursor, 1);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

#[test]
fn agent_can_revise_future_steps_without_rewriting_completed_history() {
    let (mut rt, path) = fixture();
    let id = create(&mut rt, plan());
    control(&mut rt, &id, "start").unwrap();
    runner::advance(&mut rt, &path, 10).unwrap();
    let before = state(&mut rt, &id);
    assert_eq!(before.cursor, 1);
    control(&mut rt, &id, "pause").unwrap();
    let mut revised = plan();
    revised["goal"] = json!("approved second increment with revised acceptance");
    revised["steps"][0]["id"] = json!("first-revised");
    revised["steps"][1]["id"] = json!("second-revised");
    // A completed position cannot be renamed, but a future step can be replaced.
    let revision = state(&mut rt, &id).revision;
    assert!(
        tool(
            &mut rt,
            "update_activity_plan",
            json!({"run_id":id,"expected_revision":revision,"plan":revised.clone()})
        )
        .is_err()
    );
    revised["steps"][0]["id"] = json!("first");
    let mut rewritten = revised.clone();
    rewritten["steps"][0]["verify"][0]["equals"] = json!(999);
    assert!(
        tool(
            &mut rt,
            "update_activity_plan",
            json!({"run_id":id,"expected_revision":revision,"plan":rewritten})
        )
        .is_err()
    );
    revised["steps"][1]["id"] = json!("second-revised");
    let revision = state(&mut rt, &id).revision;
    let updated = tool(
        &mut rt,
        "update_activity_plan",
        json!({"run_id":id,"expected_revision":revision,"plan":revised}),
    )
    .unwrap();
    assert_eq!(updated["updated"], true);
    let after = state(&mut rt, &id);
    assert_eq!(after.cursor, 1);
    assert_eq!(after.records.len(), 1);
    assert_eq!(after.plan_history.len(), 1);
    assert_eq!(after.status, Status::Paused);
    domain(&mut rt, 1, true);
    runner::advance(&mut rt, &path, 20).unwrap();
    assert_eq!(state(&mut rt, &id).cursor, 1);
    rt.checkpoint(&path).unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    runner::advance(&mut restored, &path, 25).unwrap();
    assert_eq!(state(&mut restored, &id).status, Status::Paused);
    assert_eq!(state(&mut restored, &id).cursor, 1);
    assert_eq!(state(&mut restored, &id).plan_history.len(), 1);
    assert_eq!(
        state(&mut restored, &id).records[0].tx_id,
        before.records[0].tx_id
    );
    domain(&mut restored, 1, true);
    control(&mut restored, &id, "resume").unwrap();
    runner::advance(&mut restored, &path, 30).unwrap();
    runner::advance(&mut restored, &path, 40).unwrap();
    let finished = state(&mut restored, &id);
    assert_eq!(finished.status, Status::Completed, "{finished:?}");
    assert_eq!(finished.records.len(), 2);
    assert_eq!(finished.records[1].step_id, "second-revised");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn crashed_running_step_requires_evidence_not_replay() {
    let (mut rt, path) = fixture();
    let id = create(&mut rt, plan());
    control(&mut rt, &id, "start").unwrap();
    let e = find(&mut rt.world, &id).unwrap();
    {
        let mut run = rt.world.get_mut::<ActivityRun>(e).unwrap();
        run.status = Status::Running;
        run.uncertain_step = true;
    }
    rt.checkpoint(&path).unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    assert_eq!(state(&mut restored, &id).status, Status::Blocked);
    assert!(control(&mut restored, &id, "resume").is_err());
    control(&mut restored, &id, "recheck").unwrap();
    assert!(state(&mut restored, &id).uncertain_step);
    assert!(runner::advance(&mut restored, &path, 20).unwrap().is_none());
    domain(&mut restored, 1, false);
    control(&mut restored, &id, "recheck").unwrap();
    assert_eq!(state(&mut restored, &id).cursor, 1);
    runner::advance(&mut restored, &path, 30).unwrap();
    assert_eq!(state(&mut restored, &id).status, Status::Waiting);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn tool_success_is_not_goal_success_and_ambiguous_keys_fail() {
    let (mut rt, path) = fixture();
    let mut p = plan();
    p["steps"].as_array_mut().unwrap().truncate(1);
    let id = create(&mut rt, p);
    control(&mut rt, &id, "start").unwrap();
    runner::advance(&mut rt, &path, 10).unwrap();
    runner::advance(&mut rt, &path, 20).unwrap();
    assert_eq!(state(&mut rt, &id).status, Status::Blocked);
    domain(&mut rt, 2, true);
    control(&mut rt, &id, "recheck").unwrap();
    assert_eq!(state(&mut rt, &id).status, Status::Completed);
    let checks: Vec<Check> = serde_json::from_value(json!([check("/n", json!(2))])).unwrap();
    let duplicate = rt.world.spawn_empty().id();
    rt.component_registry
        .insert_component(
            &mut rt.world,
            duplicate,
            "ActivityProbe",
            r#"{"id":"one","n":2,"approved":true}"#,
        )
        .unwrap();
    assert!(!all_pass(&read_checks(
        &rt.world,
        &rt.component_registry,
        &checks,
        30
    )));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn partial_execution_failure_and_missing_checkpoint_do_not_auto_retry() {
    let (mut rt, path) = fixture();
    register(
        &mut rt,
        "failing",
        r#"for e in query("ActivityProbe") { let c=get_component(e._entity_id,"ActivityProbe"); c.n+=1; set_component(e._entity_id,"ActivityProbe",c); } throw "after mutation";"#,
    );
    let mut p = plan();
    p["steps"][0]["system_id"] = json!("failing");
    let id = create(&mut rt, p);
    control(&mut rt, &id, "start").unwrap();
    runner::advance(&mut rt, &path, 10).unwrap();
    assert_eq!(state(&mut rt, &id).status, Status::Blocked);
    assert!(state(&mut rt, &id).uncertain_step);
    let record = state(&mut rt, &id).records.remove(0);
    assert!(
        record.output.contains("after mutation"),
        "{}",
        record.output
    );
    assert_eq!(record.evidence[0].actual, Some(json!(1)));
    assert!(runner::advance(&mut rt, &path, 20).unwrap().is_none());
    assert!(control(&mut rt, &id, "resume").is_err());
    let mut p = plan();
    p["id"] = json!("no-save");
    let other = create(&mut rt, p);
    control(&mut rt, &other, "start").unwrap();
    // A directory cannot be replaced by the checkpoint file.
    assert!(runner::advance(&mut rt, path.parent().unwrap(), 30).is_err());
    assert!(state(&mut rt, &other).records.is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn deadline_and_script_budget_stop_work_without_starving_other_runs() {
    let (mut rt, path) = fixture();
    let mut expired = plan();
    expired["deadline_epoch"] = json!(5);
    let first = create(&mut rt, expired);
    control(&mut rt, &first, "start").unwrap();
    runner::advance(&mut rt, &path, 10).unwrap();
    assert_eq!(state(&mut rt, &first).status, Status::Blocked);
    assert!(state(&mut rt, &first).records.is_empty());
    register(&mut rt, "endless", "loop {}");
    let mut p = plan();
    p["id"] = json!("budget");
    p["steps"][0]["system_id"] = json!("endless");
    let second = create(&mut rt, p);
    control(&mut rt, &second, "start").unwrap();
    runner::advance(&mut rt, &path, 20).unwrap();
    let blocked = state(&mut rt, &second);
    assert_eq!(blocked.status, Status::Blocked);
    assert!(
        blocked.records[0]
            .output
            .to_lowercase()
            .contains("operations"),
        "{}",
        blocked.records[0].output
    );
    let mut p = plan();
    p["id"] = json!("healthy");
    let third = create(&mut rt, p);
    control(&mut rt, &third, "start").unwrap();
    runner::advance(&mut rt, &path, 30).unwrap();
    assert_eq!(state(&mut rt, &third).cursor, 1);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn another_dynamic_domain_uses_same_runner_and_foreground_has_priority() {
    let (mut rt, path) = fixture();
    rt.define_dynamic_component(
        "DemoOrder",
        crate::dynamic_component::parse_schema_from_json(
            &json!({"order_id":"string","paid":"boolean","status":"string"}),
        )
        .unwrap(),
    )
    .unwrap();
    let entity = rt.world.spawn_empty().id();
    rt.component_registry
        .insert_component(
            &mut rt.world,
            entity,
            "DemoOrder",
            r#"{"order_id":"o-1","paid":false,"status":"new"}"#,
        )
        .unwrap();
    register(
        &mut rt,
        "fulfill_demo",
        r#"for row in query("DemoOrder") { let order=get_component(row._entity_id,"DemoOrder"); if order.order_id==args.order_id { order.status="fulfilled"; set_component(row._entity_id,"DemoOrder",order); } }"#,
    );
    let expected = json!({"component":"DemoOrder","key_pointer":"/order_id","key":"o-1","value_pointer":"/status","equals":"fulfilled"});
    let id = create(
        &mut rt,
        json!({"id":"order-v1","scope":"demo:orders","goal":"fulfill a paid simulated order",
        "steps":[{"id":"fulfill","system_id":"fulfill_demo","args":{"order_id":"o-1"},
            "wait_for":[{"component":"DemoOrder","key_pointer":"/order_id","key":"o-1","value_pointer":"/paid","equals":true}],"verify":[expected.clone()]}],"acceptance":[expected]}),
    );
    control(&mut rt, &id, "start").unwrap();
    let spark = rt.push_spark_front("foreground fixture".into());
    assert!(runner::advance(&mut rt, &path, 10).unwrap().is_none());
    rt.world.despawn(spark);
    rt.spark_queue.clear();
    runner::advance(&mut rt, &path, 20).unwrap();
    assert_eq!(state(&mut rt, &id).status, Status::Waiting);
    rt.component_registry
        .patch_component(&mut rt.world, entity, "DemoOrder", r#"{"paid":true}"#)
        .unwrap();
    runner::advance(&mut rt, &path, 30).unwrap();
    runner::advance(&mut rt, &path, 40).unwrap();
    let run = state(&mut rt, &id);
    assert_eq!(run.status, Status::Completed, "{run:?}");
    assert_eq!(run.records.len(), 1);
    assert_eq!(run.evidence[0].actual, Some(json!("fulfilled")));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
