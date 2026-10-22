# Execution Guide

This guide describes how to execute each stage of the roadmap and how the user should test it.

## Working Rules

1. Keep changes stage-scoped.
2. Read current code before editing.
3. Prefer existing crate boundaries and tool patterns.
4. Do not replace ECS with an external database in the first pass.
5. Store new agent-visible concepts in ECS components and expose bounded query tools.
6. Add prompt/guide changes only when the agent needs to know how to use the new capability.
7. Treat frontend rendering as a surface, not the core system.

## Stage Completion Checklist

Each stage is complete only when:

- code compiles,
- focused tests pass,
- a manual runtime test recipe exists,
- agent-facing guide/tool descriptions are aligned,
- persistence/restart behavior is considered,
- action boundaries and context projection behavior are documented,
- known limitations are explicitly listed.

## Testing Levels

### Unit Tests

Use for:

- component schema serialization,
- action record creation,
- index update logic,
- job lifecycle transitions,
- manifest parsing,
- policy evaluation.

### Runtime Tests

Use for:

- a Spark becoming a transaction/job,
- tool calls creating ActionRecords,
- fetch/search creating SourceRecords,
- packages loading schemas/systems/guides,
- view events becoming jobs.

### Manual Tests

Each stage should include a short manual recipe:

```text
1. Start runtime.
2. Enter a Spark.
3. Observe terminal output.
4. Query ECS state.
5. Inspect outputs/*.jsonl or checkpoint.
6. Restart runtime if persistence is part of the stage.
```

## Cycle 1 Manual Acceptance Recipes

### Action Log / Transaction

1. Start the runtime.
2. Submit: "create a simple Todo component and one todo entity".
3. Query `TransactionRecord`.
4. Query `ActionRecord`.
5. Inspect `outputs/action_log.jsonl`.
6. Confirm the write action records tool name, input summary, status, and before/after where available.

### Indexes

1. Use `search` and `fetch_url` on a URL.
2. Trigger projection if content is large.
3. Query `SourceRecord`.
4. Query `ShardIndex`.
5. Confirm the shard can be traced back to the source.

### Job / Spark

1. Submit a Spark.
2. Confirm a Job entity is created.
3. Let the agent finish.
4. Confirm job status is `done` or `failed`.
5. Push a follow-up Spark and confirm it becomes a separate job.

## Cycle 2 Manual Acceptance Recipes

### App Package

1. Create or load a sample app package.
2. Confirm schemas and systems are registered.
3. Query App registry entities.
4. Run one app-provided guide/system.

### Source / Sync

1. Register a URL source.
2. Run a sync job.
3. Run the sync job again.
4. Confirm content hash and changed/unchanged state.
5. Confirm derived shards/facts link to source version.

### Declarative UI Contracts

1. Create a `ViewDefinition`.
2. Bind it to a query or entity.
3. Trigger a user/view event.
4. Confirm event becomes a job or view event record.

## Cycle 3 Manual Acceptance Recipes

### Policy Engine

1. Define a deny or ask policy for a write/destructive action.
2. Ask agent to perform that action.
3. Confirm policy evaluation occurs before mutation.
4. Confirm denied/blocked action is logged.

### Multi-Agent Worker

1. Register two workers with different capabilities.
2. Submit a research job.
3. Confirm it is assigned to the research worker.
4. Confirm all writes still go through transactions.

### Package Distribution / Self-Improvement

1. Export a sample package.
2. Reinstall it into a fresh world or test world.
3. Trigger a known failure.
4. Ask agent to inspect Action Log and propose guide/test changes.

## Commit Guidance

Commit at stage boundaries unless a stage is too large. Good commit messages:

```text
Add transaction action log foundation
Add source and shard indexes
Normalize Spark processing into jobs
Add app package manifest loader
Add source sync model
Add declarative view definitions
Add policy evaluation before actions
Add agent worker assignment model
Add package export workflow
```

Do not mix unrelated frontend experiments, vendor formatting, or generated runtime snapshots into roadmap commits.
