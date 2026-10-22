use super::*;
use crate::AgentRuntime;

const GOOD: &str = "let total = 0; for n in input { if n > 0 { total += n; } } total";
const BROAD: &str = "if obs.best >= 1.0 { [] } else { [0] }";

fn call(rt: &mut AgentRuntime, name: &str, input: Value) -> Result<Value, String> {
    dispatch(
        &mut rt.world,
        &ToolCall {
            id: "dream-test".into(),
            name: name.into(),
            input,
        },
    )
    .map(|s| serde_json::from_str(&s).unwrap())
}

fn task(rt: &mut AgentRuntime, action: &str) -> Value {
    call(
        rt,
        "dream_task",
        json!({"action":action,"id":"positive-sum"}),
    )
    .unwrap()
}

fn fixture() -> AgentRuntime {
    let mut rt = AgentRuntime::new_for_test();
    call(
        &mut rt,
        "dream_task",
        json!({"action":"create","id":"positive-sum","task":{
            "description":"Sum positive integers", "budget":6,
            "cases_json":json!([{"input":[-2,3,0,4],"expected":7},{"input":[8,-1],"expected":8}]).to_string()
        }}),
    )
    .unwrap();
    rt
}

fn attempt(rt: &mut AgentRuntime, script: &str) -> Value {
    let next = task(rt, "next");
    assert!(next.get("batch_id").is_some(), "{next}");
    call(rt,"dream_task",json!({"action":"submit","id":"positive-sum","batch_id":next["batch_id"],"scripts":[script]})).unwrap()
}

fn first_round(rt: &mut AgentRuntime) {
    for script in ["0", "0", "0", GOOD, GOOD, GOOD] {
        attempt(rt, script);
    }
}

fn improve(rt: &mut AgentRuntime, policies: Value) -> Value {
    let state = task(rt, "inspect");
    call(
        rt,
        "dream_policy",
        json!({"id":"positive-sum","expected_revision":state["revision"],"policies":policies}),
    )
    .unwrap()
}

#[test]
fn dream_full_loop_measures_replays_adopts_and_controls_new_discovery() {
    let mut rt = fixture();
    first_round(&mut rt);
    let before = rt.world.entities().len();
    let result = improve(&mut rt, json!([BROAD]));
    assert_eq!(result["adopted"], true);
    assert_eq!(result["baseline"][0]["attempts"], 6);
    assert_eq!(
        result["candidates"][0]["worlds"][0]["visited"],
        json!([1, 4])
    );
    assert_eq!(result["candidates"][0]["worlds"][0]["attempts"], 2);
    assert_eq!(result["new_candidate_executions"], 0);
    assert_eq!(rt.world.entities().len(), before);
    // Live selection uses the new controller, opening another root after a failure.
    attempt(&mut rt, "0");
    let next = task(&mut rt, "next");
    assert_eq!(next["parents"][0]["id"], 0);
    call(
        &mut rt,
        "dream_task",
        json!({"action":"submit","id":"positive-sum","batch_id":next["batch_id"],"scripts":[GOOD]}),
    )
    .unwrap();
    let stopped = task(&mut rt, "next");
    assert_eq!(stopped["stopped"], true);
    assert_eq!(stopped["result"]["measured"]["attempts"], 2);
    assert_eq!(stopped["result"]["measured"]["best"], 1.0);
    let replay_again = improve(&mut rt, json!([BROAD]));
    assert_eq!(replay_again["worlds"], 2);
    assert_eq!(replay_again["adopted"], false);
    assert!(rt.spark_queue.is_empty());
    assert_eq!(
        rt.world
            .query::<&crate::TickAction>()
            .iter(&rt.world)
            .count(),
        0
    );
    println!(
        "Dream-RSI fixture: online baseline 6 attempts; replay 2; next real sandbox round 2; both score 1.0. Scripted proposals, no LLM claim."
    );
}

#[test]
fn dream_replay_never_reads_unrevealed_scores_or_executes_recorded_scripts() {
    let mut tree = Tree::default();
    tree.nodes.push(replay::evaluate(
        "0",
        &[Case {
            input: json!(1),
            expected: json!(1),
        }],
        1,
        0,
    ));
    tree.nodes.push(replay::evaluate(
        "1",
        &[Case {
            input: json!(1),
            expected: json!(1),
        }],
        2,
        1,
    ));
    for node in &mut tree.nodes {
        node.script = "throw \"must not run\";".into();
    }
    // If future score=1 leaked into the first observation, this policy would stop early.
    let result = replay::replay(
        "if obs.best > 0.0 { [] } else if obs.nodes.len() == 0 { [0] } else { [obs.nodes[0].id] }",
        &tree,
        8,
        1,
    )
    .unwrap();
    assert_eq!(result.visited, vec![1, 2]);
    assert_eq!(result.best, 1.0);
    assert!(replay::replay("[999]", &tree, 8, 1).is_err());
    assert!(replay::replay("[0,0]", &tree, 8, 2).is_err());
    assert!(replay::replay("[obs.nodes[1].id]", &tree, 8, 1).is_err());
}

#[test]
fn dream_sandbox_contains_failures_and_has_no_live_capabilities() {
    let cases = vec![Case {
        input: json!(1),
        expected: json!(1),
    }];
    for script in [
        "loop {}",
        "spawn_entity(#{})",
        "import \"/etc/passwd\" as p; 1",
        "eval(\"1\")",
        "let x = ;",
    ] {
        let node = replay::evaluate(script, &cases, 1, 0);
        assert_eq!(node.passed, 0, "{script}");
        assert!(!node.errors.is_empty());
    }
    let mut rt = fixture();
    let result = attempt(&mut rt, "loop {}");
    assert_eq!(result["attempts"][0]["score"], 0.0);
    assert_eq!(task(&mut rt, "inspect")["active_attempts"], 1);
}

#[test]
fn dream_case_transport_preserves_json_types_without_guessing() {
    let mut rt = AgentRuntime::new_for_test();
    let cases = json!([
        {"input":7,"expected":7}, {"input":"7","expected":"7"},
        {"input":[],"expected":[]}, {"input":null,"expected":null},
        {"input":true,"expected":true}, {"input":{"x":2},"expected":{"x":2}}
    ]);
    call(
        &mut rt,
        "dream_task",
        json!({"action":"create","id":"positive-sum","task":{
            "description":"Preserve JSON values", "cases_json":cases.to_string(), "budget":1
        }}),
    )
    .unwrap();
    let stored = task(&mut rt, "inspect");
    assert_eq!(stored["task_spec"]["cases"], cases);
    let result = attempt(&mut rt, "input");
    assert_eq!(result["attempts"][0]["passed"], 6);
    let mismatch = replay::evaluate(
        "7",
        &[Case {
            input: json!(7),
            expected: json!("7"),
        }],
        1,
        0,
    );
    assert_eq!(mismatch.passed, 0);
    assert!(mismatch.errors[0].contains("actual=7, expected=\"7\""));
}

#[test]
fn dream_missing_continuation_marks_history_boundary_instead_of_invalid_policy() {
    let tree = Tree {
        nodes: vec![
            replay::evaluate(
                "0",
                &[Case {
                    input: json!(1),
                    expected: json!(1),
                }],
                1,
                0,
            ),
            replay::evaluate(
                "1",
                &[Case {
                    input: json!(1),
                    expected: json!(1),
                }],
                2,
                1,
            ),
        ],
        ..Tree::default()
    };
    let result = replay::replay("[0]", &tree, 8, 1).unwrap();
    assert_eq!(result.visited, vec![1]);
    assert!(result.history_boundary);
    assert_eq!(result.best, 0.0);
}

#[test]
fn dream_invalid_regressive_tied_and_stale_policies_cannot_replace_incumbent() {
    let mut rt = fixture();
    first_round(&mut rt);
    let before = task(&mut rt, "inspect");
    let result = improve(
        &mut rt,
        json!([
            "[]",
            "if obs.nodes.len() == 0 { [0] } else { [] }",
            BASELINE,
            "loop {}"
        ]),
    );
    assert_eq!(result["adopted"], false);
    assert_eq!(result["candidates"][1]["quality_preserved"], false);
    assert!(
        result["candidates"][3]["error"]
            .as_str()
            .unwrap()
            .contains("operations")
    );
    assert!(
        call(
            &mut rt,
            "dream_policy",
            json!({"id":"positive-sum","expected_revision":before["revision"],"policies":[BROAD]})
        )
        .unwrap_err()
        .contains("revision")
    );
    assert_eq!(task(&mut rt, "inspect")["policy"], BASELINE);
}

#[test]
fn dream_pending_receipts_are_idempotent_and_task_definition_is_frozen() {
    let mut rt = fixture();
    let next = task(&mut rt, "next");
    assert_eq!(next, task(&mut rt, "next"));
    let input =
        json!({"action":"submit","id":"positive-sum","batch_id":next["batch_id"],"scripts":[GOOD]});
    let first = call(&mut rt, "dream_task", input.clone()).unwrap();
    assert_eq!(first, call(&mut rt, "dream_task", input.clone()).unwrap());
    let mut changed = input;
    changed["scripts"] = json!(["0"]);
    assert!(
        call(&mut rt, "dream_task", changed)
            .unwrap_err()
            .contains("different scripts")
    );
    assert!(call(&mut rt,"dream_task",json!({"action":"create","id":"positive-sum","task":{"description":"different","cases_json":"[{\"input\":1,\"expected\":1}]"}})).unwrap_err().contains("immutable task"));
    let state = task(&mut rt, "inspect");
    assert_eq!(state["active_attempts"], 1);
    assert!(
        call(
            &mut rt,
            "dream_policy",
            json!({"id":"positive-sum","expected_revision":state["revision"],"policies":[BROAD]})
        )
        .unwrap_err()
        .contains("no active round")
    );
}

#[test]
fn dream_checkpoint_restores_policy_history_and_pending_batch() {
    let mut rt = fixture();
    first_round(&mut rt);
    improve(&mut rt, json!([BROAD]));
    let pending = task(&mut rt, "next");
    let dir = std::env::temp_dir().join(format!("dream-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("checkpoint.json");
    rt.checkpoint(&path).unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    assert_eq!(pending, task(&mut restored, "next"));
    let result = call(&mut restored,"dream_task",json!({"action":"submit","id":"positive-sum","batch_id":pending["batch_id"],"scripts":[GOOD]})).unwrap();
    assert_eq!(result["attempts"][0]["passed"], 2);
    task(&mut restored, "next");
    let state = task(&mut restored, "inspect");
    assert_eq!(state["completed_rounds"], 2);
    let rolled_back = call(
        &mut restored,
        "dream_task",
        json!({"action":"rollback","id":"positive-sum","expected_revision":state["revision"]}),
    )
    .unwrap();
    assert_eq!(rolled_back["rolled_back"], true);
    assert_eq!(task(&mut restored, "inspect")["policy"], BASELINE);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dream_limits_reject_unbounded_tasks_and_history_growth() {
    let mut rt = fixture();
    let mut invalid = json!({"action":"create","id":"bad","task":{"description":"x","cases_json":"[]","budget":100}});
    assert!(call(&mut rt, "dream_task", invalid.clone()).is_err());
    invalid["task"]["cases_json"] = json!("[{\"input\":1,\"expected\":1}]");
    assert!(call(&mut rt, "dream_task", invalid).is_err());
    first_round(&mut rt);
    for _ in 0..4 {
        improve(&mut rt, json!([BASELINE]));
    }
    let state = task(&mut rt, "inspect");
    assert!(
        call(
            &mut rt,
            "dream_policy",
            json!({"id":"positive-sum","expected_revision":state["revision"],"policies":[BASELINE]})
        )
        .unwrap_err()
        .contains("4 dreaming calls")
    );
    for _ in 1..MAX_WORLDS {
        first_round(&mut rt);
    }
    assert!(
        call(
            &mut rt,
            "dream_task",
            json!({"action":"next","id":"positive-sum"})
        )
        .unwrap_err()
        .contains("8 recorded rounds")
    );
    let record = rt
        .world
        .query::<&DreamExperiment>()
        .iter(&rt.world)
        .next()
        .unwrap();
    assert!(
        record.state_json.len() < 32_768,
        "small practice must stay compact"
    );
}

#[test]
fn dream_batch_policy_obeys_shared_online_and_replay_interface() {
    let tree = Tree {
        nodes: vec![
            replay::evaluate(
                "0",
                &[Case {
                    input: json!(1),
                    expected: json!(1),
                }],
                1,
                0,
            ),
            replay::evaluate(
                "1",
                &[Case {
                    input: json!(1),
                    expected: json!(1),
                }],
                2,
                0,
            ),
            replay::evaluate(
                "1",
                &[Case {
                    input: json!(1),
                    expected: json!(1),
                }],
                3,
                1,
            ),
        ],
        batches: 2,
        policy_version: 0,
    };
    let policy = "if obs.nodes.len() == 0 { [0] } else { [0,1] }";
    let result = replay::replay(policy, &tree, 3, 2).unwrap();
    assert_eq!(result.visited, vec![1, 2, 3]);
    assert_eq!(result.batches, 2);
    assert!(replay::replay(policy, &tree, 3, 1).is_err());
}
