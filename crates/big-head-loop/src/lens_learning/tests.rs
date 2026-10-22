use super::*;
use big_head_ecs::{
    TickAction,
    lens::{LensBinding, LensInstance, LensTrigger},
};

fn fixture() -> (crate::AgentRuntime, Entity) {
    let mut rt = crate::AgentRuntime::new_for_test();
    let tick = rt
        .world
        .spawn(TickAction {
            enabled: true,
            script: "weather".repeat(80),
            last_run: 12,
            cooldown_secs: 21600,
        })
        .id();
    rt.world.spawn(LensApplication { id:"weather.old".into(),definition_json:json!({"id":"weather.old","version":"1","purpose":"天气刷新状态","usage":{"tasks":["检查天气数据是否更新"],"tags":["weather"]},"reads":["TickAction"],"stage":{"kind":"identity"}}).to_string(),last_run_json:"keep old receipt".into() });
    let binding = LensBinding {
        id: "chat.weather".into(),
        lens_id: "weather.old".into(),
        lens_version: "1".into(),
        agent: "coordinator".into(),
        trigger: LensTrigger::BeforeReasoning,
        enabled: true,
        priority: 1,
        source_updated_at_pointer: None,
        stale_threshold_ms: None,
        refresh_interval_ms: 0,
        revision: 1,
    };
    rt.world
        .spawn((binding.clone(), LensInstance::new(&binding)));
    (rt, tick)
}

fn call(rt: &mut crate::AgentRuntime, name: &str, input: Value) -> Result<Value, String> {
    dispatch(
        &mut rt.world,
        &rt.component_registry,
        &ToolCall {
            id: "test".into(),
            name: name.into(),
            input,
        },
    )
    .map(|s| serde_json::from_str(&s).unwrap())
}

fn proposal() -> Value {
    json!({"definition":{"id":"weather.focused","version":"1","purpose":"紧凑天气更新时间观察","usage":{"tasks":["天气更新时间"],"limitations":["运行时间不证明天气来源已更新"]},"reads":["TickAction"],"stage":{"kind":"project","fields":{"last_run":"/TickAction/last_run"}}},"based_on":{"id":"weather.old","version":"1"},"reason":"已有观察包含过多脚本，只需要运行时间；它不证明天气更新"})
}

fn trial(rt: &mut crate::AgentRuntime) -> Value {
    call(rt,"trial_lens_candidate",json!({"candidate_id":"weather.focused","checks":{"required_fields":["/last_run"],"max_output_bytes":2048}})).unwrap()
}

#[test]
fn feedback_candidate_trial_and_adoption_preserve_live_bindings() {
    let (mut rt, tick) = fixture();
    rt.world.insert_resource(DispatchContext {
        source_id: "user-spark".into(),
        correlation_id: "task".into(),
    });
    let found = call(
        &mut rt,
        "discover_lenses",
        json!({"query":"看看天气数据更新"}),
    )
    .unwrap();
    assert_eq!(found["matches"][0]["id"], "weather.old");
    let feedback = call(&mut rt,"record_lens_feedback",json!({"lens_id":"weather.old","kind":"correction","note":"不要只说运行成功，保留观察的局限"})).unwrap();
    assert_eq!(feedback["feedback"]["source_id"], "user-spark");
    let mut p = proposal();
    p["feedback_ids"] = json!([feedback["feedback"]["id"]]);
    call(&mut rt, "propose_lens_candidate", p).unwrap();
    assert!(
        !crate::lens_runtime::catalog(&mut rt.world)
            .unwrap()
            .1
            .contains_key("weather.focused")
    );
    assert_eq!(
        call(&mut rt, "discover_lenses", json!({"query":"天气"})).unwrap()["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let t = trial(&mut rt);
    assert_eq!(t["passed"], true);
    assert!(
        t["candidate_metrics"]["output_bytes"].as_u64().unwrap()
            < t["baseline_metrics"]["output_bytes"].as_u64().unwrap()
    );
    let adopted = call(&mut rt,"adopt_lens_candidate",json!({"candidate_id":"weather.focused","trial_id":t["trial_id"],"reason":"满足所需字段且减少脚本上下文，保留来源新鲜度局限"})).unwrap();
    assert_eq!(adopted["bindings_changed"], false);
    let repeated = call(&mut rt,"adopt_lens_candidate",json!({"candidate_id":"weather.focused","trial_id":t["trial_id"],"reason":"retry same adoption"})).unwrap();
    assert_eq!(repeated["reused"], true);
    let binding = rt
        .world
        .query::<&LensBinding>()
        .iter(&rt.world)
        .next()
        .unwrap();
    assert_eq!(binding.lens_id, "weather.old");
    assert_eq!(binding.revision, 1);
    assert!(rt.spark_queue.is_empty());
    assert_eq!(rt.world.get::<TickAction>(tick).unwrap().last_run, 12);
    assert_eq!(
        crate::lens_runtime::catalog(&mut rt.world).unwrap().1.len(),
        2
    );
}

#[test]
fn comparison_keeps_each_lens_read_scope_on_shared_inputs() {
    let (mut rt, _) = fixture();
    let mut p = proposal();
    p["definition"]["reads"] = json!(["TickAction", "LensBinding"]);
    p["definition"]["stage"] = json!({"kind":"identity"});
    call(&mut rt, "propose_lens_candidate", p).unwrap();
    let result = call(
        &mut rt,
        "trial_lens_candidate",
        json!({"candidate_id":"weather.focused","checks":{"max_output_bytes":65536}}),
    )
    .unwrap();
    assert_eq!(result["passed"], true);
    assert_eq!(result["baseline_metrics"]["facts"], 1);
    assert_eq!(result["candidate_metrics"]["facts"], 2);
}

#[test]
fn trial_preserves_membership_after_projection_and_rechecks_it_before_adoption() {
    let (mut rt, tick) = fixture();
    rt.component_registry
        .define_dynamic_component(
            &mut rt.world,
            "Completed",
            big_head_ecs::schema::DynamicSchema {
                fields: Default::default(),
            },
        )
        .unwrap();
    let mut p = proposal();
    p["definition"]["stage"] = json!({"kind":"sequence","stages":[
        {"kind":"project","fields":{"last_run":"/TickAction/last_run"}},
        {"kind":"archetype_without","component":"Completed"}
    ]});
    call(&mut rt, "propose_lens_candidate", p).unwrap();
    let first = trial(&mut rt);
    assert_eq!(first["passed"], true);
    assert_eq!(first["baseline_metrics"]["facts"], 1);
    rt.component_registry
        .insert_component(&mut rt.world, tick, "Completed", "{}")
        .unwrap();
    assert!(call(&mut rt, "adopt_lens_candidate", json!({
        "candidate_id":"weather.focused", "trial_id":first["trial_id"], "reason":"old membership"
    })).is_err());
    let second = trial(&mut rt);
    assert_eq!(second["passed"], false);
    assert_eq!(second["candidate_metrics"]["facts"], 0);
    assert_eq!(second["baseline_metrics"]["facts"], 1);
}

#[test]
fn task_discovery_is_metadata_only_and_never_activates_candidates() {
    let (mut rt, _) = fixture();
    let before = rt.world.entities().len();
    let text = task_context(&mut rt.world, "检查天气是否更新");
    assert!(text.contains("weather.old"));
    assert!(text.contains("No Lens was executed"));
    assert_eq!(before, rt.world.entities().len());
    assert!(rt.spark_queue.is_empty());
}

#[test]
fn missing_fields_and_empty_world_cannot_pass_trial() {
    let (mut rt, tick) = fixture();
    call(&mut rt, "propose_lens_candidate", proposal()).unwrap();
    let failed = call(
        &mut rt,
        "trial_lens_candidate",
        json!({"candidate_id":"weather.focused","checks":{"required_fields":["/not_present"]}}),
    )
    .unwrap();
    assert_eq!(failed["passed"], false);
    assert!(call(&mut rt,"adopt_lens_candidate",json!({"candidate_id":"weather.focused","trial_id":failed["trial_id"],"reason":"cannot adopt"})).is_err());
    rt.world.despawn(tick);
    assert_eq!(trial(&mut rt)["passed"], false);
    let inspected = call(
        &mut rt,
        "inspect_lens_learning",
        json!({"candidate_id":"weather.focused","details":true}),
    )
    .unwrap();
    assert_eq!(inspected["candidates"][0]["status"], "failed");
}

#[test]
fn source_change_requires_a_new_reviewed_trial() {
    let (mut rt, tick) = fixture();
    call(&mut rt, "propose_lens_candidate", proposal()).unwrap();
    let first = trial(&mut rt);
    rt.world.get_mut::<TickAction>(tick).unwrap().last_run = 34;
    assert!(call(&mut rt,"adopt_lens_candidate",json!({"candidate_id":"weather.focused","trial_id":first["trial_id"],"reason":"old evidence"})).unwrap_err().contains("evidence changed"));
    assert_eq!(
        crate::lens_runtime::catalog(&mut rt.world).unwrap().1.len(),
        1
    );
    let second = trial(&mut rt);
    call(&mut rt,"adopt_lens_candidate",json!({"candidate_id":"weather.focused","trial_id":second["trial_id"],"reason":"reviewed current inputs"})).unwrap();
}

#[test]
fn candidate_update_invalidates_old_trial_and_enforces_revision() {
    let (mut rt, _) = fixture();
    call(&mut rt, "propose_lens_candidate", proposal()).unwrap();
    let first = trial(&mut rt);
    assert!(call(&mut rt, "propose_lens_candidate", proposal()).is_err());
    let mut updated = proposal();
    updated["expected_revision"] = json!(1);
    updated["definition"]["stage"]["fields"] =
        json!({"last_run":"/TickAction/last_run","cooldown":"/TickAction/cooldown_secs"});
    call(&mut rt, "propose_lens_candidate", updated).unwrap();
    assert!(call(&mut rt,"adopt_lens_candidate",json!({"candidate_id":"weather.focused","trial_id":first["trial_id"],"reason":"stale trial"})).is_err());
    assert_eq!(trial(&mut rt)["candidate_revision"], 2);
}

#[test]
fn invalid_feedback_or_published_id_is_rejected_without_mutation() {
    let (mut rt, _) = fixture();
    let mut p = proposal();
    p["feedback_ids"] = json!(["invented"]);
    assert!(call(&mut rt, "propose_lens_candidate", p).is_err());
    p = proposal();
    p["definition"]["id"] = json!("weather.old");
    assert!(call(&mut rt, "propose_lens_candidate", p).is_err());
    assert!(candidates(&mut rt.world).unwrap().is_empty());
    let input = json!({"lens_id":"weather.old","kind":"gap","note":"need context"});
    let first = call(&mut rt, "record_lens_feedback", input.clone()).unwrap();
    let second = call(&mut rt, "record_lens_feedback", input).unwrap();
    assert_eq!(first["feedback"]["id"], second["feedback"]["id"]);
    assert_eq!(second["reused"], true);
}

#[test]
fn unknown_inputs_record_failure_without_publishing() {
    let (mut rt, _) = fixture();
    let mut p = proposal();
    p["definition"]["reads"] = json!(["UnknownSource"]);
    call(&mut rt, "propose_lens_candidate", p).unwrap();
    let result = trial(&mut rt);
    assert_eq!(result["passed"], false);
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("unknown input component")
    );
    assert_eq!(
        crate::lens_runtime::catalog(&mut rt.world).unwrap().1.len(),
        1
    );
}

#[test]
fn candidates_feedback_and_trials_survive_checkpoint() {
    let (mut rt, _) = fixture();
    call(
        &mut rt,
        "record_lens_feedback",
        json!({"lens_id":"weather.old","kind":"useful","note":"baseline works"}),
    )
    .unwrap();
    call(&mut rt, "propose_lens_candidate", proposal()).unwrap();
    let tested = trial(&mut rt);
    let dir = std::env::temp_dir().join(format!("lens-learning-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    rt.checkpoint(&dir.join("checkpoint.json")).unwrap();
    let mut restored = crate::AgentRuntime::new_for_test();
    restored.restore(&dir.join("checkpoint.json")).unwrap();
    let inspection = call(
        &mut restored,
        "inspect_lens_learning",
        json!({"candidate_id":"weather.focused"}),
    )
    .unwrap();
    assert_eq!(
        inspection["candidates"][0]["trial"]["trial_id"],
        tested["trial_id"]
    );
    assert_eq!(inspection["feedback"].as_array().unwrap().len(), 1);
    assert_eq!(
        crate::lens_runtime::catalog(&mut restored.world)
            .unwrap()
            .1
            .len(),
        1
    );
    assert_eq!(trial(&mut restored)["passed"], true);
    std::fs::remove_dir_all(dir).unwrap();
}
