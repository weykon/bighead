# Durable Business Activities

Editing is not permission to execute: `update_activity_plan` preserves Draft, Paused, or non-uncertain Blocked status. Pause an active activity BEFORE editing; a Running step cannot be edited. After review, explicitly start a Draft or resume a Paused/non-uncertain Blocked activity using the new revision. The runtime advances activities directly; they do not need a TickAction. Verification happens AFTER a step, so it cannot prevent that step's external effects. Never use a pending post-execution acceptance check as a pre-send authorization gate.

Do not promise that a human can pause in time between automatically advancing steps. For review before an external action, either keep that action in a separate Draft until authorized, or define a domain approval record and a pre-execution wait_for condition that remains false until an authorized decision updates it. The latter checks data, not who authorized the change: enforce authority on the writer separately. Never rely on a post-execution verify check to prevent sending.

Use activities for an authorized piece of work that needs multiple steps, waiting, recovery and evidence-based completion. A normal conversation does not need an activity. The user should describe their work in ordinary language; you translate it into domain components, systems and checks. Explain outcomes in the user's terms rather than listing runtime state names.

An activity is separate from a Spark. A Spark may finish while the activity waits. A tool returning success is not goal completion. Call inspect_activity to check status and evidence; never claim completion merely because you created or started an activity.

## Contract

1. Discover/define the domain's dynamic components and short Rhai systems first. Business types are NOT new Rust built-ins. Use stable business keys, not entity indexes in persisted plans or scripts.
2. create_activity takes id, scope, goal, steps and acceptance. It creates a draft. Same scope+id and identical plan returns the existing run; a different plan under that key is rejected. Definitions are pinned by source hash at creation.
3. Each step has id, system_id, optional args object, optional wait_for checks, and nonempty verify checks. The final acceptance checks must be nonempty and represent the user's goal.
4. A check is {"component":"DomainRecord","key_pointer":"/id","key":"stable-business-id","value_pointer":"/status","equals":"ready"}. Both field paths are JSON pointers. Exactly one matching entity must exist; missing or ambiguous matches fail. This proves ECS state at the observation time, not independently verified external reality.
5. control_activity({run_id,expected_revision,action:"start"}) starts the plan. The main loop advances at most one eligible step per poll, with a five-second per-run cadence. It does not call the model or create follow-up Sparks by itself; scripts can still use existing world APIs. Keep scripts short; they run under a 100,000-operation budget and bounded collection/string sizes.
6. Waiting conditions are business preconditions, NOT an authorization mechanism. Missing conditions cause waiting; they are checked again automatically. Pause stops progression, resume continues ready/paused/waiting or a non-uncertain blocker, cancel stops future work without undoing prior effects. Controls require the current revision from inspect_activity.
7. Execution intent is checkpointed before running, then actual results and independent read-back are checkpointed. If execution fails, verification fails, or the process restarts during execution, the activity blocks; it never silently repeats the step.
8. recheck reads an uncertain step's verification conditions without executing it. If they hold, that step is reconciled and progression may continue. This establishes current state, not exactly-once effects or who caused them. If they fail, investigate and repair the business state or cancel/replan explicitly. Do not start a new run to blindly duplicate an unknown external effect.
9. Changed/missing pinned code blocks execution. Do not edit the ActivityRun component directly. Inspect the blocker; restore the intended code or create a deliberately revised plan.
10. The agent may change the future workflow with `update_activity_plan` after `inspect_activity`: supply the current revision, keep the same business id/scope, and preserve completed step positions. Pause first when active. A revised plan is validated and its old plan is kept in `plan_history`; it does not rewrite prior records or repeat an uncertain effect. If the business identity itself changes, create a new activity.

## Minimal Two-Step Example

Updating a plan preserves Draft, Paused, or Blocked status. It never grants permission to start or resume. After review, use an explicit start for Draft or resume for Paused/non-uncertain Blocked with the NEW revision. In read-only contexts, inspect this guide through inspect_ecs_capabilities systems source; do not invoke run_system. The operations catalog lists only currently permitted tools, not the entire system's capabilities.

Define dynamic ActivityDemo {id:string, count:integer, approved:boolean}; create one entity with id="sample", count=0, approved=false. Define `activity_demo_increment`:

```rhai
for row in query("ActivityDemo") {
    let value = get_component(row._entity_id, "ActivityDemo");
    if value.id == args.id {
        value.count += 1;
        set_component(row._entity_id, "ActivityDemo", value);
    }
}
```

Create the activity with:

```json
{
  "id":"two-increments-v1", "scope":"demo:activity", "goal":"Increment twice, with approval before the second increment",
  "steps":[
    {"id":"first","system_id":"activity_demo_increment","args":{"id":"sample"},"verify":[{"component":"ActivityDemo","key_pointer":"/id","key":"sample","value_pointer":"/count","equals":1}]},
    {"id":"second","system_id":"activity_demo_increment","args":{"id":"sample"},"wait_for":[{"component":"ActivityDemo","key_pointer":"/id","key":"sample","value_pointer":"/approved","equals":true}],"verify":[{"component":"ActivityDemo","key_pointer":"/id","key":"sample","value_pointer":"/count","equals":2}]}
  ],
  "acceptance":[{"component":"ActivityDemo","key_pointer":"/id","key":"sample","value_pointer":"/count","equals":2}]
}
```

Start with the returned run_id and revision. Observe first step verified and second step waiting. Only when the caller authorizes the simulated approval, set approved=true on the business entity. Do not run the increment system manually. The activity runs the second step, reads it back, then separately verifies acceptance. Confirm records count=2, status=completed, and count=2. Repeat create_activity with the same plan: it must reuse the completed run and not increment again.

## First-Version Limits

Only registered Rhai execution and ECS equality checks are supported. No CLI/remote exactly-once guarantee, automatic rollback, compensation, general DAG, or independent long-task scheduling is implemented. Waiting can last across restart, but the main loop still waits for foreground model work. Each step uses full checkpoint barriers; measure cost before deploying many high-frequency activities. A runtime supports up to 256 retained runs, with at most 32 steps per plan. Checkpoint failure prevents new effects or blocks unknown outcomes. ActivityRun records are trusted runtime metadata, not a sandbox against arbitrary privileged Rhai scripts.
