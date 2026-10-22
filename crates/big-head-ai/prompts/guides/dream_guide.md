# Dream-RSI practice

This is a small implementation of history-based exploration-policy improvement,
inspired by https://arxiv.org/abs/2609.14858. The domain is pure Rhai programs
over JSON test inputs. It does not train model weights or deploy production skills.
No background worker is scheduled. Work only on an explicitly requested practice
task, and finish within the requested budget. Never push Sparks to continue practice.

## Online discovery

1. `dream_task({action:"inspect"})` finds existing experiments. Reuse an appropriate
   experiment; task definitions and test cases are immutable after creation.
2. Create, for example:
   `dream_task({action:"create",id:"positive-sum",task:{description:"Sum strictly positive integers in input",cases_json:"[{\"input\":[-2,3,0,4],\"expected\":7},{\"input\":[],\"expected\":0},{\"input\":[-3,-1],\"expected\":0}]",budget:6,workers:1}})`.
   cases_json is serialized JSON text so arbitrary JSON types survive provider
   tool transports. Numbers stay numbers, strings stay strings, and [] stays an
   empty array. Inspect task_spec to verify the stored cases before practice.
   Choose expectations from the task specification, not from your candidate's output.
   Passing these visible cases is not evidence of correctness on unseen inputs.
3. `dream_task({action:"next",id:"positive-sum"})` starts/resumes a round. The
   deployed policy selects a batch of parents. Parent 0 means a fresh attempt;
   otherwise refine the returned parent script using its actual diagnostics.
4. Submit one Rhai expression per parent, in order:
   `dream_task({action:"submit",id:"positive-sum",batch_id:"<returned ID>",scripts:["let total = 0; for n in input { if n > 0 { total += n; } } total"]})`.
   Each expression receives `input` and its final value is compared to `expected`.
   Pure Rhai only: no ECS helpers, CLI, import, eval, filesystem or network.
   Runtime records failures and fraction passed; never invent or submit scores.
5. Repeat next/submit within the fixed attempt budget. The budget automatically
   seals a round. To seal early, `finish` requires the latest `expected_revision`.
   An empty round cannot become replay evidence. `next` retries reuse the pending
   batch; an immediate identical `submit` retry reuses its receipt.

At most 4 experiments, 8 completed trees each, 32 attempts/tree, 4 workers and 16
cases. Scripts <=8 KiB, case data <=16 KiB, persisted experiment <=1 MiB. Each
script execution is operation/size/recursion bounded. Batches execute locally in
sequence in v1: batching is represented in the policy but is NOT wall-time parallelism.
The initial policy refines a branch three times, then opens another branch.

## Offline dreaming

Inspect a completed tree with `dream_task({action:"inspect",id:"positive-sum",round:1})`.
Using its actual failures and rewards, propose executable exploration policies:

```rhai
if obs.best >= 1.0 { [] } else { [0] }
```

This example keeps opening fresh branches and stops on a fully passing candidate.
It is not universally better. Another policy may refine a promising branch.
The expression receives only `obs`:

- `nodes`: revealed `{id,parent,score,passed}` observations, no hidden outcomes.
- `available`: eligible parent IDs. 0 opens a branch; other IDs must be leaves.
- `best`, `batches`, `remaining` (attempt budget), `workers` (batch width).
- Return distinct eligible IDs, at most min(workers,remaining), or `[]` to stop.

`dream_policy({id:"positive-sum",expected_revision:<latest>,policies:["<Rhai expression>"]})`
evaluates 1..4 candidates and the incumbent across every recorded tree. Replay
reveals stored children; it does not generate outcomes or execute candidate programs.
Root children open in recorded order; nonroot branches follow their recorded chain.
A recorded dead end reveals no new result. If a whole batch has no recorded
continuation, replay stops with history_boundary=true; this is missing evidence,
not proof that the corresponding online branch would fail. A policy cannot explore
outside history. Check obs.available before selecting IDs.
Policies must start with a nonempty batch, so they can collect future evidence.

Fixed utility = best fraction passed - 0.01 * revealed attempts
+ 0.001 * revealed attempts / decision batches (zero for an empty run).
Only a strict mean-utility improvement with no per-tree best-score regression is
adopted. Ties keep the incumbent. Invalid candidates return errors and are not adopted.
At most four dreaming calls per completed-history version. Do not invent unnecessary
revisions to consume this budget.

The next `next` uses the selected policy for NEW real executions. Compare that
round's actual score and attempt count with earlier rounds, acknowledging stochastic
candidate generation and visible-test limitations. `rollback` with the latest
`expected_revision` restores the preceding policy between rounds.

Runtime stores bounded trees, the active/previous policies and the latest replay
receipt in DreamExperiment, restored through the existing checkpoint path. Normal
tool success does not itself promise a completed disk checkpoint. Do not modify this
runtime-owned evidence with generic ECS tools.
