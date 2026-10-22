# Three-Cycle Roadmap

The roadmap uses 3 development cycles. Each cycle contains 3 stages. A cycle is complete only when its stages are implemented, tested, and usable by the agent through tools/guides or runtime state.

## Cycle 1: Control Plane

Goal: make the runtime know what it changes, find what it stores, and schedule work with clear lifecycle.

### Stage 1.1: Action Log / Transaction

Objective:

Create an append-only audit layer for agent cycles and tool calls.

Minimum deliverables:

- `TransactionRecord` component.
- `ActionRecord` component.
- JSONL writer at `outputs/action_log.jsonl`.
- One transaction per agent cycle.
- Every tool call records an action-level audit entry.
- Write tools progressively record before/after values.
- ContextGate projection records `ProjectContext` actions.

Acceptance:

- User submits a Spark.
- Runtime creates a transaction.
- Tool calls are visible in ECS and JSONL.
- Failed tool calls are logged with `is_error = true`.
- Agent can query recent transactions from ECS.

Out of scope:

- true rollback,
- database transaction isolation,
- multi-user audit identity.

### Stage 1.2: World Shape / Shard / Source Access

Objective:

Give the agent stable access paths for world data and projected content.

Minimum deliverables:

- Expose Bevy archetype membership through read-only world shape counts and bounded samples;
  reuse Query and Lens for values instead of maintaining a separate `EntityIndex` entity list.
- `ShardIndex` keyed by `paradigm`, `source_label`, and created time.
- `SourceRecord` for URL/file/API origins.
- `ProvenanceLink` from facts/shards/actions to source records.
- Query tools for source and shard indexes.

Acceptance:

- World shape filters agree with Query membership; paging and sample limits are explicit.
- Fetching a URL creates or updates a `SourceRecord`.
- Large fetched content can be projected to shards and found by source.
- Agent can answer: "which source produced this fact or shard?"

Out of scope:

- vector search,
- advanced ranking,
- distributed index storage.

### Stage 1.3: Job / Spark Normalization

Objective:

Turn Spark from a loose queue into a formal job input and lifecycle system.

Minimum deliverables:

- `Job` component with status, priority, retry count, deadline, source.
- Spark ingestion creates or links a Job.
- Agent cycle processes a bounded job scope.
- Failed jobs record failure reason and retry policy.
- View events and TickAction events can become jobs.

Acceptance:

- New user input becomes a pending job.
- Running the loop moves job status through running/done or failed.
- Queued follow-up work is visible and queryable.
- Old Spark behavior remains compatible during migration.

Out of scope:

- multi-worker scheduling,
- distributed job leases,
- full cron scheduler redesign.

## Cycle 2: App Plane

Goal: make runtime capabilities reusable as installable app modules with source-backed facts and declarative UI contracts.

### Stage 2.1: App Package

Objective:

Define and load app bundles.

Minimum deliverables:

- `big-head-app.json` manifest format.
- Package directory layout.
- Loader for component schemas, guides, systems, tools, seed data.
- App registry components in ECS.
- Package tests or validation command.

Acceptance:

- A sample `research` package can be loaded.
- It registers schemas, guide docs, and optional systems.
- App state is discoverable by agent through ECS/tools.

Out of scope:

- remote package marketplace,
- package signing,
- dependency solver.

### Stage 2.2: Source / Sync / Connector Model

Objective:

Move from one-off fetches to source records and repeatable sync.

Minimum deliverables:

- `ConnectorDefinition`.
- `SyncState`.
- source sync job type.
- fetch/search results linked to `SourceRecord`.
- change detection via content hash.

Acceptance:

- A URL source can be synced twice.
- Runtime records whether content changed.
- Derived shards/facts keep provenance to the source version.

Out of scope:

- OAuth connectors,
- browser automation,
- bidirectional sync writes.

### Stage 2.3: Declarative System UI Contracts

Objective:

Prepare the future frontend as a system UI surface without making rendering the core.

Minimum deliverables:

- `ViewDefinition`.
- `DataBinding`.
- `ActionBinding`.
- `LayoutState`.
- event-to-job bridge.
- app package can include default view definitions.

Acceptance:

- Agent can create a declarative view over ECS data.
- User events flow back as jobs/view events.
- View definitions survive restart.

Out of scope:

- full design system,
- arbitrary React app generation,
- advanced renderer implementation.

## Cycle 3: Ecosystem Plane

Goal: make the runtime safe, extensible, multi-worker capable, and able to improve itself through logged evidence.

### Stage 3.1: Policy Engine

Objective:

Introduce capability and approval checks before risky actions execute.

Minimum deliverables:

- `Capability`.
- `PolicyRule`.
- tool/action risk classification.
- approval modes: allow, ask, deny.
- transaction policy evaluation record.

Acceptance:

- Low-risk read tools execute automatically.
- Risky write/destructive tools can require approval.
- Denied actions are logged as failed/blocked ActionRecords.

Out of scope:

- enterprise RBAC,
- multi-tenant admin UI,
- cryptographic identity.

### Stage 3.2: Multi-Agent Worker Model

Objective:

Split work by job type and capability instead of by ad-hoc chat roles.

Minimum deliverables:

- `AgentWorker` component.
- worker capability declaration.
- job assignment rules.
- worker-specific guide/tool exposure.
- verifier worker prototype.

Acceptance:

- Research jobs can be assigned separately from write/execution jobs.
- Worker output is committed through normal transactions.
- Verifier can inspect action/source provenance and create review jobs.

Out of scope:

- distributed worker cluster,
- complex negotiation protocols,
- hidden cross-agent chats.

### Stage 3.3: Package Distribution and Self-Improvement Loop

Objective:

Make apps reusable and let the system improve guides/packages through evidence.

Minimum deliverables:

- local package install/list/update commands or tools.
- package migration records.
- guide/test update workflow using transactions.
- failure pattern reports from Action Log.
- sample package export/import.

Acceptance:

- A local app package can be exported and reinstalled.
- Agent can inspect failed actions and propose guide/test changes.
- System changes are tested before commit.

Out of scope:

- public marketplace,
- automatic self-modification without review,
- remote distributed sync.

## Cross-Cycle Rule

Every stage must preserve the core principle:

```text
agent decides
runtime enforces boundaries
ECS stores state
logs store history
indexes make state discoverable
packages make capabilities reusable
policies make actions safe
views make state interactive
```
