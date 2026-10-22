# World-owned operation loop

Domain facts and behavior belong to the live ECS world, not to Rust component structs.
Rust owns only mechanisms: dynamic CRUD, Rhai execution, capability checks, structured
queries, audit retention and loop safety. Install the following pieces only when a world
actually needs them; do not create them for ordinary conversation.

## Runtime ownership

Use `define_component` for these world-owned dynamic components:

```text
WeatherObservation {
  observation_id:string, location:string, observed_at:string,
  observed_at_epoch:integer, temperature:string, condition:string,
  humidity:string, source:string
}

WorldClock {
  current_epoch:integer, timezone_offset_secs:integer,
  time_of_day:string, light:string, updated_at:string
}

ContextWeatherLink { observation_id:string }

OperationPlan {
  plan_id:string, goal:string, status:string,
  obligations_json:string, completed_json:string,
  blocked_reason:string, created_at:integer, updated_at:integer
}

AcceptanceCheck {
  check_id:string, plan_id:string, description:string,
  read_tool:string, input_json:string, expectation_json:string,
  status:string, evidence_action_id:string, checked_at:integer
}
```

These are examples of a reusable composition, not reserved component names. Inspect the
world first. Reuse a compatible existing component; do not redefine or fork it merely
because the name is already present. `SessionContext` is a legacy dynamic component whose
schema is currently bootstrapped by the runtime. Do not add weather identity to that schema
in Rust. Attach `ContextWeatherLink` to the same entity instead.

## Rhai owns derived state

Define a user Rhai system such as `world_clock_refresh` to update one WorldClock singleton.
The script derives `time_of_day` and `light` from `timestamp()` and an explicit timezone
offset. A TickAction may run that script at a sparse cadence. It must update the singleton,
not spawn one entity per tick, and it must not call the LLM when no semantic event occurred.

Define a user Rhai tool such as `record_weather_observation` to:

1. validate its explicit arguments;
2. spawn one immutable WeatherObservation;
3. attach or update ContextWeatherLink on the current SessionContext entity;
4. project a short compatibility summary into legacy `SessionContext.weather` if current UI
   or Lens code still consumes it;
5. read WorldClock for time/light rather than asking the model to calculate them;
6. return the observation entity/id for independent read-back.

Network fetch remains outside this Rhai tool. The Agent fetches data, then invokes the
bounded tool with observed values. Never parse a prose weather blob back into facts.

## Plans and acceptance checks

For a multi-step mutation, create one OperationPlan plus one AcceptanceCheck per requested
obligation. Keep `obligations_json` and `completed_json` as bounded JSON arrays encoded in
strings until nested dynamic schemas exist. Valid states are:

```text
OperationPlan: planned | running | blocked | completed | abandoned
AcceptanceCheck: pending | passed | failed | blocked
```

After every write, execute the declared read-only check and store its ActionRecord id in
`evidence_action_id`. A successful write is not a passed check. A plan becomes completed
only when every check passed and cleanup obligations passed. If no fact or state changed in
two attempts, mark the plan blocked or change strategy; do not repeat the same call.

For field filtering, prefer `query_ecs` with structured component/path/operator/value input.
Use `query_entities` only for compact composition queries that do not require guessing field
syntax.

## Laya decision gates

Laya does not own schemas or actions. Use the fixed `laya_assess` scenarios at explicit
world transitions:

- `anomaly_triage`: a new or changed failure needs routing;
- `behavior_selection`: more than one existing verified behavior is plausible;
- `completion_review`: checks or cleanup may remain.

Use `laya_run_spec` only when state, bounded existing actions and read-back checks can all be
declared before inference. Keep open-ended planning and new Rhai design in the main Agent.

## Sparse scheduling

Keep deterministic WorldClock maintenance, real sensor refresh and anomaly stewardship.
Do not revive dormant awareness/heartbeat loops merely to show activity. A periodic system
must update bounded singleton state or emit a Spark only for a meaningful change. Routine
healthy ticks stay local and do not call Laya or the LLM.

## Audit persistence

The external `outputs/action_log.jsonl` is the complete append-only history. ECS/checkpoint
keeps recent ActionRecord detail and longer-lived TransactionRecord summaries. Defaults are
7 and 90 days. `BIGHEAD_ACTION_RETENTION_DAYS=0` or
`BIGHEAD_TRANSACTION_RETENTION_DAYS=0` disables the corresponding pruning. Restored `open`
transactions are marked `abandoned_after_restart`; inspect the JSONL before retrying partial
effects.
