use super::*;
use big_head_ai::ToolDefinition;

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    let check = json!({"type":"object","properties":{
        "component":{"type":"string"},"key_pointer":{"type":"string","description":"JSON pointer to stable business key, e.g. /id; never an entity index"},
        "key":{},"value_pointer":{"type":"string"},"equals":{}
    },"required":["component","key_pointer","key","value_pointer","equals"],"additionalProperties":false});
    vec![ToolDefinition {
        name:"create_activity".into(),
        description:"Create a durable business activity as draft. Read run_system('guide_activity') first. Not execution or completion. Define domain components/Rhai systems first. Plan and system hashes are pinned; scope+id deduplicates identical plans. Start with control_activity. Runtime independently reads checks; no LLM is called to advance steps. Use for authorized multi-step work, not every chat.".into(),
        input_schema:json!({"type":"object","properties":{
            "id":{"type":"string"},"scope":{"type":"string"},"goal":{"type":"string"},
            "deadline_epoch":{"type":["integer","null"]},
            "steps":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"object","properties":{
                "id":{"type":"string"},"system_id":{"type":"string"},"args":{"type":["object","null"]},
                "wait_for":{"type":"array","items":check.clone()},"verify":{"type":"array","minItems":1,"items":check.clone()}
            },"required":["id","system_id","verify"],"additionalProperties":false}},
            "acceptance":{"type":"array","minItems":1,"items":check}
        },"required":["id","scope","goal","steps","acceptance"],"additionalProperties":false}),
    }, ToolDefinition {
        name:"update_activity_plan".into(),
        description:"Revise a durable activity's future workflow. Read inspect_activity first and provide expected_revision. Only Draft, Paused, or a non-uncertain Blocked run can be revised; a Running or uncertain run must be rechecked or cancelled. Completed step ids and their position cannot be rewritten. New plan is validated and system source hashes are repinned; old plan is preserved in plan_history. This changes future workflow, never past evidence or external effects.".into(),
        input_schema:json!({"type":"object","properties":{"run_id":{"type":"string"},"expected_revision":{"type":"integer"},"plan":{"type":"object","properties":{"id":{"type":"string"},"scope":{"type":"string"},"goal":{"type":"string"},"deadline_epoch":{"type":["integer","null"]},"steps":{"type":"array","minItems":1,"maxItems":32},"acceptance":{"type":"array","minItems":1}},"required":["id","scope","goal","steps","acceptance"],"additionalProperties":true}},"required":["run_id","expected_revision","plan"],"additionalProperties":false}),
    }, ToolDefinition {
        name:"control_activity".into(),
        description:"Control durable work using run_id and expected_revision from inspect_activity: start, pause, resume, cancel, recheck. Completed requires runtime evidence; cannot force success. recheck reads an uncertain step without re-executing it. cancel does not undo effects. Only local authorized callers may control activities.".into(),
        input_schema:json!({"type":"object","properties":{"run_id":{"type":"string"},"expected_revision":{"type":"integer"},"action":{"type":"string","enum":["start","pause","resume","cancel","recheck"]}},"required":["run_id","expected_revision","action"],"additionalProperties":false}),
    }, ToolDefinition {
        name:"inspect_activity".into(),description:"Inspect one durable activity including plan, steps, evidence and revision, or list activity summaries optionally by scope. Spark completion is not activity completion.".into(),
        input_schema:json!({"type":"object","properties":{"run_id":{"type":"string"},"scope":{"type":"string"}},"additionalProperties":false}),
    }]
}

pub(crate) fn dispatch(
    world: &mut World,
    registry: &ComponentActionRegistry,
    systems: &SystemRegistry,
    name: &str,
    input: &Value,
) -> Result<Value, String> {
    let now = crate::audit::now_secs();
    match name {
        "create_activity" => {
            let plan: Plan = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
            if let Some(run) = world
                .query::<&ActivityRun>()
                .iter(world)
                .find(|r| r.plan.id == plan.id && r.plan.scope == plan.scope)
            {
                return if run.plan == plan {
                    Ok(json!({"reused":true,"activity":run}))
                } else {
                    Err("scope+id already exists with another plan; inspect or use a new id".into())
                };
            }
            if world.query::<&ActivityRun>().iter(world).count() >= 256 {
                return Err("activity v1 capacity is 256 retained runs".into());
            }
            let hashes = validate(&plan, systems)?;
            let run = ActivityRun {
                run_id: uuid::Uuid::new_v4().to_string(),
                plan,
                plan_history: vec![],
                system_hashes: hashes,
                status: Status::Draft,
                revision: 1,
                cursor: 0,
                uncertain_step: false,
                created_at: now,
                updated_at: now,
                next_poll_epoch: 0,
                reason: "awaiting start".into(),
                records: vec![],
                evidence: vec![],
            };
            let output = json!({"reused":false,"activity":run});
            world.spawn(run);
            Ok(output)
        }
        "update_activity_plan" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Update {
                run_id: String,
                expected_revision: u64,
                plan: Plan,
            }
            let request: Update =
                serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
            let entity = find(world, &request.run_id)?;
            let mut run = world.get::<ActivityRun>(entity).unwrap().clone();
            if run.revision != request.expected_revision {
                return Err(format!(
                    "activity revision conflict: current={}",
                    run.revision
                ));
            }
            if run.uncertain_step || run.status == Status::Running {
                return Err(
                    "cannot revise a running or uncertain activity; recheck or cancel first".into(),
                );
            }
            if !matches!(run.status, Status::Draft | Status::Paused | Status::Blocked) {
                return Err(format!(
                    "plan revision is only allowed in Draft, Paused or non-uncertain Blocked; current={:?}",
                    run.status
                ));
            }
            if request.plan.id != run.plan.id || request.plan.scope != run.plan.scope {
                return Err("activity id and scope are immutable; create a new activity for another business identity".into());
            }
            if request.plan.steps.len() < run.cursor {
                return Err("revised plan cannot remove already completed step positions".into());
            }
            for index in 0..run.cursor {
                if request.plan.steps[index] != run.plan.steps[index] {
                    return Err(format!(
                        "completed step at position {index} cannot be rewritten or reordered"
                    ));
                }
            }
            let hashes = validate(&request.plan, systems)?;
            run.plan_history.push(run.plan.clone());
            run.plan = request.plan;
            run.system_hashes = hashes;
            run.evidence.clear();
            run.next_poll_epoch = 0;
            run.change(
                run.status.clone(),
                now,
                "future workflow revised by agent; prior records preserved",
            );
            let output = json!({"updated":true,"activity":run});
            world.entity_mut(entity).insert(run);
            Ok(output)
        }
        "inspect_activity" => {
            if let Some(id) = input.get("run_id").and_then(Value::as_str) {
                let entity = find(world, id)?;
                Ok(json!(world.get::<ActivityRun>(entity).unwrap()))
            } else {
                let scope = input.get("scope").and_then(Value::as_str);
                let mut runs: Vec<_> = world
                    .query::<&ActivityRun>()
                    .iter(world)
                    .filter(|r| scope.is_none_or(|s| r.plan.scope == s))
                    .collect();
                runs.sort_by_key(|r| (r.created_at, &r.run_id));
                Ok(json!(runs.iter().map(|r|json!({"run_id":r.run_id,"id":r.plan.id,"scope":r.plan.scope,
                    "goal":r.plan.goal,"status":r.status,"revision":r.revision,"cursor":r.cursor,"steps":r.plan.steps.len(),
                    "reason":r.reason,"uncertain_step":r.uncertain_step})).collect::<Vec<_>>()))
            }
        }
        "control_activity" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Control {
                run_id: String,
                expected_revision: u64,
                action: String,
            }
            let request: Control =
                serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
            let entity = find(world, &request.run_id)?;
            let mut run = world.get::<ActivityRun>(entity).unwrap().clone();
            if run.revision != request.expected_revision {
                return Err(format!(
                    "activity revision conflict: current={}",
                    run.revision
                ));
            }
            match request.action.as_str() {
                "start" if run.status == Status::Draft => run.change(Status::Ready, now, "started"),
                "pause" if matches!(run.status, Status::Ready | Status::Waiting) => {
                    run.change(Status::Paused, now, "paused")
                }
                "resume"
                    if matches!(
                        run.status,
                        Status::Paused | Status::Waiting | Status::Blocked
                    ) && !run.uncertain_step =>
                {
                    run.change(Status::Ready, now, "resumed")
                }
                "cancel"
                    if !matches!(
                        run.status,
                        Status::Completed | Status::Cancelled | Status::Running
                    ) =>
                {
                    run.change(
                        Status::Cancelled,
                        now,
                        "cancelled; prior effects are not undone",
                    )
                }
                "recheck" if run.status == Status::Blocked => {
                    if run
                        .plan
                        .deadline_epoch
                        .is_some_and(|deadline| now >= deadline)
                    {
                        return Err(
                            "activity deadline exceeded; inspect/cancel and explicitly replan"
                                .into(),
                        );
                    }
                    if run.uncertain_step {
                        let step = &run.plan.steps[run.cursor];
                        let evidence = read_checks(world, registry, &step.verify, now);
                        let passed = all_pass(&evidence);
                        if let Some(record) = run.records.last_mut() {
                            record.evidence = evidence.clone();
                            if passed {
                                record.outcome = "reconciled_by_readback_not_replayed".into();
                                record.finished_at = Some(now);
                            }
                        }
                        run.evidence = evidence;
                        if passed {
                            run.uncertain_step = false;
                            run.cursor += 1;
                            run.change(
                                Status::Ready,
                                now,
                                "step reconciled from current evidence; no execution repeated",
                            );
                        } else {
                            run.change(
                                Status::Blocked,
                                now,
                                "recheck failed; unknown effects still require investigation",
                            );
                        }
                    } else if run.cursor == run.plan.steps.len() {
                        run.evidence = read_checks(world, registry, &run.plan.acceptance, now);
                        if all_pass(&run.evidence) {
                            run.change(Status::Completed, now, "goal independently rechecked");
                        } else {
                            run.change(Status::Blocked, now, "goal acceptance still fails");
                        }
                    } else {
                        return Err(
                            "no uncertain step to reconcile; inspect blocker before resume".into(),
                        );
                    }
                }
                _ => {
                    return Err(format!(
                        "action '{}' not valid in {:?}; uncertain_step={}",
                        request.action, run.status, run.uncertain_step
                    ));
                }
            }
            run.next_poll_epoch = 0;
            let output = json!(run);
            world.entity_mut(entity).insert(run);
            Ok(output)
        }
        _ => Err("unknown activity tool".into()),
    }
}
