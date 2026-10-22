# Goal Loop Prompt

Use this prompt when asking an implementation agent to execute the Big Head system roadmap. Replace the bracketed fields before use.

```text
You are working in the Big Head repository.

Objective:
Implement [CYCLE/STAGE NAME] from `big-plan/three-cycle-roadmap.md`.

Before coding:
1. Read:
   - `big-plan/README.md`
   - `big-plan/definitions.md`
   - `big-plan/three-cycle-roadmap.md`
   - `big-plan/execution-guide.md`
2. Inspect the current code paths related to this stage.
3. Check `git status --short`.
4. Identify unrelated dirty files and do not revert or stage them.

Execution rules:
1. Keep the implementation scoped to [CYCLE/STAGE NAME].
2. Use existing crate boundaries and patterns before adding new abstractions.
3. Runtime boundaries belong in code, not only prompt text.
4. Agent-facing behavior must be reflected in tool descriptions or guides when the agent needs to use it.
5. Large tool results must remain compatible with ContextGate and ShardData.
6. ECS should expose system state that the agent needs to inspect.
7. Add tests that match the risk and blast radius.
8. Do not implement future stages unless required to finish this stage cleanly.

Expected deliverables:
- Code changes for [CYCLE/STAGE NAME].
- Focused tests.
- Any needed guide/tool-description updates.
- A short manual test recipe.
- A summary of intentionally deferred work.

Validation:
Run the narrowest relevant checks first, then a broader check if the stage touches shared runtime behavior.

Suggested checks:
- `cargo test -p big-head-ai tool_definitions` when tool schemas/guides change.
- `cargo test -p big-head-loop <module_or_test_name>` when runtime tools change.
- `cargo check -p big-head-loop` when shared runtime paths change.
- Frontend build only if UI code is changed.

Completion response:
Report:
1. What changed.
2. Where the core files are.
3. Which tests passed.
4. How the user can manually test it.
5. What remains out of scope.

Do not claim the whole roadmap is complete. Only complete [CYCLE/STAGE NAME].
```

## Cycle-Specific Prompt Fragments

### Cycle 1: Control Plane

Use when implementing Action Log, indexes, or Job/Spark normalization.

```text
Prioritize runtime observability and durable system state. The agent should be able to inspect what happened, what exists, and what work remains. Prefer append-only records and ECS-visible components before rollback or advanced scheduling.
```

### Cycle 2: App Plane

Use when implementing packages, source sync, or declarative UI contracts.

```text
Prioritize reusable app definitions. A capability should be loadable, discoverable, testable, and represented in registries/manifests rather than living only in prompt text or ad-hoc runtime state.
```

### Cycle 3: Ecosystem Plane

Use when implementing policy, worker model, or package distribution.

```text
Prioritize safety, repeatability, and ecosystem boundaries. Actions must remain transaction-logged. Workers and packages should use capabilities and policies instead of hidden trust.
```
