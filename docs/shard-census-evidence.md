# Shard Census Evidence

The observed failure was not missing shard storage: the operator fetched one
50-entity page plus two narrow samples, then presented global distribution,
maximum size, overlap and historical-reference claims. The final narrator only
received that interpretation and repeated it.

## Tool Contract

inspect_shards defaults to view=summary. Its summary scans all matching shard
metadata, independent of item pagination. It reports logical content bytes,
group counts, managed/unmanaged/pinned counts, prefix-family aggregates
and five largest groups by the sum of their shard bytes. Prefixes are naming
categories, not semantic classification. Summary scope is either entire_world or
all_matching_groups, with the filter included. The family table displays at most
32 rows and explicitly reports when families are omitted. Exact global totals
still include omitted families.

view=groups pages group totals in descending byte order; view=shards pages exact
entity metadata. Coverage distinguishes computed aggregates, returned item count,
total matching items and whether any body was read (false). Responses are capped
at 14000 serialized bytes; next_offset reflects the actual returned items.
Oversized metadata that cannot fit even one item produces an explicit error.

expected_snapshot detects catalogue metadata changes between calls. This is a
metadata digest, not a transaction snapshot or content integrity hash: a same-size
body edit alone need not change it. It includes identities, source labels, times,
indices, sizes, policies and explicit source links. Statistics describe the live
in-memory world at measurement time; they do not assert checkpoint durability.

Created timestamps are formatted as ISO UTC, avoiding model conversion. Epochs
beyond year 9999 return null. Unrecorded split strategy remains unknown, including
legacy two-chunk groups. No method is guessed from a paradigm or chunk length.
Retention eligibility is explicitly not guaranteed deletion. All context artifact
subtypes can be managed; legacy policy absence does not identify source type.

Reference evidence covers only explicit ShardSources links. Other component
references, historical usage and external consumers are not scanned by this
tool, and orphan status is explicitly unknown. Existing zero links cannot prove
that no business artifact was derived or that data is disposable.

## Final Response Handoff

The narrator now receives raw successful shard summaries and scope metadata in
shard_evidence alongside the operator receipt. The latest unfiltered summary is
prioritized, followed by at most two distinct filtered summaries, within a 10000
byte evidence budget. Failed/unparseable outputs are excluded. When evidence is
present, operator prose is limited to 2000 characters with a truncation flag;
other operation handoffs retain their prior 10000-character limit.

The narrator prompt prioritizes supplied aggregates over contradictory operator
claims and forbids combining snapshots/filters or upgrading zero explicit links
to zero historical use. This improves evidence availability; it is not a promise
that model-generated prose can never contain an error. No live model evaluation
or running-process restart is performed by the regression tests.

## Compatibility

Callers expecting entity items must now pass view=shards. Default summary returns
items=[] and still provides matching_shards/matching_groups. Existing source links
and live shard data are not mutated by inspection. Rebuild/restart activates the
new tool contract; persisted custom agent prompts are not overwritten.

## Validation

Five focused catalogue tests passed, including four new regressions covering the
observed inference failures. The loop/AI/application regression passed 308 tests
with 11 ignored and no failures. Coverage includes a largest group outside the
first entity page, group totals exceeding the largest individual shard, unmanaged
tool results, filtered scopes, UTC conversion, unknown split/reference status,
metadata changes between pages, bounded response coverage and narrator evidence
selection. These tests validate deterministic tool and handoff behavior, not a
live model's final prose.
