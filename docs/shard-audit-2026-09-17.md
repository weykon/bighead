# Shard Data and Interface Audit

Read-only measurement at 2026-09-17 17:53:53 +0800, using local-default's
25,849,222-byte v3 checkpoint plus its one committed journal frame. The scanner
verified the frame checksum/chain/HEAD and confirmed the base and HEAD did not
change during the scan. This describes persisted state, not unsaved runtime data.

## Findings

- 19,669 entities; 5,776 ShardData entities in 1,012 groups.
- 1,039,694,017 bytes of logical shard text. All 4,838 referenced unique blobs
  passed SHA-256 verification (1,023,797,115 bytes).
- 963 context:tool_result groups hold 1,037,391,882 bytes, about 99.78% of text.
- 22 live groups have retention policies, none pinned; 990 have no policy.
  Of those 990, 983 are context groups. Unmanaged does not mean unreferenced.
- Seven groups have explicit shard source labels. Existing ActionRecords contain
  four successful shard_content calls, 111 get_shards and 146 get_shard_content
  entries. These are retained records, not a complete historical invocation count.
- Explicit groups include overview_components (23 shards), world-overview-projection
  (24), spark-archive (31), external:axum-harness-2026, agenthand progress,
  tool:execute_cli:world_overview and mind_view_v10_tail.
- spark-archive is structurally ambiguous: 31 separate entities all declare index
  zero and total one. Content remains stored, but group/index cannot uniquely
  address it. Existing dedicated reads return the first matching entity. No live
  repair or deletion was performed.
- Structured data already exists: 18 WorkItem, 21 AgentHandSessionV1,
  13 TopicInsight, 24 TopicRelation, 15 RegistryIndex and 4 LensApplication
  components, among others. Their existence contradicts a blanket claim of no
  modeling, but does not prove shard-to-fact provenance or successful reuse.

The scanner examined full-paradigm references in non-shard persistent metadata
(excluding ProjectContext creation receipts) and systems for the top 20, all
managed groups and explicit groups. It did not scan cross-shard body references,
external consumers or entity-ID-only dependencies, so it cannot declare orphans
or prove absence of derived facts. The full report and reproducible script are
outputs/measure/current-shards-2026-09-17.json and current_shards.py (gitignored).

## Interface Changes

shard_content now accepts content_ref="paradigm:<existing group>" as well as
__last_tool_result. It verifies completeness, orders chunks and concatenates
stored bytes without separators or overlap removal. Source data is untouched;
dedicated source access renews retention. Historical tool_call_id is still
unsupported and returns an explicit error. Existing or reclaimed destination
names are rejected; use a new batch-specific name for archives and derived groups.

Tool schema now exposes ranges (zero-based half-open line intervals), head_lines
and tail_lines. Dispatch rejects malformed/out-of-bounds ranges, invalid numeric
parameters and head/tail sum overflow. Failed component insertion rolls back the
new destination entities. Large valid overlap values now always advance the
split cursor. The archive guide uses a unique batch ID rather than spark-archive.

No live checkpoint data was edited, no running process restarted, and the existing
ambiguous archive was preserved. Rebuild/restart is required for the new interface.

## Validation

AI/loop/shard library suites passed 265 tests with 9 ignored; the subsequently
added tool-schema regression also passed. Cargo check for big-head passed.
Diff whitespace checks passed for files changed in this task. The full worktree
check reports an unrelated new trailing blank line in runtime.rs, left untouched.
