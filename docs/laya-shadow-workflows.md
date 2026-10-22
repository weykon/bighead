# Laya shadow workflows

The automatic TickAction observer in this document collects evidence, not autonomous
authority. It cannot dispatch a
Spark, change a system, acknowledge an observer event, or mark a task complete.
Its suggestions do not change the existing steward's prompt or decisions. This
allows later comparison without contaminating the baseline with model advice.

This is separate from on-demand `laya_run_spec`. DecisionSpec v2 may dispatch one
predeclared bounded action when its call uses execute mode and its deterministic gates
pass. Execute is available by default. It does not grant authority
to this background observer.

## Running it

Keep the MLX service on `127.0.0.1:3031` and normally restart the rebuilt Big Head.
The application enables TickAction shadow observations before entering its main loop.
`BIGHEAD_LAYA_SHADOW=0` disables that observer. Libraries/tests must explicitly call
`enable_laya_shadow`; constructing AgentRuntime alone does not start network work.

Agent tools:

- `inspect_laya_shadow({})`: observer counters, pending count, evidence path and limits.
- `laya_assess({"scenario":"...","state":{...}})`: one of four fixed routing tasks.
- `laya_decide`: the previous exact-tool candidate interface remains available.

Read `run_system("guide_laya")` for state fields and route meanings. Route labels
like `repair_agent`, `invoke_existing` or `finish` are recommendations only, never
actual tool names. All recommendations retain executed/verified=false flags.

## Automatic anomaly observations

The runtime emits real TickAction execution reports. The observer:

1. Ignores routine healthy executions unless a failed incident needs a follow-up.
2. Opens an incident for a failure, recording stable source identity, correlation id,
   entity id, script hash, execution time and a bounded error preview. It does not
   copy source code, logs, shard text, or checkpoint content.
3. Coalesces repeats by stable source identity plus script/error fingerprint.
   Entity rotation does not open a duplicate incident. Error/script changes close
   the previous incident as unknown and open another. An incident older than 24h
   is closed/reopened on its next failed report.
4. Enqueues compact evidence without awaiting inference in the tick path. A worker
   asks Laya for anomaly routing advice when its budget permits.
5. Records the next execution separately, whether it fails or succeeds. A subsequent
   error-free execution adds execution_recovered. It does not assert business success,
   correct routing, or causal improvement from a suggestion that was never executed.

`pending_repair` is currently null because runtime reports alone do not prove whether
a matching investigation exists. Schema presence is not looked up speculatively.
Queueing a Spark is not evidence of a pending repair. Those unknowns are explicit
inputs; building a reliable repair-task correlation is a later enhancement.

The worker queue holds 32 records and tracking holds at most 128 incidents. Excess
records are dropped and counted; the tick loop does not wait. Inference has a two
second deadline, minimum five second interval, 300 second circuit break after service
failure and 256 attempts per runtime session. Skips are recorded, not retried in a
hot loop. The worker accepts no more than 16 MiB in one session evidence file; it
stops and exposes writer_stopped on reaching the cap or a write failure. The ordinary
service's prediction JSONL also remains outside checkpoints.

The tracker is a transient ECS resource, not a persistent component. No entities,
schemas, TickActions or systems are created for this observer. Session restart resets
tracking. Normal shutdown drains bounded work and records unresolved incident IDs;
hard exits, timeouts or queue drops can leave incomplete evidence.

## Other three workflows

They are available on demand through `laya_assess`; they are not background loops.

| Scenario | Required independent facts | Meaning of successful use |
| --- | --- | --- |
| observation_selection | Goal, required facts, observed freshness, available Lens/entity | Subsequent real read obtains missing facts; count unnecessary calls separately |
| behavior_selection | Event, known registered system/hash, preconditions, cooldown, arguments | Actual chosen system execution and component/view read-back meet the goal |
| completion_review | Every user obligation and its evidence, missing checks, cleanup state | Independent checks establish all obligations; the model's finish label is not proof |

The caller supplies observed facts. These tools do not magically inspect a full world
or prove that the supplied facts are true. Their calls use normal ActionRecord audit.
For actual behavior, the main agent must separately check preconditions, invoke the
existing tool/system, and verify output. No automatic write permission is added.

## Review and training boundary

Each observer session writes `outputs/laya/shadow-<uuid>/events.jsonl`. Source reports,
predictions and follow-up evidence join on `incident_id`; runtime identities survive
entity rotation. The header includes runtime session/world and evidence origin.
Unit/live-test demonstrations are marked `isolated_test`, never live production data.

```sh
.big-head/laya-venv/bin/python integrations/laya/review.py
```

This produces `outputs/laya/review-queue.jsonl`. The queue retains unknown expected
route, correctness, reviewer, rationale and business outcome; training_eligible is
always false. It also records missing session closure, drops and malformed partial
lines. It is a review queue, not a ready-to-train dataset. Do not edit regenerated
queue files as the only copy of a human review; retain reviewed labels separately.

Three distinct labels will be needed before training: appropriate decision for the
original observed state, actual execution success, and independently verified goal
completion. Recovery on a later tick proves only execution success at that time.
Split future training/evaluation by incident/source/task, not adjacent ticks. No
training, model adoption or confidence-threshold automation is implemented here.

## Validation

`integrations/laya/workflow_cases.jsonl` contains 12 fixed synthetic cases covering
all four workflows. Expected labels never enter the model request. Run the isolated
runtime demonstration and fixed workflow baseline with:

```sh
CARGO_INCREMENTAL=0 cargo test -p big-head-loop laya --lib -- --include-ignored --test-threads=1 --nocapture
.big-head/laya-venv/bin/python -m unittest discover -s integrations/laya -p 'test_*.py'
```

Cargo runs the tests from the loop crate, so its demonstrations and baseline are in
`crates/big-head-loop/outputs/laya/`. Review those explicitly with
`review.py --root crates/big-head-loop/outputs/laya --output outputs/laya/isolated-review.jsonl`.
They exercise real model calls and real isolated Rhai failure/recovery, never the
running production world. Tests cover duplicate coalescing, entity rotation, changed
errors, tracker and queue bounds, unavailable-model circuit breaking, and preserving
unknown correctness labels after recovery.

First measured workflow baseline (2026-09-21, MLX FP16): **4/12 correct**. The model
selected repair_agent for all three anomaly fixtures, reuse_observation for all
three observation fixtures, invoke_existing for all three behavior fixtures, and
cleanup for all three completion fixtures. This shows poor sensitivity to the
different states in these fixtures; the result does not qualify any route for
automatic execution. Keep the full failures for later domain-data design, and do
not reinterpret successful transport tests as decision-quality acceptance.

The isolated runtime demonstration recorded failure, a real MLX prediction, the
next successful execution and recovery, with no queued Spark or model-driven writes.
The ordinary loop/AI test suites passed 302 tests (13 ignored); 13 targeted Laya
checks including live calls passed, along with five Python service/review tests.
