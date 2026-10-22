# World Operation Loop v1

The runtime/world ownership decision is deliberate:

| Layer | Owns |
| --- | --- |
| Rust runtime | Dynamic ECS/Rhai mechanisms, permissions, typed query, audit retention, loop guards |
| ECS world | WeatherObservation, WorldClock, links, OperationPlan, AcceptanceCheck and their instances |
| Rhai systems | Derived time, observation projection, plan/check transitions and sparse scheduling |
| Agent/Laya | Open reasoning and bounded choices at declared decision gates |

`SessionContext` is already dynamic ECS data, although its legacy schema currently has a
Rust startup fallback. New domain schemas must not be added as Rust components. Compose a
separate dynamic component such as `ContextWeatherLink` onto the context entity.

The installation recipe and lifecycle contract is available to the Agent through
`run_system("guide_world_operation_loop")`. Definitions are intentionally applied through
the existing runtime `define_component`, `define_system` and TickAction tools so each world
can adopt, alter or remove them without rebuilding Big Head.

`query_ecs` is the typed field-query path. Full action history remains in
`outputs/action_log.jsonl`; checkpoints retain seven days of ActionRecord detail and ninety
days of TransactionRecord summaries by default. Set the matching retention environment
variable to `0` to preserve all ECS records.
