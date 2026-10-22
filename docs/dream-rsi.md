# Dream-RSI: Rhai Practice

This first implementation connects a fixed task, real program execution, recorded
exploration trees, offline policy comparison, and the next online round. It is
inspired by [Dream-RSI](https://arxiv.org/abs/2609.14858), not a reproduction of its
benchmark results. The deployed object is an exploration controller for practice;
neither the underlying model nor production ECS systems are modified.

## Run

After rebuilding/restarting Big Head, the coordinator can use `dream_task` and
`dream_policy`. The complete protocol and examples are bundled in
`run_system("guide_dream")`.

A real MiniMax smoke run is also available:

```sh
cargo run --example dream_practice
```

It reads the normal `.env` configuration without printing credentials, creates a
fresh in-memory world, and requests two online rounds with one dreaming phase.
MiniMax authors both the candidate programs and controller revisions. The runtime
measures the outcomes. The first four-attempt baseline round is deliberately run
to its budget, so the controller can subsequently learn to stop after success.
This is a small orchestration test, not evidence of general reasoning improvement.

The example writes checkpoint and `report.json` under a unique
`outputs/dream-practice/session_...` directory. It never loads or writes the live
default world. An incomplete model session returns a nonzero exit status after
saving evidence. No daemon, TickAction, Spark or automatic retry loop is installed.

## Mechanism

- A task freezes its description, JSON input/expected cases, attempt budget and
  worker width. Scoring is exact JSON equality, averaged over those visible cases.
  Creation accepts cases_json as explicit JSON text, preserving numeric, string,
  null and empty-array types across model tool transports without coercion.
- `next` selects eligible parent IDs using the active Rhai controller. `submit`
  executes candidate expressions in a separate bounded Rhai engine and records
  parentage, source and measured results. Programs receive only `input`.
- Completed trees are retained as replay worlds. A replay starts with an empty
  prefix and reveals only selected recorded children. It cannot rerun programs,
  call a model, query ECS or synthesize a missing outcome.
- Controller code receives revealed scores/topology and returns a batch of parent
  IDs. Root branches open in recorded order; nonroot continuations follow their
  original chain. Batching is modeled, but local execution is sequential in v1.
  A batch without any recorded continuation ends replay with history_boundary=true.
  This marks unavailable evidence, not an online failure or a proven stop decision.
- Each candidate and the incumbent are scored on all completed trees. Utility is
  `best - 0.01 * attempts + 0.001 * attempts / batches`. Adoption requires a strict
  mean improvement and no per-tree best-score loss. Ties retain the incumbent.
- The selected controller drives the next real round. The preceding controller
  can be restored with `rollback` between rounds.

Generic ECS write tools reject edits to DreamExperiment. This is an accidental
tampering guard, not a security boundary against a privileged CLI or custom native
runtime code. Task evidence is owned by the runtime's dream tools.

## Durability and Bounds

DreamExperiment uses the existing native-component checkpoint and journal path.
Its stable task ID, active/previous policies, completed trees, pending batch and
latest replay receipt survive restore. A normal tool receipt means an in-memory
change; normal runtime checkpoint timing still determines disk durability.

There are at most four experiments per world, eight trees per experiment and 32
attempts per tree. Cases are limited to 16 and 16 KiB in total, scripts to 8 KiB,
and each experiment to 1 MiB. Only the latest submission/replay receipts are kept.
The runtime permits four dreaming calls per history version, with at most four
candidate policies each. It refuses further growth at the limit; it does not
silently delete old evidence. Existing ActionRecord retention remains unchanged.

## Verification and Limits

```sh
cargo test -p big-head-loop dream --lib -- --nocapture
cargo test -p big-head-ai guides --lib
```

Tests cover the full loop, actual next-round control, prefix visibility, no program
execution during replay, invalid/regressive controllers, operation bounds,
idempotent batches, checkpoint restore, capacity limits and tool dispatch/audit.

The 2026-09-20 real MiniMax smoke run completed two rounds and one replay. MiniMax
authored the programs and proposed a controller that stops when `obs.best >= 1.0`.
The baseline used four attempts at score 1.0; replay selected the new controller,
and the second real round used one attempt at score 1.0. DreamExperiment occupied
2,313 bytes; the isolated checkpoint including normal runtime audit was 114,578
bytes. Evidence is in
`outputs/dream-practice/session_3820b364-c6ff-4d90-820e-09a2a68d796a/`.
This validates the loop and early stopping, not a harder-task capability gain.

The first smoke run is also retained. It exposed numeric test values being submitted
as strings through an underspecified tool schema, and dream writes being classified
as read-only by the convergence guard. Explicit cases_json transport and action-aware
write classification fixed these integration issues; regression tests cover both.

Historical improvement does not guarantee future improvement. Visible test cases
can be overfit; this version has no hidden test set. The agent can inspect completed
trees while developing policies, but policy evaluation reveals only prefixes.
Historical tool-output blobs and ActionRecords are not automatically converted to
trees: they usually lack counterfactual branches and fixed evaluation scores.
No learned program is automatically installed as a production Rhai system or Lens.
