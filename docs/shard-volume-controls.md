# Shard Volume Controls

The 2026-09-17 measurement found 1,039,694,017 bytes of logical shard text,
99.78% from tool-result projections. This change controls new amplification;
it does not delete, truncate or silently rewrite existing source material.

## Output and Storage

World overview, entity queries, entity/component reads and shard listings now
return at most 4000 characters by default (output_limit up to 8000). Oversized
results include total_chars, next_offset, has_more and sha256. Passing
expected_sha256 rejects a changed result rather than combining different pages.
The omitted suffix is not copied into another shard. These reads currently still
generate the complete result before paging, so CPU/memory cost is not yet bounded
by page size. Recomputed pages are not transaction snapshots.

Other tool results larger than 16 KiB are projected before entering conversation
history and ToolHistory. Those consumers receive a reference and bounded preview.
ToolHistory already capped its per-entry summary at 500 characters; this change
puts a usable source reference into that summary rather than claiming it previously
stored every full result. The full per-cycle tool log also now holds the notice.
The last-result cache retains the original for __last_tool_result. If projection
fails, the original is preserved and the existing outbound guard remains active.
Explicit bounded shard reads are excluded from this early projection step.

Context projections now use source-kind plus content SHA-256 as identity, without
message position or tool-call ID. tool_result and tool_result_budget share a
namespace. Identical new content across calls reuses a verified complete group;
per-call origin remains in ProjectContext audit receipts when a transaction is
active. The shard's source_label remains its initial source. This is exact-content
deduplication, not semantic deduplication or retroactive merging of legacy groups.

## Discovery and Provenance

inspect_shards lists metadata without full bodies, including unmanaged groups,
entity IDs, ambiguous indices, retention and explicit derived-entity links.
Pagination is deterministic within an unchanged world. link_shard_sources merges
up to 16 source paradigms onto an existing artifact as ShardSources. This component
is persisted, reflected for reads and available to Rhai/Lens; retention treats it
as a reference. These links describe provenance, not correctness or proof that
all text was read. Absence of links does not prove absence of use.

Re-sharding via paradigm reference automatically links the first destination
entity to its source. Agent-authored structured artifacts require an explicit
link call. Guidance connects discovery, bounded reading, artifact creation,
provenance and task-relevant Rhai/Lens validation without inventing business facts.

## Legacy Archive Access

The 31 spark-archive entities with duplicate index zero remain unchanged.
get_shards and inspect_shards expose exact runtime entity IDs. A duplicate
paradigm/index read now fails explicitly unless entity_id selects a matching
entity. This restores addressable access without renaming historical references.
IDs are runtime-local; rediscover them after restore. New writes already reject
existing destination groups and the archive guide requires unique batch names.

## Activation and Remaining Work

Rebuild and restart to activate. This implementation does not restart processes
or modify the live checkpoint. The existing 1.04 GB logical corpus will not shrink
immediately. Actual historical reclamation still requires reference/retention
decisions, followed by committed save and backup-aware blob GC. Cross-file and
entity-ID-only dependencies are not comprehensively tracked. No historical group
was classified as disposable from an empty metadata reference search.

## Validation

Selected workspace unit/doc suites passed 390 tests with 13 ignored and zero
failures. Cargo check for big-head and diff whitespace checks passed; existing
compiler warnings remain. Tests cover Unicode page reconstruction and change
detection, a 200,000-character read that creates no shard, repeated large tool
results sharing one group, exact legacy archive access, source-link merge/failure,
reflected reads, restore and retention protection. These are correctness checks,
not a live-world claim about reclaimed bytes or latency.
