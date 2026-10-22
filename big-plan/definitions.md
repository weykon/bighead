# Definitions

This document fixes the terms used by the roadmap. These definitions are intentionally concrete so agents and humans can discuss the same system boundaries.

For the implementation baseline and remaining gaps, read [Runtime Model](runtime-model.md). Terms such as Job, Source, and App Package below describe architectural targets, not an assertion that a unified implementation already exists. A recorded component may represent an observation, inference, intent, or presentation state; persistence alone does not establish external truth.

## Runtime

The runtime is the long-lived process that owns the ECS world, tool dispatch, persistence, external I/O, and event loops. The runtime is not the model provider and not the frontend.

Responsibilities:

- maintain the ECS world,
- dispatch tools,
- create and persist logs,
- enforce context and tool-output boundaries,
- expose system state to agents and future UI surfaces,
- load app packages and system definitions.

## ECS World

The ECS world is the canonical state space. Facts, tasks, source records, shards, app records, transactions, and runtime metadata should be represented as entities and components when the agent needs to inspect or operate on them.

The world is not just a cache. It is the agent-readable system memory.

## Dynamic Component

A dynamic component is a schema-defined component that can be created at runtime. It is the basis for app-defined data models.

Rules:

- schemas must be discoverable,
- writes must validate required fields and JSON types,
- schema changes should become logged system actions.

## System

A system is executable behavior registered in the runtime. Current forms include built-in systems, Rhai systems, guide systems, and tools.

Systems are not only automation. They are also how the runtime exposes procedural capabilities to the agent.

## Tool

A tool is an agent-callable system boundary. Tool results are part of the API context path and must be subject to ContextGate.

Tools should do bounded mechanical work. Semantic decisions belong to the agent unless the system explicitly defines deterministic behavior.

## Spark

A Spark is a user or system intent waiting to be processed by the agent loop. It is currently a lightweight task queue item.

Long term, Spark should become one input source for a more formal Job model.

## Job

A Job is an executable unit of work with lifecycle and scheduling semantics:

```text
pending -> running -> done
pending -> running -> failed -> retrying
pending -> cancelled
```

Jobs should support priority, dependency, retry, deadline, owner, and assigned worker.

## Action Log

The Action Log is append-only history of what the system attempted and what actually changed.

It should answer:

- who or what initiated the change,
- what tool/system ran,
- what entity/component changed,
- what the before/after values were,
- whether the action succeeded,
- which transaction it belonged to.

## Transaction

A Transaction is a bounded group of related actions. First version: one agent cycle equals one transaction.

Current TransactionRecord is an audit grouping, not an atomic database transaction. A failed group does not roll back successful writes. Execution outcome, durable save, delivery acknowledgement, and business completion must be tracked separately; see [the runtime lifecycle](runtime-model.md#5-执行与持续活动).

Later versions can support nested transactions:

```text
user spark transaction
  -> run_system subtransaction
  -> tool call subtransaction
```

## Index

An Index is a derived access path over runtime data. Indexes let the agent find facts, shards, entities, sources, actions, and packages without scanning everything.

Important indexes:

- component/entity access through Bevy archetypes, world shape counts and bounded samples
  (no separate persistent membership list),
- shard/paradigm index,
- source/provenance index,
- full-text index,
- action/transaction index,
- time index.

## Source

A Source is an external data origin: URL, file, API, repository, feed, message thread, or connector object.

Source records should track:

- origin,
- fetched/synced time,
- content hash,
- source type,
- related shards,
- derived facts,
- verification status.

## Provenance

Provenance is the trace from a runtime fact back to the source and content region that produced it.

Example:

```text
Fact entity
  -> SourceRecord(url)
  -> ShardData(paradigm, shard_index)
  -> ActionRecord(fetch_url)
```

## App Package

An App Package is an installable bundle of system definitions:

```text
manifest
component schemas
systems/tools
guides
view definitions
seed data
indexes
policies
tests
migrations
```

Packages are how the system becomes an ecosystem instead of a one-off local world.

## Policy

Policy defines what is allowed, what needs approval, and what must be denied. It applies before actions execute.

Policy is broader than RBAC:

```text
actor + capability + target + context + risk + approval mode
```

## View Runtime

The View Runtime is the browser-facing UI layer. It renders ECS/app state and forwards user events back to the runtime. It should not define core system semantics.

Future UI should be built from declarative definitions and bindings, not only one-off generated panels.
