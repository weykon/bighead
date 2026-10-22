# big-head

Clean ECS-based AI agent runtime — extracted from agent-ide as a minimal foundation.

## Workspace Structure

```
big-head/
  vendor/                        # Vendored bevy crates (0.19.0-dev)
    bevy_ecs/                    # Core ECS framework
    bevy_reflect/                # Runtime reflection
    bevy_ptr/, bevy_utils/, bevy_tasks/, bevy_platform/
    bevy_macro_utils/, bevy_reflect/derive/
  external/
    ai_api_provider/             # 22+ LLM providers, Tool Use API, retry logic, token budget
  crates/
    big-head-ecs/                # Pure ECS data types (schema, dynamic_component, components, system_def, registry)
    big-head-save/               # World snapshot serialization + persistence (checkpoint/restore)
    big-head-script/             # Rhai engine build + ECS bridge functions
    big-head-loop/               # Orchestration loop (AgentRuntime, tools, scripting re-export)
    big-head-ai/                 # Agent, prompts, tool definitions, API client
    big-head-query/              # String DSL with value filters
```

## Crate Dependency Direction

```
big-head-loop → big-head-script → big-head-save → big-head-ecs → bevy_ecs, bevy_reflect
              → big-head-ai → ai_api_provider
              → big-head-query
```

big-head-loop is the orchestration crate. Any code needing both SystemDefinition and prompt constants (ai) lives here.

## Core Concepts

- **AgentRuntime**: Owns ECS World, spark queue, component registry, system registry, Rhai engine
- **Spark**: Simple struct `{ description: String }` — task unit dispatched to agents
- **RuntimeToolExecutor**: Bridge between Anthropic tool-use protocol and ECS world
- **ComponentActionRegistry**: Maps component names to insert/patch/remove closures
- **DynamicComponentStore**: Runtime-defined component types via unsafe Bevy ComponentDescriptor
- **SystemDefinition/SystemRegistry**: Declarative system descriptions (Llm or RhaiScript)
- **AgentSubscription**: Filter — currently `respond_to_all: bool`, matches every spark

## Tool Inventory (12 ECS tools + dynamic Rhai tools)

**Read**: get_world_overview, query_entities, get_entity, get_component
**Write**: spawn_entity, modify_component, remove_component, despawn_entity
**Define**: define_component, define_system, run_system
**Other**: push_spark
**Dynamic**: Rhai scripts can register additional tools via `define_tool()` — visible to LLM alongside built-in tools

## Key Patterns

- **Upsert semantics**: all patch tools merge or insert
- **Disjoint field borrow**: `.take()` agent out of self, create executor from `&mut self.world` inside scoped block, drop borrows, then restore agent
- **Read-before-write**: enforced in system prompts
- **Rhai scripting**: `eval_with_world()` bridges Rhai scripts to live ECS world
- **Token budget**: `TokenBudget` in `ai_api_provider::tokens` tracks input tokens; agent loop checks before each API call, returns early on `BudgetStatus::Exceeded`

## Token Estimation

`ai_api_provider::tokens` provides:
- `model_limits(model)` — static per-model input/output token caps
- `estimate_tokens(text)` — fast heuristic (no deps)
- `TokenBudget` — runtime budget tracking with `Ok` / `Warning` / `Exceeded` status

## Build & Test

```bash
cargo check                        # Full workspace check
cargo test --workspace             # All tests
cargo test -p big-head-loop         # Loop crate only
cargo run                          # Root binary (placeholder)
```

## Rhai Engine

Built via `scripting::build_ecs_engine()`. Available in scripts:
`all_entities`, `entities_with`, `has_component`, `get_component`, `set_component`,
`remove_component`, `components_of`, `query`, `spawn_entity`, `despawn`, `log`, `timestamp`,
`define_component`, `define_system`, `define_tool`

`define_tool(id, description, source)` — registers a Rhai system as an LLM-visible tool.
The tool appears in the LLM's tool list and can be called like any built-in tool.
Tools are stored in `SystemRegistry` with `is_tool: true` and exported via `tool_definitions()`.

Engine is `Arc<rhai::Engine>` (requires `rhai` sync feature) for Send+Sync safety.

## Origin

Extracted from `~/Desktop/p/agent-ide` as "big-head" — a clean third iteration.
Stripped subsystems: persistence, lens registry, supervisor, memory context,
ASCII enhancement, LOD rendering, A2A protocol, GUI/TUI, CLI executor, meta tracking.
Kept: core ECS tools, Rhai scripting (minimal), dynamic components, system registry.

## Edition

Rust 2024 edition, resolver 3.
