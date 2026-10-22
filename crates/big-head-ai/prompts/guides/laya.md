# Local Laya decisions

## DecisionSpec v2: closed-loop decisions

Use `laya_run_spec` for a recurring decision whose observed inputs, bounded existing
actions and read-back acceptance checks can all be declared before inference. It runs:

```text
compact observed state -> parallel choice/score/noul -> Rust policy gates
-> bounded existing action -> independent read-back -> recorded outcome
```

`mode:"execute"` is available by default. It acts only when the selected tool is
currently available and allowlisted, an action confidence gate is `gte >= 0.5`, all
gates pass, and the action declares at least one read-only verification. Use
`mode:"shadow"` when observation without mutation is useful; shadow never dispatches
its selected action. A failed gate returns `defer` or `escalate`; it is not a tool error.

Observe exact entity ids, values and registered tool/system names first. Never ask
Laya to invent state or arguments. Choice labels must exactly match `actions` plus
`defer_choices`. Available gate metrics are `confidence`, `choice_probability`,
`score` and `noul`, with `gte`/`lte`. Verification uses `get_entity`, `get_component`,
`query_entities`, `query_ecs`, `inspect_scene`, `inspect_work` or `inspect_world_shape`, with
`success`, `output_contains`, `output_not_contains`, `json_pointer_equals` or
`json_pointer_exists` expectations.

Read the returned `policy.verdict`, `execution`, `verification`, `outcome` and
`outcome_record`. A successful write with failed read-back is not completion:
`behavior_verified=false`, `goal_achieved=null`. Laya cannot define/remove
components or systems, call CLI/network tools, recursively call itself, or bypass the
active pipeline capability set. Keep open-ended planning and new Rhai/schema design
in the main agent. The complete contract and example are in
`docs/laya-decision-spec-v2.md`.

## Single advisory choice

`laya_decide` recommends an operation; it never executes it or generates tool arguments.
The local multilingual base checkpoint has not been trained or calibrated on our ECS tasks.
High confidence is not evidence of correctness. Keep complex planning and Rhai generation
in the main agent. Do not replace deterministic runtime checks with model guesses.

1. Inspect the actual operation catalog and read relevant entities/schemas/systems.
2. Supply a short goal, a compact observed `state` object, and 1..7 candidates, each
   `{ "operation": "exact_available_tool_name", "description": "short intended effect" }`.
   The service adds `defer`. Never paste a checkpoint, shard body or full tool catalog.
3. Read the recommendation and probabilities. `executed:false` and
   `behavior_verified:false` remain false even for a high-confidence answer.
4. If acting within the user's task, check preconditions and call the actual ECS tool
   separately. Read back the resulting state. Report each requested acceptance step,
   including failures and unexecuted steps; transaction completion is not task completion.

The model's token budget is smaller than the JSON byte limit. Oversized state/options
are rejected rather than truncated. Shorten the input or split the choice into stages.
Unavailable/busy service is a local decision-service error, not a MiniMax API failure.
Use the existing agent/tools to continue if local inference is unavailable.

The operator starts `.big-head/laya-venv/bin/python integrations/laya/service.py`.
Default address: `http://127.0.0.1:3031`; override runtime with `BIGHEAD_LAYA_URL`.
This is separate from the web view on 3030. Model loads once per service process.
Predictions are recorded in `outputs/laya/decisions-YYYY-MM-DD.jsonl`, outside checkpoints.
Those records have `training_eligible:false`. DecisionSpec v2 appends separately linked
runtime outcomes, but those outcomes do not prove the model chose the best action. Do
not relabel predictions as correct decisions. Fixed evaluation cases are synthetic baseline data,
not evidence that arbitrary live-world tasks work.

## Narrow workflow advice

Prefer `laya_assess({"scenario": "...", "state": {...}})` for the following decisions.
It returns fixed route labels, not executable tools. It cannot run a system, create a
Spark, acknowledge an event or mark a task complete. Use `inspect_laya_shadow` to read
automatic observer health and counters without making an inference request.

| Scenario | State evidence to supply | Route meanings |
| --- | --- | --- |
| anomaly_triage | error, execution_succeeded, consecutive_failures_observed, pending_repair (true/false/null), schema check status | observe / repair_agent / merge_pending / wait / defer |
| observation_selection | goal, required facts, existing observation completeness/freshness, matching Lens id, known entity id | reuse_observation / refresh_lens / read_entity / main_agent / defer |
| behavior_selection | event, actual existing system id/hash, verified preconditions, cooldown readiness, unresolved parameter needs | invoke_existing / wait / main_agent / defer |
| completion_review | every requested obligation, evidence reference and status; read-back results; cleanup obligations | verify / cleanup / handoff / finish / defer |

Use null for unknown facts. Never infer a pending repair from the number of queued
Sparks, or assume a registered system is ready to run. Long state is rejected.
Check selected behavior IDs with inspect_ecs_capabilities; Laya only selects a route.
`finish` is a recommendation, never evidence of completion. Independently check every
obligation before reporting completion. Deterministic rules remain runtime code/Rhai.

The normal application observes TickAction errors in shadow mode by default;
`BIGHEAD_LAYA_SHADOW=0` disables it. Errors with the same source identity, script and
error hash coalesce even when entity IDs change. Healthy routine ticks do not call
the model. New/changed incidents may get a recommendation; follow-up execution and
recovery are separate evidence records. Pending repair state is currently unknown,
not guessed. The steward and dispatch pipeline are not changed by these suggestions.

Limits: queue 32, tracked incidents 128, inference timeout 2 seconds, minimum interval
5 seconds, backoff 300 seconds after failure, at most 256 attempts/session. Evidence
is in outputs/laya/shadow-<uuid>/events.jsonl (16 MiB/session cap, no checkpoint data).
Queue drops, capped or unavailable inference remain visible. Restart starts a new
observation session; pending tracking is not persisted across restarts. Exit leaves
unresolved outcomes unknown. `execution_recovered` only means a later tick returned
without error, not that Laya caused recovery or business goals were achieved.

Run `python integrations/laya/review.py` to generate a review queue joining each
incident with its prediction and subsequent execution. All expected routes, correctness
and business outcome labels remain null until independently reviewed. Do not train on
model self-labels or treat later recovery as proof that its earlier advice was correct.
