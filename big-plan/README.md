# Big Head System Plan

This folder defines the next architecture path for Big Head as a computer-system substrate, excluding future renderer work. The frontend is treated as a future system UI surface, not as the core system boundary.

## Documents

- [runtime-model.md](runtime-model.md): code-grounded runtime model (2026-09-19), object boundaries, execution/durability/delivery distinctions, and the next implementation order. Start here for the current baseline; proposals are explicitly separate from implemented behavior.
- [definitions.md](definitions.md): shared vocabulary and architectural boundaries.
- [three-cycle-roadmap.md](three-cycle-roadmap.md): three development cycles, each with three stages.
- [execution-guide.md](execution-guide.md): how to implement, test, and accept each stage.
- [goal-loop-prompt.md](goal-loop-prompt.md): a reusable prompt for an agent to execute this plan.

## Core Direction

Big Head should evolve from "an agent that can mutate an ECS world" into an agent-native runtime:

```text
ECS World
  + Action Log
  + Indexes
  + Job Queue
  + App Packages
  + Policy Engine
  + Source/Connector Graph
  + View Runtime
```

The runtime owns boundaries and persistence. The agent owns decisions. Prompt text should guide behavior, but durable system knowledge should live in registries, manifests, indexes, logs, schemas, and packages.

The existing character View demo is a concrete validation case for interaction lifecycle, alongside background TickAction observation. Using it to validate runtime contracts does not make renderer work the core architecture boundary. The roadmap below describes intended work, not a claim that every listed subsystem is implemented.

## Development Shape

The plan is split into 3 cycles:

1. **Control Plane**: Action Log / Transaction, indexes, Job/Spark normalization.
2. **App Plane**: App packages, source/provenance, declarative system UI contracts.
3. **Ecosystem Plane**: policy engine, multi-agent workers, package distribution and self-improvement loop.

Each cycle has 3 stages. Each stage should end with:

- code merged locally,
- tests passing,
- a short human test recipe,
- a brief note on what remains intentionally out of scope.
