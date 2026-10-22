# Laya DecisionSpec v2

DecisionSpec v2 is the executable contract between observed ECS state, the local
Laya reflex model and Big Head's audited tool runtime. The model supplies typed
judgments. Rust owns policy, authority, execution and verification.

```text
observe -> parallel choice/score/noul -> deterministic gates
        -> bounded action -> independent read-back -> outcome record
```

Use `laya_run_spec` when the same kind of local decision will recur and its safe
actions and acceptance checks can be stated before inference. Use ordinary agent
reasoning for open-ended planning, new schema/system design, code generation and
decisions whose success cannot be read back.

## Modes and authority

- `shadow` evaluates policy and records an outcome without dispatching the action.
- `execute` is available by default and needs no process environment switch.
- Execute specs require an action-choice confidence gate of at least `0.5`, all
  declared gates to pass, a currently available bounded action, and at least one
  read-only verification.
- Laya cannot define or remove components/systems, invoke CLI or network tools,
  recursively invoke Laya, or bypass the pipeline's capability boundary.
- Model confidence is not calibrated for ECS. The threshold is a policy input, not
  proof. Use shadow when you want to measure a new spec before allowing mutation.

The current action allowlist is `spawn_entity`, `modify_component`,
`remove_component`, `despawn_entity`, `run_system`, `record_work`, `arrange_work`,
the read tools, and already registered Rhai tool systems. A name must also exist in
the current tool catalog. Dynamic definitions remain an ordinary agent task.

## Contract

```json
{
  "mode": "shadow",
  "spec": {
    "version": 2,
    "id": "work_item.advance.v1",
    "goal": "Advance a ready work item to active",
    "state": {
      "entity_id": 42,
      "status": "ready",
      "blockers": 0
    },
    "questions": {
      "action": {
        "type": "choice",
        "instructions": "Choose the next bounded operation",
        "criteria": {
          "activate": "set the existing item status to active",
          "defer": "leave the item unchanged"
        }
      },
      "ready": {
        "type": "noul",
        "instructions": "Is the observed evidence sufficient to activate it?"
      },
      "risk": {
        "type": "score",
        "instructions": "How risky is this state transition?",
        "criteria": ["low", "medium", "high"]
      }
    },
    "policy": {
      "action_question": "action",
      "defer_choices": ["defer"],
      "gates": [
        {"question": "action", "metric": "confidence", "operator": "gte", "value": 0.8},
        {"question": "ready", "metric": "noul", "operator": "gte", "value": 0.9},
        {"question": "risk", "metric": "score", "operator": "lte", "value": 1.0}
      ],
      "on_gate_failure": "escalate"
    },
    "actions": {
      "activate": {
        "tool": "modify_component",
        "description": "Set the observed WorkItem status to active",
        "input": {
          "entity_id": 42,
          "component": "WorkItem",
          "value": {"status": "active"}
        },
        "verify": [
          {
            "id": "status",
            "tool": "get_component",
            "input": {"entity_id": 42, "component": "WorkItem"},
            "expect": {
              "kind": "json_pointer_equals",
              "pointer": "/status",
              "value": "active"
            }
          }
        ]
      }
    }
  }
}
```

Action choice labels must exactly equal `actions` plus `defer_choices`. Specs are at
most 32 KiB; selected state is at most 8 KiB. There may be at most 16 questions,
16 actions and 16 verification checks per action. IDs are stable ASCII identifiers.

## Questions and gates

- `choice`: criteria is a map from stable label to short meaning. Gate either its
  answer `confidence` or one option's `choice_probability`.
- `score`: criteria is an ordered list of 2..8 levels. Gate the interpolated `score`.
- `noul`: a binary probability in `[0,1]`. Optional criteria may name `false` and
  `true`. Gate the returned `noul` value.
- Operators are `gte` and `lte`. Gate evaluation is deterministic Rust code.
- Selecting a defer label always yields `defer`. A failed gate yields the declared
  `defer` or `escalate`. Passing gates in shadow yields `shadow`; only qualified
  execute mode yields `act`.

## Verification

Verification tools are read-only: `get_entity`, `get_component`, `query_entities`,
`inspect_scene`, `inspect_work` and `inspect_world_shape`.

Expectation kinds:

- `success`: the read completed without a tool error.
- `output_contains` / `output_not_contains`: bounded text assertion.
- `json_pointer_equals`: parsed JSON at `pointer` equals `value`.
- `json_pointer_exists`: parsed JSON contains `pointer`.

`get_component` output such as `WorkItem: {"status":"active"}` is parsed before
JSON Pointer evaluation. An empty pointer addresses a primitive root value. Prefer
semantic JSON checks over text checks. If the action fails, verification is not run.
If any read-back fails, `behavior_verified=false` and `goal_achieved=null` even when
the write tool itself returned success.

## Evidence and training boundary

The result contains model answers, gate observations, execution output, every
verification result, the computed outcome, and the `/outcome` acknowledgement.
Nested action and verification calls are written as normal `ActionRecord` entries in
the enclosing transaction. Service JSONL records join prediction and outcome by
`decision_id`, `spec_id` and the canonical `spec_hash`.

Runtime read-back establishes what happened; it does not establish that Laya chose
the best action. Outcomes remain `training_eligible:false`. A reviewed training row
still needs an independent expected decision, reviewer/provenance, and task-level
success label. Split later datasets by task/session/spec lineage rather than adjacent
calls, and preserve failures, deferrals and escalations.
