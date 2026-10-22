use super::*;
use crate::{AgentRuntime, audit};
use std::path::Path;

/// One step per poll, with durable intent before effects. Unknown outcomes never auto-replay.
pub(super) fn advance(
    rt: &mut AgentRuntime,
    checkpoint: &Path,
    now: u64,
) -> Result<Option<Value>, String> {
    let poll_started = std::time::Instant::now();
    if rt.is_cancelled() || rt.pending_spark_count() > 0 {
        return Ok(None);
    }
    let candidate = rt
        .world
        .query::<(Entity, &ActivityRun)>()
        .iter(&rt.world)
        .filter(|(_, r)| {
            matches!(r.status, Status::Ready | Status::Waiting) && r.next_poll_epoch <= now
        })
        .min_by_key(|(_, r)| (r.next_poll_epoch, r.created_at, &r.run_id))
        .map(|(e, r)| (e, r.clone()));
    let Some((entity, mut run)) = candidate else {
        return Ok(None);
    };
    run.next_poll_epoch = now.saturating_add(5);
    if run
        .plan
        .deadline_epoch
        .is_some_and(|deadline| now >= deadline)
    {
        run.change(
            Status::Blocked,
            now,
            "activity deadline exceeded; no further execution",
        );
        return persist(rt, checkpoint, entity, run).map(Some);
    }
    if run.cursor == run.plan.steps.len() {
        run.evidence = read_checks(&rt.world, &rt.component_registry, &run.plan.acceptance, now);
        if all_pass(&run.evidence) {
            run.change(
                Status::Completed,
                now,
                "all steps and goal independently checked",
            );
        } else {
            run.change(
                Status::Blocked,
                now,
                "steps finished but goal acceptance failed",
            );
        }
        return persist(rt, checkpoint, entity, run).map(Some);
    }
    let step = run.plan.steps[run.cursor].clone();
    let hash = system_hash(&rt.system_registry, &step.system_id);
    if hash.as_ref().ok() != Some(&run.system_hashes[run.cursor]) {
        run.change(
            Status::Blocked,
            now,
            "registered system missing or changed from pinned code; no execution",
        );
        return persist(rt, checkpoint, entity, run).map(Some);
    }
    run.evidence = read_checks(&rt.world, &rt.component_registry, &step.wait_for, now);
    if !all_pass(&run.evidence) {
        if run.status == Status::Waiting {
            rt.world.entity_mut(entity).insert(run);
            return Ok(None);
        }
        run.change(
            Status::Waiting,
            now,
            "waiting for business preconditions; no model or action invoked",
        );
        return persist(rt, checkpoint, entity, run).map(Some);
    }
    let before = run.clone();
    let tx = audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        &format!("activity:{}:{}", run.run_id, step.id),
        &[],
        rt.agent_entity,
        Some(rt.session_id.clone()),
    );
    run.change(Status::Running, now, "intent persisted before execution");
    run.uncertain_step = true;
    run.records.push(StepRecord {
        step_id: step.id.clone(),
        tx_id: tx.tx_id.clone(),
        effect_key: format!("{}:{}", run.run_id, step.id),
        system_hash: run.system_hashes[run.cursor].clone(),
        started_at: now,
        finished_at: None,
        outcome: "running".into(),
        output: String::new(),
        evidence: vec![],
    });
    if let Err(error) = persist(rt, checkpoint, entity, run.clone()) {
        let mut stopped = before;
        stopped.change(
            Status::Blocked,
            now,
            format!("pre-execution checkpoint failed; no effects: {error}"),
        );
        rt.world.entity_mut(entity).insert(stopped);
        audit::fail_transaction(&mut rt.world, &tx, None, error.clone());
        return Err(error);
    }
    let execution_at = now.saturating_add(poll_started.elapsed().as_secs());
    if rt.is_cancelled()
        || run
            .plan
            .deadline_epoch
            .is_some_and(|deadline| execution_at >= deadline)
    {
        run.records.pop();
        run.uncertain_step = false;
        run.change(
            if rt.is_cancelled() {
                Status::Paused
            } else {
                Status::Blocked
            },
            execution_at,
            "cancelled or deadline exceeded after checkpoint; no action executed",
        );
        audit::fail_transaction(&mut rt.world, &tx, None, run.reason.clone());
        return persist(rt, checkpoint, entity, run).map(Some);
    }
    let call = big_head_ai::ToolCall {
        id: format!("{}:execute", tx.tx_id),
        name: "run_system".into(),
        input: json!({"id":step.system_id,"args":if step.args.is_null(){json!({})}else{step.args.clone()}}),
    };
    let engine = crate::scripting::bounded_ecs_engine();
    let previous_context = rt
        .world
        .remove_resource::<big_head_ecs::execution::DispatchContext>();
    rt.world
        .insert_resource(big_head_ecs::execution::DispatchContext {
            source_id: format!("{}:{}", run.run_id, step.id),
            correlation_id: tx.tx_id.clone(),
        });
    let (output, failed, _) = crate::tools::run_system::dispatch_run_system(
        &mut rt.system_registry,
        &engine,
        &mut rt.world,
        &mut rt.component_registry,
        Some(&mut rt.spark_queue),
        rt.log_emitter.clone(),
        &call,
    );
    rt.world
        .remove_resource::<big_head_ecs::execution::DispatchContext>();
    if let Some(context) = previous_context {
        rt.world.insert_resource(context);
    }
    let finished_at = now.saturating_add(poll_started.elapsed().as_secs());
    let evidence = read_checks(&rt.world, &rt.component_registry, &step.verify, finished_at);
    let verified = !failed && all_pass(&evidence);
    let record = run.records.last_mut().unwrap();
    record.finished_at = Some(finished_at);
    record.output = crate::experience::excerpt(&output, 2400);
    record.evidence = evidence.clone();
    record.outcome = if verified {
        "verified"
    } else if failed {
        "execution_failed_effects_unknown"
    } else {
        "verification_failed"
    }
    .into();
    run.evidence = evidence;
    if verified {
        run.uncertain_step = false;
        run.cursor += 1;
        run.change(
            Status::Ready,
            now,
            "step independently verified; next poll advances activity",
        );
        audit::commit_transaction(
            &mut rt.world,
            &tx,
            Some(1),
            format!("activity {} step {} verified", run.run_id, step.id),
        );
    } else {
        run.change(
            Status::Blocked,
            now,
            "step failed or read-back mismatched; inspect/recheck, never auto-replay",
        );
        audit::fail_transaction(&mut rt.world, &tx, Some(1), run.reason.clone());
    }
    let output =
        json!({"run_id":run.run_id,"status":run.status,"cursor":run.cursor,"reason":run.reason});
    if let Err(error) = persist(rt, checkpoint, entity, run.clone()) {
        run.cursor = before.cursor;
        run.uncertain_step = true;
        run.change(
            Status::Blocked,
            now,
            format!("post-execution checkpoint failed; effects may exist: {error}"),
        );
        if let Ok(mut entity) = rt.world.get_entity_mut(entity) {
            entity.insert(run);
        }
        return Err(error);
    }
    Ok(Some(output))
}

fn persist(
    rt: &mut AgentRuntime,
    path: &Path,
    entity: Entity,
    run: ActivityRun,
) -> Result<Value, String> {
    let output =
        json!({"run_id":run.run_id,"status":run.status,"cursor":run.cursor,"reason":run.reason});
    rt.world
        .get_entity_mut(entity)
        .map_err(|_| "activity entity was removed during execution")?
        .insert(run);
    rt.checkpoint(path)?;
    Ok(output)
}

impl AgentRuntime {
    /// Explicit path permits deterministic isolated tests and shares the world's checkpoint.
    pub fn advance_activity(&mut self, checkpoint: &Path) -> Result<Option<Value>, String> {
        advance(self, checkpoint, audit::now_secs())
    }
}
