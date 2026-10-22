use super::*;

fn report(error: Option<&str>, at: u64) -> TickActionReport {
    TickActionReport {
        entity: bevy_ecs::entity::Entity::from_raw_u32(1).unwrap(),
        source_id: "stable-source".into(),
        correlation_id: format!("run-{at}"),
        spark_ids: vec![],
        script: "unused".into(),
        script_hash: "hash-v1".into(),
        started_at_epoch: at,
        duration_ms: 1,
        queued_sparks: 0,
        logs: vec![],
        error: error.map(String::from),
        state: None,
    }
}

#[test]
fn incidents_coalesce_by_identity_and_follow_real_execution() {
    let mut tracker = Tracker::default();
    assert!(tracker.ingest(&report(None, 1)).is_empty());
    let opened = tracker.ingest(&report(Some("schema missing"), 2));
    let id = opened[0]["incident_id"].clone();
    let mut rotated = report(Some("schema missing"), 3);
    rotated.entity = bevy_ecs::entity::Entity::from_raw_u32(77).unwrap();
    let followup = tracker.ingest(&rotated);
    assert_eq!(followup.len(), 1);
    assert_eq!(followup[0]["incident_id"], id);
    assert_eq!(followup[0]["kind"], "next_execution");
    assert_eq!(followup[0]["evidence"]["execution_succeeded"], false);
    for n in 4..100 {
        assert!(
            tracker
                .ingest(&report(Some("schema missing"), n))
                .is_empty()
        );
    }
    let mut fixed = report(None, 100);
    fixed.script_hash = "hash-v2".into();
    let recovered = tracker.ingest(&fixed);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0]["kind"], "execution_recovered");
    assert_eq!(recovered[0]["incident_id"], id);
    assert_eq!(recovered[0]["training_eligible"], false);
    assert!(recovered[0]["goal_achieved"].is_null());
    assert!(tracker.pending.is_empty());
}

#[test]
fn changed_errors_close_old_incident_and_tracking_is_bounded() {
    let mut tracker = Tracker::default();
    tracker.ingest(&report(Some("first error"), 1));
    let events = tracker.ingest(&report(Some("second error"), 2));
    assert!(events.iter().any(|e| e["kind"] == "incident_closed"));
    assert!(events.iter().any(|e| e["kind"] == "incident_opened"));
    for n in 0..200 {
        let mut r = report(Some("error"), n);
        r.source_id = format!("source-{n}");
        tracker.ingest(&r);
    }
    assert_eq!(tracker.pending.len(), MAX_PENDING);
}

#[test]
fn queue_saturation_drops_evidence_without_blocking_or_world_writes() {
    let (tx, _rx) = mpsc::channel(1);
    let mut shadow = Shadow {
        tracker: Tracker::default(),
        tx,
        task: None,
        counters: Arc::new(Mutex::new(Counters::default())),
        directory: PathBuf::new(),
    };
    shadow.ingest(&[report(Some("error"), 1), report(None, 2)]);
    let counts = shadow.counters.lock().unwrap();
    assert_eq!(counts.queued, 1);
    assert_eq!(counts.dropped, 2);
}

#[tokio::test]
async fn unavailable_model_keeps_followup_evidence_and_opens_circuit() {
    let root = std::env::temp_dir().join(format!("laya-shadow-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("events.jsonl");
    let file = std::fs::File::create(&path).unwrap();
    let (tx, rx) = mpsc::channel(8);
    let counts = Arc::new(Mutex::new(Counters::default()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let task = tokio::spawn(worker(rx, file, base, counts.clone()));
    let mut tracker = Tracker::default();
    for r in [
        report(Some("error"), 1),
        report(None, 2),
        report(Some("new error"), 3),
    ] {
        for event in tracker.ingest(&r) {
            tx.send(event).await.unwrap();
        }
    }
    drop(tx);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let records: Vec<Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(records.iter().any(|e| e["kind"] == "execution_recovered"));
    assert_eq!(counts.lock().unwrap().errors, 1);
    assert_eq!(counts.lock().unwrap().skipped, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "Requires real local Laya service; uses an isolated test world"]
async fn laya_live_shadow_records_failure_and_recovery_without_dispatch() {
    let mut rt = AgentRuntime::new_for_test();
    rt.enable_laya_shadow(std::path::Path::new("outputs/laya"))
        .unwrap();
    let path = rt.world.resource::<Shadow>().directory.join("events.jsonl");
    let e = rt
        .world
        .spawn(crate::TickAction {
            enabled: true,
            script: "throw(\"missing test schema\");".into(),
            ..Default::default()
        })
        .id();
    rt.evaluate_tick_actions();
    rt.world.get_mut::<crate::TickAction>(e).unwrap().script = "log(\"test repaired\");".into();
    rt.world.get_mut::<crate::TickAction>(e).unwrap().last_run = 0;
    rt.evaluate_tick_actions();
    assert!(rt.spark_queue.is_empty());
    rt.finish_laya_shadow().await;
    let records: Vec<Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .any(|r| r["kind"] == "shadow_prediction" && r["prediction"]["backend"] == "mlx")
    );
    assert!(records.iter().any(|r| r["kind"] == "execution_recovered"));
    assert!(records.iter().all(|r| r["training_eligible"] == false));
    println!("Isolated shadow evidence: {}", path.display());
}

#[test]
fn all_four_workflows_use_fixed_non_executable_routes() {
    for scenario in [
        "anomaly_triage",
        "observation_selection",
        "behavior_selection",
        "completion_review",
    ] {
        let (request, choices) =
            crate::laya_workflows::prepare(&json!({"scenario":scenario,"state":{}})).unwrap();
        assert!(choices.contains("defer"));
        assert!(!choices.contains("despawn_entity"));
        assert_eq!(request["state"]["scenario"], scenario);
    }
    assert!(crate::laya_workflows::prepare(&json!({"scenario":"unknown","state":{}})).is_err());
    assert!(
        crate::laya_workflows::prepare(
            &json!({"scenario":"completion_review","state":{"text":"x".repeat(4097)}})
        )
        .is_err()
    );
}

#[tokio::test]
#[ignore = "Requires real Laya; fixed synthetic workflow baseline, never ECS writes"]
async fn laya_live_workflow_baseline() {
    let mut results = Vec::new();
    for line in include_str!("../../../integrations/laya/workflow_cases.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let input = json!({"scenario":case["scenario"],"state":case["state"]});
        let prediction = crate::laya_workflows::assess_at(&input, "http://127.0.0.1:3031")
            .await
            .unwrap();
        assert_eq!(prediction["executed"], false);
        assert_eq!(prediction["completion_verified"], false);
        assert_eq!(prediction["decision_kind"], "route_not_executable_tool");
        results.push(
            json!({"case_id":case["id"],"scenario":case["scenario"],"expected":case["expected"],
            "correct":prediction["operation"]==case["expected"],"prediction":prediction}),
        );
    }
    let correct = results.iter().filter(|r| r["correct"] == true).count();
    let report = json!({"dataset":"laya-workflows-smoke-v1","synthetic":true,"correct":correct,
        "total":results.len(),"training_eligible":false,"results":results});
    std::fs::create_dir_all("outputs/laya").unwrap();
    std::fs::write(
        "outputs/laya/workflow-baseline.json",
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "Workflow baseline: {correct}/{}; outputs/laya/workflow-baseline.json",
        report["total"]
    );
}
