# ECS operation model v1

Discover -> inspect current state -> choose a legal operation -> execute -> verify.
Use this method when a task needs ECS work, not as mandatory overhead for greetings.

## Discover actual capabilities

`inspect_ecs_capabilities({view:"operations"})` lists the ECS operations available
to this stage, their effects, preconditions and verification requirements. Follow
`next_offset`; the current page is not the entire catalog. Pass `name` for the
exact tool input schema. This is a curated ECS operation catalog, not all tools.

`view:"components"` lists registered JSON handlers and dynamic schemas, including
types with zero instances. A Rust schema of null means unknown here, not empty.
An explicitly empty dynamic schema is a presence-only marker. This list is not
every opaque Bevy internal component and does not override write restrictions.

`view:"systems"` reads SystemRegistry. Systems are not ECS entities named
RuntimeSystem or SystemScript. Use `name` for a definition hash and paged source;
pass `expected_definition_hash` on subsequent source pages. Source offsets count
Unicode characters. Catalog metadata never proves that a system ran or is healthy.

## Compose existing capabilities

Define a missing schema once. Attach a marker using modify_component with value
{}; adding/removing a component changes archetype membership while preserving the
entity. There is no separate archetype object to author or synchronize.

Use inspect_world_shape for actual composition counts. Query `* where WITH(A)
AND WITHOUT(B)` to find entities with A but without B. Fetching `A, B` alone does
not require both. Read the query guide for predicates; do not invent dynamic
Changed/Added filters.

modify_component upserts: an absent component requires a complete value, while
an existing component accepts a schema-valid patch. Define schemas before spawn.
Read stable source identities and resolve current entity IDs after restore.

## Hot-edit behavior

define_system creates a registered Rhai definition, not a periodic task.
define_tool also exposes it to the agent. The next tool wave refreshes the tool
list, subject to the current stage's allowlist.

Custom tools and run_system accept an args JSON object. A fresh Rhai `args` map
contains these values for each call (for example, args.entity_id). Missing args
means an empty map; a non-object is rejected before execution. Arguments do not
persist across calls. Only is_tool=true systems can be called by their own names.

To change an existing behavior:
1. Inspect the exact system and retain the complete original source/metadata.
2. Call update_system with id, expected_definition_hash, complete source and reason.
   description/is_tool are optional; omitted metadata is preserved.
3. Inspect the new definition. Then run_system and inspect its intended ECS result
   when the task authorizes that execution. This is separate from publishing code.
4. If the behavior is wrong, update back to the retained source/metadata using the
   NEW current hash. Reverting code does not undo effects of previous executions.

Update checks syntax and the full definition hash before replacement. It does not
run the source, validate every referenced component/helper, or migrate state.
In-flight evaluations finish before this exclusive registry edit; later calls
resolve the new definition. No Bevy native function pointers are patched here.

remove_system requires the current hash and rejects explicit tool_names dependents.
It removes future registration/tool exposure, not earlier ECS effects, free-form
code/text references or TickActions. Those callers must be checked separately.
Built-in guides and LLM stages are not editable through these tools.

TickAction stores its own script. Use update_tick_action to change that script;
updating a similarly named system does not update TickAction. Periodic execution
requires an explicit scheduler/trigger, not a claim of registration. Verify actual
execution with inspect_runtime and TickActionHealth.

## Evidence for a future decision model

The operation catalog is a model-readable contract, not trained model weights.
Existing ActionRecord/TransactionRecord capture real calls and results. System
edits include definitions before/after in the audit when a transaction is active.
Persistence follows the normal checkpoint/journal lifecycle, not per-call fsync.

A training example needs task intent, pre-action observations, available choices,
chosen operation, the definition version used, actual result and an independent
task check. Keep post-action information out of decision inputs. Mark unknown
outcomes unknown. Tool success or valid syntax is not a positive correctness label.
Do not generate extra entities or duplicate whole-world/shard snapshots merely to
collect samples. This version does not export training datasets or train weights.

Current Dream practice uses pure Rhai tasks, not isolated mutable ECS task worlds.
Do not claim its replay results validate these production operations.

## Boundaries

Rust component layout migration, Rust system code hotpatching and dynamic
Changed/Added query filters are not exposed. Dynamic schemas can be defined and
retired when unused, but cannot be changed in place by this interface. Retiring a
schema leaves Bevy's registration behind; do not assume its name can be reused.
