# Programmable Character World (2026-09-22)

The current `character-demo` panel connects conversation to ECS tools. The model can create entities and dynamic components, author Rhai systems, update their code, and schedule continuous execution. The browser renders an ECS projection; orbiting and floating are not built-in actions or browser animation presets. The older bounded-proposal version is documented below as history and no longer describes the chat contract.

## Current Contract

| Component | Meaning |
| --- | --- |
| `SceneName` | Persistent UUID and name; numeric entity IDs can change on restore. |
| `SceneTransform` | World position, Euler rotation in radians, scale. |
| `SceneRenderable` | Primitive/asset kind, hex color, visibility, lamp intensity. |
| `SceneSystem` | Rhai system ID, enabled state, elapsed simulation time, run count, last error. |

Rendering supports box, sphere, cylinder, cone, torus, plane and existing lamp/desk/shelf/floor assets. Agent-defined components and scripts compose new behaviors by updating these visual components. Arbitrary meshes, uploaded assets, physics, collision validation and navigation are not implemented. Direct transform edits are free placement, not the old surface-checked lamp proposal.

Legacy furniture migrates once, retaining saved positions. Thereafter `SceneTransform` is authoritative; deleting furniture or removing a renderable does not recreate it on publication. Notes retain their inspector and user correction controls. General scripts have broader authority than the old proposal validator: quote validation and confirmed-note protection are not enforced for arbitrary scripts.

`character-world-agent-v1` exposes ECS discovery/CRUD, component definitions, system definition/execution/hot update and `inspect_scene`. Its reasoning budget defaults to 100 rounds; `BIGHEAD_CHARACTER_MAX_ITERATIONS` accepts 1..1000. Tool-loop convergence guards still stop repeated failures, read-only wandering and excessive action counts earlier. Iteration exhaustion is shown as partial progress with the actual runtime cause because successful writes are not rolled back. The agent returns ordinary text after tool calls. Its always-on system prompt contains only the role, current-context meanings, evidence rules and a route to `guide_scene_world`. The detailed scene/work capability manual is an on-demand built-in guide loaded with `run_system`; live facts come from `world_projection`, `inspect_scene` and `inspect_work`, not from the guide. Ordinary greetings and read-only discussion therefore do not pay for or receive the full manual. Inspection, trial execution and state readback are required for mutations; this is not a guarantee that every response fulfills the request.

## Continuous Execution

Schedules run in the main loop (nominally 50 ms), receiving `args.dt`, `args.elapsed` and the scheduling entity ID. ASTs are cached by system ID and exact source; code updates take effect next execution without replacing entities. Each call has a 100,000-operation budget and bounded collection/string sizes. Limits: 16 active schedules, 128 projected objects. Unsupported visuals are reported. Failed schedules disable themselves and expose an error; prior writes can remain.

Projection publishes at most every 100 ms. Frame steps update components in place and do not create per-frame ActionRecord, Spark or ShardData entities. Existing checkpoint throttling saves dirty state; interaction completion also saves. A crash can lose animation progress since the last save. Headless Ctrl+C requests a final checkpoint and graceful exit at the next safe point.

**Execution remains serial:** while a model request or other main-loop work is in flight, scene scripts pause. Simulation time resumes with bounded steps without catching up missed wall time. The character's decorative animation is browser presentation, not evidence of ECS execution during that pause. Concurrent simulation/model execution remains follow-up work.

Accepted chat receipts save before requesting the model. Changes and final responses save afterward. A model error does not undo successful tools; save failure is displayed and leaves the world dirty for retry. The last 16 interaction IDs deduplicate recent retries; in-flight turns become interrupted on restart. Scripts have broad ECS authority, so this trusted local prototype is not a malicious-code sandbox. Room tools expose no shell/network tools. Laya does not authorize mutations.

## Current Verification

`scene_world/tests.rs` covers scripted motion, changing one entity while another continues, source cache invalidation, stopping without resetting, runaway-script suspension, rejected visual data, migration and restore. Rhai `query()` now returns structured native fields rather than reflection Debug strings, matching dynamic component access.

The real MiniMax/browser acceptance script is `outputs/room-demo/world.cjs`: three colored cubes orbit the lamp, then one floats vertically while two keep orbiting. It checks actual ECS coordinate changes, refresh/reconnect, nonblank moving canvases and responsive layouts. Reports/screenshots are stored alongside it. After restart, `CHECK_RESTORE=1 node outputs/room-demo/world.cjs` verifies stable identities and resumed motion. The run command below still applies.

The completed run passed both MiniMax requests and a graceful-save/restart check. MiniMax authored `OrbitParams`, `FloatParams`, and the `orbit_cubes` script; all three stable object IDs survived restore. The loop/save library suites passed 290 tests, with 15 existing ignored tests. Frontend build and four viewport checks passed; the saved demo checkpoint was about 209 KB after the run.

# Historical Bounded Room Prototype

The `character-demo` View tab is a deliberately specific shared-room prototype, not a generic interaction framework. It uses the existing View transport, a native `CharacterDemo` component, and separate `RoomObject` entities. A Three.js character shares a desk and shelf with the user. The configured MiniMax agent replies and chooses whether to leave or update one small conversation object. Greetings may leave nothing.

## Run

```sh
cargo build -p big-head
cd crates/big-head-view/frontend
npm run build
cd ../../..
VIEW_PORT=3033 BIGHEAD_LAYA_SHADOW=0 target/debug/big-head --headless --view-server --world local:room-demo-20260921
```

Open `http://localhost:3033/#panel=character-demo`. Port 3031 is reserved for the local Laya service. The frontend connects to `/ws` on the same origin. For Vite development, the existing proxy forwards `/ws` to port 3030.

The dedicated world isolates the prototype from the main world's observers and queued work. MiniMax configuration is read through the existing `.env` path. The new tab also appears in the normal View after rebuilding and restarting the application.

## Contract

- A click on the character or greeting button sends `greet`; the text composer sends `say`. Each includes `interaction_id` and `text`.
- Local send and runtime receipt are different UI states. Only a persisted reply becomes a successful final response. Errors and restart interruptions remain visible.
- Accepted interactions are saved before the model request. Completed replies are saved before publication. The last 16 IDs are deduplicated, including after restore; an ID reused with different content is rejected. This is bounded retry protection, not permanent idempotency for arbitrary historical IDs.
- A `sync` event requests the authoritative state after connecting. The server also publishes the state on startup. Reconnecting clients receive the current panel snapshot.
- The character pipeline runs serially in the existing main loop. In a busy main world, other LLM stages can delay receipt. This prototype does not yet implement concurrent model scheduling, streaming replies, general scene actions, or autonomous character tasks.
- Animation is presentation, not a claim of autonomous reasoning. Only actual model replies appear in the conversation.

## Room Objects

- The tool-free `character-room-compose-v2` system returns a validated JSON reply with either no change, one note create/update proposal, or one movement proposal. Room mutations happen only after validation. There is no model-generated code execution or automatic installation of a system.
- Each model invocation has one reasoning round. A rejected JSON reply or room proposal gets one bounded correction retry before any mutation, with the validation error returned to the model. A second invalid result is shown as a local validation failure, distinct from an incomplete API call. Successfully applied proposals never enter this retry path.
- Each object has a stable UUID, revision, title, body, kind (`idea` or `note`), exact source quote, source turn ID, reason, location (`desk` or `shelf`), and confirmation state. New proposals must cite a substring of the current user message. The runtime constructs a `note` body directly from that verified quote, ignoring any model paraphrase in `body`; interpretations are displayed as guesses. Source validation does not prove an interpretation is correct.
- A maximum of eight objects is enforced. The context includes the last 16 turns, current objects, and last 32 room changes. A selected object can accompany a new message through `object_id`.
- Selecting a physical object or its list entry opens the evidence inspector. Confirm, correct, move and withdraw are direct runtime actions; they do not invoke the model. Corrections replace the object's body/evidence and mark it confirmed. Model proposals cannot overwrite confirmed objects.
- User mutations include `interaction_id`, `object_id`, and `expected_revision`. Stale revisions are rejected. The last 32 user/model room receipts provide bounded retry protection; successful mutations are saved before publication. Save failure rolls back the room mutation. ECS entity IDs may change on recovery; object UUIDs remain stable.
- Withdraw removes `RoomObject` and despawns an otherwise empty entity. If another ECS workflow has added components, those components survive. This is not erasure of historical conversations or audit logs. Recent withdrawal receipts are model context, not a permanent semantic ban on the topic. Confirmed preferences are room-local, not a global personality profile.
- The prototype remains serial: an in-flight model request can delay other page events. Laya is not used to authorize mutations. No background dreams or autonomous world-building are implied by the character animation.

## Language Actions

`RoomSceneItem` identifies the floor, desk, shelf and lamp. `RoomPlacement` stores their world-space base position, supporting surface and revision. Startup adds missing scene entities without overwriting saved placements. These are registered ECS components, visible to the existing ECS inspection and component tools.

The character receives the actual scene inventory and can return `move: {object_id, target, expected_revision, position?}`. For example, `lamp` can move between `floor` and `desk`. The runtime picks a free position when coordinates are omitted, checks support bounds, height, furniture/character collisions and reserved note areas, then invokes the existing `AgentAction::ModifyComponent` path. It reads the component back and generates the completion reply from that result. Publication follows persistence; save failure restores placement and receipts. Movement receipts include the original surface so a later request to put it back has evidence to use.

The browser renders the lamp from ECS coordinates and animates placement changes. Selecting it opens its current state and lets the next chat refer to it. Notes, including confirmed ones, may also be moved by language to desk, shelf or floor without rewriting their contents. The eight-note limit does not include furniture.

This iteration uses fixed room geometry and axis-aligned collision bounds, not a physics simulation or a walking/carrying animation. Desk, shelf and floor are registered fixed supports; moving them, moving the lamp to a narrow shelf, spawning arbitrary furniture, and multi-action batches are not supported. The model receives those limits explicitly. The room-specific movement path validates placement; unrestricted ECS tools elsewhere retain their existing authority and do not gain these checks automatically.

The language-action checks passed 275 loop-library tests (13 ignored). Real MiniMax tests in `outputs/room-demo/movement.cjs` exercised the original lamp-to-floor request, returning it to its previous location, a read-only location question, rejecting an unsupported shelf placement, and moving a confirmed note to the floor and back without changing its text. Desktop/mobile screenshots, canvas checks, refresh and reconnect checks are retained alongside `movement-report.json`.

## Observer budget

The main coordinator's default is 100 rounds. The separately configured tick-steward previously had 8; its default is now 100. `BIGHEAD_TICK_STEWARD_MAX_ITERATIONS` accepts 1..1000, otherwise defaults to 100. The actual configured limit is supplied in its context. A round is one model reasoning/API iteration, potentially containing several tool calls, not a TickAction execution. Higher limits can increase background latency and cost; successful stages still finish early. Changes take effect after restarting the rebuilt application.

## Validation

Rust tests cover evidence validation, confirmed-object protection, capacity, revision conflicts, retries, correction, removal, persistence failure rollback, journal restore, legacy state, and interrupted responses. Browser checks and screenshots for this room iteration are under `outputs/room-demo/`; the earlier character-only checks remain under `outputs/character-demo/`.

The 2026-09-21 run passed all 11 character/room tests. The loop library suite passed 269 tests with 13 existing tests ignored. Real MiniMax browser checks created an idea, applied a user correction, moved it to the shelf, read the corrected text back through the model, created and confirmed a second object, and withdrew it. Process restarts preserved the correction and did not resurrect the withdrawn object. Refresh, offline/reconnect, mesh selection, and animated nonblank canvas checks passed at 1440x900, 390x844, 760x850, and 360x640.

Exploratory runs exposed malformed JSON, incorrect update targets, and punctuation drift in literal notes; the final implementation adds bounded rejected-proposal retries, explicit room capability context, and runtime materialization of literal quotes. This is a working prototype, not a guarantee of model reliability. Earlier failed replies remain visible in the dedicated demo world's test history; one confirmed sample object remains on its shelf.
