# Checkpoint Storage and Scheduling

## Verified Diagnosis

A complete read-only scan of one open checkpoint inode measured 1,130,602,268
file bytes, 18,406 entities, 65 component names and 64 persisted component-name
combinations. The opened file did not change during the scan. This supersedes
the previous streaming report, whose EOF handling discarded buffered entities.

ShardData: 4,903 entities, 1,025,403,033 UTF-8 content bytes. The 73 contents at
least 1 MiB account for 75.55% of content. The largest is 20,995,234 bytes in one
entity with total_shards=1, consistent with the historical pre-8fc9744 projection
writer. The current 16 KiB chunker did not create this representation. Exact
duplicate content accounts for only 12,045,464 excess bytes (1.17%).

ShardData accounts for 98.04% of compact UTF-8 component JSON bytes; this is not
an exact percentage of the pretty-printed checkpoint file. The former report
used ASCII-escaped JSON character counts as file bytes and mixed measurements.
Its claims of 99.5%, all single-component entities, and 20 archetypes are withdrawn.

Eight TickActions have cooldowns of 600, 1800 (five), 21600 and 604800 seconds.
They do not execute every 50 ms. Observer and Spark delivery checkpoints also
write the world; 300-second observer backoff applies after failed processing,
not to successful delivery or every checkpoint. Rewriting a file does not by
itself explain content growth. No sub-second restore forecast is justified.

## Implementation

- Runtime marks TickAction changes pending and polls the save deadline on every
  main-loop iteration, even when there are no new reports. The default minimum
  interval is 900 seconds; BIGHEAD_TICK_CHECKPOINT_SECS=0 disables throttling.
- A successful full checkpoint clears pending ticks and records its completion
  using a monotonic clock. Explicit saves and critical observer/Spark delivery
  saves remain immediate. Background failures retain pending work and use a
  30-second retry delay; mandatory delivery does not proceed without its save.
- Saves serialize one entity at a time to a buffered temporary file. They no
  longer materialize a complete world snapshot plus a complete JSON String.
  Snapshot inspection APIs remain available independently.
- Checkpoint format_version=3 externalizes ShardData content over 4096 bytes to
  SHA-256 addressed UTF-8 files in checkpoint.blobs next to checkpoint.json.
  Entity component metadata carries content_blob={sha256,bytes}; its ordinary
  value omits content. Other data and small contents remain inline.
- Existing content is reused after size/hash verification. New content is
  written, synced and renamed before publishing a checkpoint. The manifest is
  synced and atomically renamed; parent directories are synced. Failed writes
  preserve the old manifest and clean temporary files. Completed but unreferenced
  blobs may remain after failure, deliberately avoiding unsafe reclamation.
- Restore accepts legacy inline JSON with no version field. It validates all
  external references before inserting schemas/entities, then restores complete
  text into ECS. Missing files, invalid hashes, same-length corruption and
  unsupported future versions fail explicitly. It does not silently omit an
  external component. The legacy-default migration copies sidecars first.
- Entity/component point reads preserve dynamic, reflected and opaque results.
  Tests now register reflection rather than testing only opaque fallback.
  Machine-dependent timing tests are manual, not CI correctness assertions.

## Validation

```sh
cargo test -p big-head-ecs -p big-head-loop -p big-head-script -p big-head-save -p big-head-world -p big-head-query -p big-head --offline
BIGHEAD_TICK_CHECKPOINT_SECS=0 cargo test -p big-head-loop --offline tick_checkpoint
cargo test -p big-head-save --offline measure_large_shard_checkpoint -- --ignored --nocapture
cargo check -p big-head --offline
git diff --check
```

Coverage includes pending-tick persistence with no subsequent reports, mandatory
save clearing the pending flag, failed writes retaining pending state and retry
backoff, legacy restore, Unicode shard round trips, immutable content reuse,
corrupt/missing blob rejection before world changes and preservation of the old
manifest on save failure. No live model or live data migration is needed for tests.

Previous storage/scheduling validation: 325 passed, 13 ignored (including manual timing and doc examples),
zero failures. The interval=0 targeted run passed 2 tests. Cargo check and diff
whitespace checks passed. Existing vendor cfg/dead-code warnings remain.

Manual debug-build measurement with one 20 MiB ASCII shard: old snapshot plus
pretty JSON serialization CPU 685 ms (20,971,713 bytes); first external save
203 ms; repeated external save 34 ms; manifest 296 bytes with full text retained
in the blob. The new timings include file sync; the old baseline excludes disk
writing and is not a full end-to-end comparison. These are synthetic local
numbers, not a live-world latency guarantee.

## Operational Contract

Rebuild and restart to activate. The next save writes v3 and externalizes eligible
contents; the running process and existing business data have not been migrated
by this development work. First conversion needs room for the new blob files
alongside the old manifest until atomic replacement. Do not erase old files to
make room without a verified backup and explicit cleanup scope.

Back up or move checkpoint.json, checkpoint.blobs AND checkpoint.journal together.
If renaming the JSON filename, rename both corresponding sidecar directories too.
An old binary is not compatible with v3; rollback requires a compatible backup.
Do not remove blobs based solely on one manifest: older backups may reference
them. Automatic policy-based shard and file GC is described in
[Checkpoint Lifecycle](checkpoint-lifecycle.md); legacy groups remain permanent.

This reduces manifest writes and temporary serialization memory, not resident
ECS content size: restore still loads all shard text, and each save still visits
all entities and hashes/verifies large contents. Observer/Spark saves now use
incremental journal writes; lazy loading remains future work. Background deadlines are polled;
long blocking work and storage failures can extend the durability window.
