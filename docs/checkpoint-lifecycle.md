# Checkpoint Lifecycle

## Temporary Shards

New runtime-created `context:` artifacts use namespace plus SHA-256 of the
complete content as their paradigm. Repeated identical content reuses a verified
complete group. The default retention period is seven days since creation or the
last dedicated shard read/reuse. Existing groups without a ShardRetention policy,
including legacy data and explicit user archives, remain permanent.

`set_shard_retention` explicitly enrolls an existing complete group and sets
`retention_secs` (86400..315360000) and `pinned`. Pinning prevents collection.
`inspect_shard_retention` returns a read-only report. Inspection checks at most
128 expired groups and reports at most 256 policies; unchecked is not eligible.

The runtime checks once per hour while polling background checkpoints. Checks
rotate to prevent referenced groups starving later candidates. Collection requires
an expired, unpinned policy, all expected shard indices exactly once, and no
detected reference. It removes only ShardData, preserving other entity components.
A persistent policy tombstone reports expiry explicitly on dedicated reads.
Tombstones themselves are retained. Collection and check progress mark the world
dirty and follow the normal background save schedule.

Reference tracking conservatively searches registered system definitions, serialized persistent components and
other shard contents for the full paradigm, including embedded JSON strings.
Policies and ProjectContext creation receipts are not retention roots. Other
history/audit references do protect content, so historical references may prevent
collection indefinitely. This is not a universal reference graph: external files,
nonpersistent memory and references encoded only as entity IDs are not tracked.
Pin groups needed by those consumers. Reads through generic queries do not renew
the access lease; get_shards/get_shard_content do. No legacy group was enrolled or
deleted as part of this implementation.

## Observer and Spark Journal

Format v3 consists of checkpoint.json, checkpoint.blobs and checkpoint.journal.
Critical delivery saves retain their immediate durability boundary. After the
baseline, they write changed entity upserts/deletions, changed schemas/systems,
and observer runtime state in a checksummed delta. Unchanged saves write nothing.
The current implementation still scans all entity metadata and verifies blob
content to find differences. It reduces disk write amplification, not that CPU
scan or resident ECS memory.

Frames carry generation, sequence and previous checksum. A synced atomic HEAD
records the committed sequence/hash; restore verifies every committed frame
before changing the world. Incomplete temporary files and frames beyond HEAD
are ignored. Missing committed frames, checksum mismatches or corrupt HEAD fail
restoration. The whole journal directory is a required recovery dependency;
deleting it or its HEAD can discard acknowledged deltas and is not supported.

128 frames or 32 MiB accumulated deltas trigger a full checkpoint on the next
delivery save. Full saves start a fresh generation. The first delivery save after
restore also makes a new baseline because runtime entity IDs are reallocated.
Explicit/exit/background full saves remain supported. Changed-base detection
rejects appending to another writer's manifest, but this is a single-writer store,
not a multi-process database. Stop the writer before backups or migration.

## File Reclamation and Backups

After a successful runtime full checkpoint, file GC protects blobs referenced by
the current manifest, same-directory backups named `checkpoint.json.*`, and
journals for their generations. It removes unreferenced hash-named blobs and old
generation journals/HEADs only after a 24-hour file-age grace period. Invalid
backup JSON or journal checksums block cleanup. Backup retention can intentionally
keep large bodies alive after logical collection. External backup locations are
not discovered automatically and require their own complete copy of sidecars.

Back up/move all three together. When renaming the JSON stem, rename both sidecar
directories. Legacy migration copies both sidecar directories before the manifest.
Older binaries cannot safely restore v3. Rebuild and restart to activate this
code; the development tests do not migrate or reclaim the live world.

## Validation

The selected workspace regression passed 333 tests with 13 ignored. After adding
registered-script reference protection and stricter GC version validation, the
affected save/loop library suites passed 227 tests with 11 ignored (one additional
reference test). `cargo check -p big-head --offline` and `git diff --check` passed.
Existing compiler warnings remain.

Coverage includes reference/pin/recent-access/legacy protection, incomplete groups,
cross-shard references, fair candidate rotation, content reuse, observer defer/ack
replay, entity changes/deletions, committed-tail loss/corruption rejection,
uncommitted-tail handling and journal compaction. A 2001-entity synthetic check
confirms a one-entity update leaves the base unchanged and writes a delta smaller
than 1/20 of the manifest; this is a byte-size assertion, not a live latency claim.
