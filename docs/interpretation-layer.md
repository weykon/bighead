# Interpretation layer

The interpretation layer preserves evidence and stores task-dependent readings as
separate ECS records. It is deliberately not a lossy rewrite of the source world.

## Data model

- `FactRecord`: immutable observed or independently verified occurrence. Older asserted/hypothesis records remain readable but cannot be created through `record_fact` now.
- `InterpretationFrame`: the task, purpose, scope and parameters that define a viewpoint.
- `InterpretationRun`: one execution receipt, including source versions and cache use.
- `InterpretationMethod`: a versioned, pure Rhai method for reading a Lens working set through a Frame.
- `InterpretationResult`: the bounded Lens projection or Frame-driven reading.
- `RelationAssertion`: frame-scoped semantic relations and `derived_from` provenance.
- `MaterializationState`: exact dependency cache and active/superseded state.
- Existing `LensApplication`: the executable, read-only selection and projection method.

Facts are append-only. A correction is another fact. An interpretation never upgrades
itself into an objective fact, and a derived result cannot be used as its own evidence
unless a later, explicit Lens chooses that source and keeps the resulting provenance.

## Smallest loop

1. `record_fact` records complete, bounded observed/verified evidence in a structured JSON object payload. JSON-encoded strings are rejected because Lens pointers must address fields directly. Large bodies belong in `ShardData`.
2. A Lens selects a bounded working set and projects the fields relevant to one frame.
3. `materialize_interpretation` stores the frame, run, output and provenance relations.
4. The same Lens/frame/input versions reuse the result and add only a cache-hit run.
5. Changed facts produce a new result and supersede the old active materialization;
   history remains inspectable through `inspect_interpretations`.

For a one-off recommendation, `record_interpretation` accepts a Frame, 1..32
existing observed/verified FactRecord ids, and a reading with evidence-backed
reasons, blockers and unknowns. It writes a derived result and provenance,
without requiring a new Lens/Rhai method. Identical calls reuse the result and
do not advance the observation stream. This is where an agent's advice belongs;
it must not be recast as an observed FactRecord. Such results carry the reserved
`lens_id=direct.fact_records` marker; it names the direct input path, not a
registered Lens definition.

## Frame-driven reading

`materialize_interpretation` accepts an optional `method_id`. Without it, the
existing Lens projection is saved unchanged. With it, a registered
`InterpretationMethod` receives only two immutable values: `frame` and `facts`
(the bounded Lens output). Its final Rhai expression must return:

```rhai
#{
    verdict: "continue",
    reasons: [#{text: "Build passed", evidence: [facts[0].source_id]}],
    blockers: [],
    unknowns: ["Tests unobserved"],
    relations: [#{from_ref: "garden", predicate: "can_continue", to: true,
                  evidence: [facts[0].source_id]}]
}
```

Register the script with `define_interpretation_method` using `id`, `version`,
`lens_id`, `script`, and the allowed `verdicts`. An update requires the current
`definition_hash` as `expected_hash`; `inspect_interpretation_methods` returns
that hash. An update keeps the previous version inactive for inspection; only
the current version runs. Pass the method id and a Frame with `parameters` to
`materialize_interpretation`. The same Lens output and Frame can then yield
different readings because the method evaluates Frame parameters.

Every reason and relative relation must cite a Lens input `source_id` or a
FactRecord id explicitly projected in a field named `evidence_fact_id` or
`*_evidence_fact_id`. Rust validates those references and creates provenance
relations. The script cannot choose its own `derived_from` records. This checks
source identity, not the semantic truth of the reason; methods still need test
cases and review. Unknown evidence belongs in `unknowns`.

The method engine has no ECS, CLI, network, or clock functions. It limits script
operations and output size. Method id, version, and definition hash join the
Lens inputs and Frame in the cache key; changing any of them creates a new
result while preserving earlier results. A malformed result fails before any
Frame, Result, Run, or Relation is written.

## Observation over time

Ordinary conversations and tool-driven interactions now also enter source-local
experience windows. See [Interaction Experience Loop](interaction-experience-loop.md)
for evidence capture, viewpoint readings, asynchronous observation and conversation recall.
These runtime evidence records do not create verified facts or user preferences.

New method-driven `InterpretationResult` records enter an
`InterpretationStream` for their Frame scope. Plain Lens projections and exact
cache hits do not enter it again. When a scope accumulates
2 new results by default (configurable with `BIGHEAD_INTERPRETATION_WINDOW_SIZE`, 1..60),
the idle runtime starts one bounded, detached request for the read-only
`interpretation-observer` LLM system. This counts interpretations, not rendered
game frames or clock ticks.

The system may notice continuity, change, contradiction, or no justified new
pattern. Its text is stored separately as `InterpretationObservation`, with the
input result ids and the preceding observation id. The next window includes
that preceding observation as context, not as an objective fact. A failed LLM
call leaves the window pending and retries after five minutes. The main thread
commits the result after checking the window still matches; foreground requests
do not wait for the observer's model call. Original facts
and interpretation results are never overwritten by an observation.

This is an experimental temporal loop, not a claim of consciousness or a
verified semantic conclusion. The model reads bounded excerpts of each
interpretation; result ids retain the full trace for later inspection.
Observer inputs now type each provenance link as `fact_record` (with its
epistemic status) or `lens_source_entity`. A source entity id is not a fact id;
an `artifact_path` field is not proof that the file exists or works now.
`inspect_interpretations` reports pending counts and accepts `observation_id`
to read one observation and its exact input result ids.

Frame `parameters` are always a JSON object. Omit the field to use `{}`; string or scalar
coercions from a tool transport are rejected before a Lens runs or artifacts are written.

Summary inspection counts records without cloning the history. Detailed run and relation
history is paged (50 items by default, 200 maximum), so a growing logical history does
not force an equally large per-call working set.

The current demo can also project a relation from each Lens output item. The relation is
relative to the frame and result, not a global edge that silently changes every future
interpretation. A missing or JSON `null` relation endpoint rejects the whole
materialization before any interpretation artifact is written; unknown evidence is not
stored as a semantic relation.

## Laya-first routing

Use models only after deterministic work:

1. Exact materialization match: reuse it, no model call.
2. The required Lens is already known: run it, no model call.
3. Choice among a small set of known Lens definitions is uncertain: call
   `select_interpretation_path`; local Laya/MLX selects a Lens, `main_agent`, or `defer`.
4. No existing Lens expresses the viewpoint: the main coordinator reasons about a new
   interpretation method, which must be reviewed and registered before reuse.
5. Rust validates and persists the result; model confidence never becomes evidence.

Laya is a good fit for the third step because its current contract provides bounded
`choice`, `score` and `noul` decisions. It is not currently an arbitrary structured
semantic generator. `select_interpretation_path` is therefore advisory and read-only;
it never runs a Lens or writes interpretation entities. The caller must explicitly pass
the selected Lens and frame to `materialize_interpretation`.

Candidate ids are sorted before they are mapped to Laya's positional choice labels, so
reordering an equivalent candidate set cannot change the request or selected route. This
removes caller-order sensitivity; it does not prove that the model itself has no label or
position bias, which remains an evaluation concern.

The local service defaults to `http://127.0.0.1:3031`. Its health response must report
`backend: mlx`; failure to reach it is a local routing-service failure, not a remote LLM
provider error.
