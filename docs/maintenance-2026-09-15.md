# Runtime Receipt and Commit Audit

## Confirmed Findings

- `675e888` introduced calls to `split_filter_tokens` and
  `parse_with_without_keyword` without their definitions. An offline query
  crate check reproduced E0425. Its commit message described tests that were
  not present in the diff.
- `6a237d9` added `exclude = ["vendor"]` but left vendor crates explicitly in
  workspace members. The correction removes those entries and excludes the
  actual package paths. Dependencies are still compiled as needed.
- `d440f2f` described enum-like cache modes and zero as unlimited caching;
  the implementation is a Policy struct, and zero disables stage caching.
  Cache hits still collect source facts to compare versions.
- The latest session log contained repeated tick-steward final-contract
  failures. It did not retain rejected final response text, so the old log
  cannot prove that prose wrapping was the only cause.

## Result Contract

```text
Tick reports -> observation lease -> bounded tool stage
    -> all Text response blocks (not Thinking)
    -> JSON structure + event coverage + successful-write evidence validation
        accepted: acknowledge lease
        rejected: one tool-free correction response
            still rejected: fail transaction and defer lease
```

The correction turn cannot perform additional tool calls. Diagnostics retain
the rejected text length and an escaped preview of at most 320 characters.
Prose surrounding JSON and multiple JSON documents remain invalid. The runtime
does not invent dispositions or evidence to make a batch pass.

Successful ECS writes are not rolled back when final validation fails.
Therefore a successful modify_component receipt does not prove that the
observation batch was acknowledged. Retries must inspect existing state and
avoid duplicating effects.

## Query Scope

Use `* where WITH(Health, Position) AND WITHOUT(Dead)` or mix keywords with
the existing presence and value filters after `where`. Keywords and AND are
case-insensitive. Quoted literals retain commas, parentheses and AND text.
Malformed lists, missing parentheses and incomplete conjunctions fail parsing.
The keywords are not a new fetch-list syntax.

No new Lens stage, live Rhai script or world snapshot is changed by this repair.
Adding archetype membership stages later requires preserving source component
membership separately from projected values: after project, the value object
is not an entity archetype description.

The existing tag and commit history are retained. Corrections belong in a
follow-up commit rather than silently moving the tagged checkpoint.
