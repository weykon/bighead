# Work District Prototype

Open the existing demo at `http://localhost:3033/#panel=character-demo`. The view selector switches between the work district, its book, and the original programmable room. On a world without a district, **建立项目街区** explicitly imports five reviewed project records. Opening the page alone does not import data into other worlds.

## Shared Facts

`WorkRecord` stores a stable ID, title, summary, status, evidence IDs and revision. `WorkAppearance` on the same entity stores position, height, model style, color and glow. `WorkConnection` stores directed links between stable work IDs. The city and book read the same projection; there is no second copy of chapter/task facts. Moving a building updates presentation only.

The initial district contains scene scripting, checkpoint journaling, Rhai practice, the work district under development, and the unresolved serial-simulation limitation. Sources are captured quotations from project documents and the user's request, with source paths and SHA-256 digests. They are historical project records, not tests rerun by the model. The UI exposes the exact quotations. Completion evidence is scoped to its original work; another building's evidence cannot complete a new task. The projection displays `unverified` when a completed record lacks applicable completion evidence.

This prototype does not ingest Git, Codex, agent-hand or live test outcomes automatically. Its evidence catalog is a reviewed initial import. The agent can create new ideas/in-progress work and reorganize records; new completion evidence currently needs an explicit developer import. A source snapshot is provenance, not a semantic proof that every sentence in a summary is correct.

## Agent Operations

- `inspect_work`: records, evidence catalog, appearance revisions, connections, bounded events and trigger health.
- `record_work`: create with expected revision 0, or replace using the current revision. Status and evidence are validated; appearance is unchanged.
- `arrange_work`: validate the whole partial batch before applying layouts and connection changes. Existing unrelated objects remain. Coordinates, height, supported styles, capacity, endpoint existence and minimum center spacing are checked. A stale appearance revision rejects the batch.
- Existing `define_component`, Rhai system creation/hot update and ECS tools remain available. A script can update `WorkAppearance` or compose behavior from custom components. Generic scripts retain broad ECS authority and can bypass mutation-tool guards; this is not a hostile-code security boundary. Invalid visual data is omitted with a warning.

Current rendering vocabulary: archive, workshop, laboratory, tower and garden. Up to 64 work records and 128 directed links are projected. Building shape and scaffolding distinguish recorded completion, ongoing work and blockers. Flags are decorative browser animation, not background agent reasoning.

## Interactions

Selecting a building, a book chapter or a connected work opens its facts and sends `work_enter` with a fresh interaction ID and stable target ID. The runtime appends an `enter` event to `WorkDistrict` and invokes matching `WorkTrigger` components. Target `*` matches any work; the event name is `enter`. A trigger names an existing Rhai system and records enabled/runs/error. No model is called by this path. Clicking **接着讨论** binds the next ordinary chat to that same work ID.

Triggers receive only `args.event`: ID, target_id, kind and sequence. Continuous `SceneSystem` scripts separately receive dt/elapsed. Delays/fades should keep a per-object remaining duration and subtract dt; event handlers reset that duration. These two clocks must not be conflated.

Recent event IDs are deduplicated across the last 32 events, including after restore. IDs reused for another target are rejected. Receipt persistence precedes execution; results persist afterward. This favors at-most-once execution within that window: interruption after the receipt can leave an event unexecuted, and it is not automatically replayed. A script failure disables that trigger; preceding writes can remain. Up to 16 matching triggers execute per event with the existing bounded Rhai engine. Triggers beyond this bound are disabled with an error.

Event receipts distinguish accepted, executed, failed and no-handler outcomes. A duplicate receipt retries persistence without rerunning scripts. This is not a transactional rollback or an exactly-once protocol.

Events submitted while the model is busy wait in the main-loop event queue. The prototype still uses serial execution; ECS animation pauses during model requests. Entering means opening an object's inspector, not physical walking, collision detection or autonomous navigation. Roads visualize work relationships; no pathfinding is implied.

## Persistence And Verification

District facts, evidence snapshots, layouts, connections, recent events, triggers, custom components and Rhai systems use existing checkpoint/journal persistence. Initialization is idempotent and does not reconstruct removed records. Normal headless Ctrl+C checkpoints at the next safe point. Forced termination can lose changes since the last successful save.

Tests in `work_world/tests.rs` cover layout/fact independence, stale and overlapping layouts, missing road endpoints, completion evidence guards, shared projection updates, actual Rhai event execution, deduplication and restore. Browser/MiniMax checks are in ignored `outputs/room-demo/work.cjs`; they exercise autonomous rearrangement, event installation, a new idea/garden, common book facts, mobile/desktop canvas checks and restart. A separate glow check exercises two real enter events and verifies their delayed fade, rather than trusting a model's manual test summary.

The 2026-09-22 run passed 282 loop-library tests (13 existing ignored), the frontend build and three desktop/mobile viewport checks. MiniMax rearranged the five imported records, created a sixth idea as a garden and linked it to the laboratory. It authored the event handler and fade system. The first fade used incompatible event/scheduler time origins; a subsequent model repair changed it to per-object remaining time. Two real clicks then measured glow around 1.39 after 0.5 seconds and 0.3 after settling, with the original trigger/scheduler retained. Model-generated behavior still needs this kind of execution verification.
