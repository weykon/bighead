//! 内置 ECS 工具集的 JSON-schema [`ToolDefinition`]。
//! 发送给 LLM 的工具清单，让它知道有哪些工具可用以及如何调用。

use ai_api_provider::ToolDefinition;

/// 返回 Anthropic Tool Use API 的 ECS 工具定义。
pub fn ecs_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions=vec![
        ToolDefinition {
            name: "inspect_runtime".into(),
            description: "Inspect bounded runtime events, Spark queue positions, orphan Pending records and recovery attempts. Follow correlation_id from TickAction to its Sparks and observer receipts; IDs remain stable across restarts. Defaults to the latest 20 records.".into(),
            input_schema: serde_json::json!({"type":"object", "properties": {
                "correlation_id":{"type":"string"}, "source_id":{"type":"string"},
                "after_sequence":{"type":"integer", "minimum":0}, "limit":{"type":"integer", "minimum":1, "maximum":100}
            }}),
        },
        ToolDefinition {
            name: "inspect_transaction".into(),
            description: "Inspect exactly one TransactionRecord by tx_id, including only ActionRecords with that same tx_id and the source kind of its Sparks. Use this instead of attributing adjacent recent actions to a transaction.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{
                "tx_id":{"type":"string","minLength":1}
            },"required":["tx_id"],"additionalProperties":false}),
        },
        ToolDefinition {
            name: "inspect_project".into(),
            description: "List project files or read a file with bounded output, without shell scans. Directory searches skip vendor, target, node_modules and world snapshots; at most 5000 entries. Pass a narrower path when truncated. File offsets are zero-based line offsets; lines are capped at 500 chars and reads at 2 MiB. Use for source and outputs/run_*.log discovery before execute_cli.".into(),
            input_schema: serde_json::json!({"type":"object", "properties": {
                "path":{"type":"string"}, "name_contains":{"type":"string"},
                "offset":{"type":"integer", "minimum":0}, "limit":{"type":"integer", "minimum":1,"maximum":200}
            }}),
        },
        ToolDefinition {
            name: "get_world_overview".into(),
            description: "Get a high-level overview of the ECS world: component registry and entity index (IDs + component names, no values). Use this first to understand what exists.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "query_entities".into(),
            description: "Query entities by component criteria. Syntax: 'Health, Position where Enemy and !Dead', 'Health where WITH(Health) AND WITH(Enemy) AND WITHOUT(Dead)', or \"TickAction where TickAction.script contains 'weather_query'\". Filters use actual ECS membership, including dynamic markers; no manual index needed. Fetch names alone do not require all components. Use '*' to fetch all components. See run_system('guide_query_dsl') for component composition and Lens examples.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query_string": {
                        "type": "string",
                        "description": "Query string, e.g. 'Session where Active', '* where Group', or \"TickAction where TickAction.script contains 'weather_query'\""
                    }
                },
                "required": ["query_string"]
            }),
        },
        ToolDefinition {
            name: "get_entity".into(),
            description: "Get all component values for a single entity by its numeric ID.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "The entity's numeric ID"
                    }
                },
                "required": ["entity_id"]
            }),
        },
        ToolDefinition {
            name: "get_component".into(),
            description: "Get a single component's value on a specific entity.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "The entity's numeric ID"
                    },
                    "component": {
                        "type": "string",
                        "description": "Component name (e.g. 'Session', 'Group')"
                    }
                },
                "required": ["entity_id", "component"]
            }),
        },
        ToolDefinition {
            name: "spawn_entity".into(),
            description: "Create a new entity with the given components. Before using a dynamic component, inspect \
                          get_world_overview for its required fields and field types. Dynamic component values must \
                          include every schema field, must not include unknown fields, and must use real JSON types: \
                          number/integer fields are unquoted JSON numbers, boolean fields are true/false, and string \
                          fields are quoted strings."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "components": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "Component type name" },
                                "value": { "description": "Component value as JSON" }
                            },
                            "required": ["name", "value"]
                        },
                        "description": "Components to add to the new entity"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional human-readable description for the entity"
                    }
                },
                "required": ["components"]
            }),
        },
        ToolDefinition {
            name: "modify_component".into(),
            description: "Update a component on an existing entity. Supports partial updates (only specified fields change). \
                          For dynamic components, inspect get_world_overview for field types before writing; number/integer \
                          fields must be JSON numbers, boolean fields true/false, and string fields quoted strings. \
                          Do not use modify_component to edit ViewState/browser panels; use render_view instead."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "The entity's numeric ID"
                    },
                    "component": {
                        "type": "string",
                        "description": "Component type name"
                    },
                    "value": {
                        "description": "New value (full or partial JSON object)"
                    }
                },
                "required": ["entity_id", "component", "value"]
            }),
        },
        ToolDefinition {
            name: "update_tick_action".into(),
            description: "Safely update a TickAction Rhai script and schedule using optimistic concurrency. The runtime verifies the current script hash, validates Rhai syntax before mutation, preserves last_run, and records the before/after state in the action audit trail. Use enabled=false to pause automatic execution without deleting the action; use the script_hash supplied by a TickAction observation as expected_script_hash."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "Numeric TickAction entity ID"
                    },
                    "expected_script_hash": {
                        "type": "string",
                        "description": "Current script hash from the observation event"
                    },
                    "script": {
                        "type": "string",
                        "description": "Optional complete replacement Rhai script; omit to change only enabled or cooldown_secs"
                    },
                    "cooldown_secs": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Optional replacement cooldown; omitted preserves the current value"
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Optional automatic scheduling switch; false pauses this action while retaining its script and state"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Concrete evidence-based reason for this repair"
                    }
                },
                "required": ["entity_id", "expected_script_hash", "reason"]
            }),
        },
        ToolDefinition {
            name: "remove_component".into(),
            description: "Remove a component from an entity.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "The entity's numeric ID"
                    },
                    "component": {
                        "type": "string",
                        "description": "Component type name"
                    }
                },
                "required": ["entity_id", "component"]
            }),
        },
        ToolDefinition {
            name: "despawn_entity".into(),
            description: "Delete an entity and all its components.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "The entity's numeric ID"
                    }
                },
                "required": ["entity_id"]
            }),
        },
        ToolDefinition {
            name: "define_component".into(),
            description: "Define a brand-new component type at runtime. After defining, use it like \
                          any built-in component with spawn_entity / modify_component / get_entity. \
                          Schemas are flat objects of primitive field types: 'string', 'integer', \
                          'number', 'boolean', 'any'. Explicit schema={} defines a presence-only marker, attached with value={}; removing that component clears the marker. Reuse existing types when appropriate. Names must not collide with existing components.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Component name (e.g. 'Deadline')"
                    },
                    "schema": {
                        "type": "object",
                        "description": "Field name → type. Example: {\"date\": \"string\", \"urgent\": \"boolean\"}. Use {} for a marker with no fields; schema must be explicitly provided.",
                        "additionalProperties": {
                            "type": "string",
                            "enum": ["string", "integer", "number", "boolean", "any"]
                        }
                    }
                },
                "required": ["name", "schema"]
            }),
        },
        ToolDefinition {
            name: "undefine_component".into(),
            description: "Undefine (soft-delete) a runtime-defined component type.                           Removes the component name from the registry so it can no longer be discovered                           or instantiated. Built-in Rust components cannot be undefined.                           Note: bevy_ecs does not support true component type removal;                           this is a soft delete that makes the component unreachable.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Component name to undefine (e.g. 'SystemRule')"
                    }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "define_system".into(),
            description: "Define a new Rhai-scripted system at runtime. The script can directly \
                          manipulate the ECS world. For Rhai syntax, run run_system('guide_rhai_syntax') \
                          first; the source parameter is just the script body.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Unique system identifier (e.g. 'health_checker')"
                    },
                    "description": {
                        "type": "string",
                        "description": "Human-readable description of what the system does"
                    },
                    "source": {
                        "type": "string",
                        "description": "Rhai script source code. See run_system('guide_rhai_syntax') for syntax reference and available ECS functions."
                    },
                    "max_iterations": {
                        "type": "integer",
                        "description": "Iteration budget (default: 1)",
                        "minimum": 1,
                        "maximum": 100
                    },
                    "is_tool": {
                        "type": "boolean",
                        "description": "Whether this system should appear as a tool in subsequent LLM interactions (default: false)"
                    }
                },
                "required": ["id", "source"]
            }),
        },
        ToolDefinition {
            name: "define_tool".into(),
            description: "Register a Rhai script as an LLM-visible tool. The script will appear \
                          as a callable tool in subsequent interactions. This is a shorthand for \
                          define_system with is_tool=true and max_iterations=1.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Unique tool identifier (e.g. 'mark_task_done')"
                    },
                    "description": {
                        "type": "string",
                        "description": "Human-readable description of what the tool does. This appears in the LLM tool list."
                    },
                    "source": {
                        "type": "string",
                        "description": "Rhai script source code. See run_system('guide_rhai_syntax') for syntax reference."
                    }
                },
                "required": ["id", "description", "source"]
            }),
        },
        ToolDefinition {
            name: "run_system".into(),
            description: "Execute a registered Rhai-scripted system by its id, including built-in guide systems \
                          such as guide_rhai_syntax, guide_query_dsl, guide_search, guide_shard, \
                          guide_tick_action, guide_push_spark, guide_cli, guide_render_view, \
                          guide_ambient, guide_agent_modes, guide_view_system, guide_scene_world, and systems \
                          created with define_system. The script runs against the live ECS world and any \
                          log() output is returned as the tool result. Optional args is a JSON object exposed as the Rhai args map for this call.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The system id to execute. Built-in guides use ids like guide_shard; user systems use ids registered via define_system."
                    },
                    "args": {
                        "type": "object",
                        "description": "Per-call inputs exposed as Rhai args; omitted means an empty map"
                    }
                },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "execute_cli".into(),
            description: "Execute a shell command via `sh -c`. Use for file I/O, running scripts, \
                          building projects, or any shell operation. Commands run in the project \
                          directory by default. Prefer scoped workspace searches (`rg --files`, \
                          `rg <pattern> crates docs`, `find . ...`) and inspect ECS/action traces \
                          before searching. Whole-machine searches such as `find / ...`, \
                          `find /Users ...`, or recursive grep from `/` are blocked because they \
                          are slow and usually lose prior context. Output is truncated if too large \
                          — use the full_output_path to read complete output. Destructive commands \
                          (rm -rf /, mkfs, etc.) are blocked.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Working directory (defaults to project root)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 120, max: 600)",
                        "minimum": 1,
                        "maximum": 600
                    },
                    "description": {
                        "type": "string",
                        "description": "Human-readable description of what this command does"
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Run command in background (setsid-detached). Returns immediately with PID + log file path. Use for long-running processes like dev servers. Limitations: NOT subject to timeout_secs (the wrapper returns immediately); no live stdout/stderr — read the returned log_path to inspect progress; backgrounded process keeps running until it exits on its own."
                    },
                    "name": {
                        "type": "string",
                        "description": "Stable human-readable name for a background process, shown in the TUI Processes tab"
                    },
                    "purpose": {
                        "type": "string",
                        "description": "Why this long-running process is being started; recorded as an ECS fact"
                    },
                    "port": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 65535,
                        "description": "Optional TCP port the process is expected to serve"
                    },
                    "health_url": {
                        "type": "string",
                        "description": "Optional URL to use later for a health check"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "inspect_processes".into(),
            description: "Inspect agent-managed background processes. This is the authoritative ECS-backed registry for programs started through execute_cli; it includes state, PID, purpose, working directory, port, and log path. A persisted PID is rechecked by the runtime and is not assumed alive after restart.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "push_spark".into(),
            description: "Queue a follow-up task (spark) that will run automatically after the current \
                          cycle finishes. Use this to decompose large tasks into sequential steps. \
                          Each pushed spark becomes a new coordinator cycle. Limit: 2 sparks per cycle."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "A self-contained description of the follow-up task. Include entity IDs, component names, and any context the next cycle will need."
                    }
                },
                "required": ["description"]
            }),
        },
        ToolDefinition {
            name: "set_session_context".into(),
            description: "Set the ambient session context used as an Ambient Cue at the start of future \
                          agent cycles. Use only when the user explicitly sets the place/atmosphere or \
                          a preset asks for it; do not infer private details. This updates the singleton \
                          SessionContext ECS component."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "Manual/preset location label, e.g. 书房, 公园, 房间."
                    },
                    "weather": {
                        "type": "string",
                        "description": "Ambient weather/atmosphere, e.g. 雨后, 黄昏后, 晴."
                    },
                    "time_of_day": {
                        "type": "string",
                        "description": "Human-scale time label, e.g. 黎明, 上午, 下午, 黄昏, 夜晚."
                    },
                    "light": {
                        "type": "string",
                        "description": "Light quality, e.g. golden_hour, overcast, 夜灯."
                    },
                    "source": {
                        "type": "string",
                        "enum": ["user_manual", "preset", "auto_detected"],
                        "description": "Where this context came from. Default user_manual."
                    }
                },
                "required": ["location", "weather", "time_of_day", "light"]
            }),
        },
        ToolDefinition {
            name: "push_ambient_signal".into(),
            description: "Push one explicit sparse ambient event into ECS and queue a follow-up spark \
                          carrying its AmbientSignal reference. Use only for meaningful atmosphere shifts \
                          such as location change or a large time-of-day transition; never for every tool \
                          call or low-value tick. Human category may describe atmosphere only, never content \
                          such as keystrokes, speech transcript, or screen text."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "signal_id": {
                        "type": "string",
                        "description": "Optional stable id. Defaults to ambient-<uuid>."
                    },
                    "category": {
                        "type": "string",
                        "enum": ["natural", "human", "technical", "emotional"],
                        "description": "Signal category."
                    },
                    "wind": {
                        "type": "string",
                        "description": "Wind/air movement label, e.g. 静, 微风, 大风."
                    },
                    "birds": {
                        "type": "string",
                        "description": "Natural sound label, e.g. 安静, 雀仔声, 蝉鸣."
                    },
                    "light": {
                        "type": "string",
                        "description": "Light quality, e.g. golden_hour, 阴天, 夜灯."
                    },
                    "temperature": {
                        "type": "string",
                        "description": "Temperature feel, e.g. 凉, 暖, 热."
                    },
                    "intensity": {
                        "type": "number",
                        "minimum": 0,
                        "maximum": 1,
                        "description": "Signal intensity from 0.0 to 1.0."
                    },
                    "source": {
                        "type": "string",
                        "enum": ["user_action", "world_tick", "external"],
                        "description": "Event source. Default user_action."
                    },
                    "ttl_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 86400,
                        "description": "How long the signal remains active. Default 300."
                    },
                    "description": {
                        "type": "string",
                        "description": "Self-contained follow-up spark text for the agent cycle."
                    }
                },
                "required": ["category"]
            }),
        },
        ToolDefinition {
            name: "shard_content".into(),
            description: "Project large content into ShardData ECS components. This stores content blocks in the \
                          world so the agent can inspect them later and decide what facts, indexes, or relations \
                          to create. Infrastructure only splits and stores; it does not semantically summarize, \
                          extract facts, or run hidden LLM map jobs.\n\n\
                          Strategies:\n\
                          - overlap: overlapping windows (recommended for logs/text). Adjacent shards share N lines \
                            so context isn't lost at boundaries.\n\
                          - custom_ranges: exact line ranges specified by agent.\n\
                          - head_tail: keep head and tail intact, slice the middle.\n\
                          - custom: currently stores the content as one block with a split intent label.\n\n\
                          After calling, use get_shards or query_entities component=ShardData, then explicitly \
                          create any derived facts, indexes, summaries, or graph relations with normal ECS tools."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Content to shard directly. Use this to pass spark text, message history summary, or any large text the agent already has in context. Takes precedence over content_ref."
                    },
                    "content_ref": {
                        "type": "string",
                        "description": "Use '__last_tool_result' or 'paradigm:<existing group>'. A paradigm reference concatenates stored chunks in index order without removing overlap or adding separators; it does not reconstruct omitted source text. Historical tool_call_id is unsupported. Ignored if content is provided. Destination paradigm must be new."
                    },
                    "strategy": {
                        "type": "string",
                        "enum": ["overlap", "custom_ranges", "head_tail", "custom"],
                        "description": "Splitting strategy: overlap (overlapping windows), custom_ranges (exact line numbers), head_tail (head+tail intact, middle sliced), custom (natural language split description)."
                    },
                    "overlap_size": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "description": "Number of overlapping lines for overlap strategy. Default 50."
                    },
                    "ranges": {
                        "type": "array", "minItems": 1,
                        "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "integer", "minimum": 0}},
                        "description": "Required for custom_ranges: zero-based half-open line ranges [start,end]; start must be less than end and end within the source line count."
                    },
                    "head_lines": {"type": "integer", "minimum": 0, "description": "Head lines retained by head_tail. Default 50."},
                    "tail_lines": {"type": "integer", "minimum": 0, "description": "Tail lines retained by head_tail. Default 50."},
                    "paradigm": {
                        "type": "string",
                        "description": "Paradigm identifier for later querying shards. Example: 'tool:execute_cli:abc123'."
                    },
                    "custom_split_description": {
                        "type": "string",
                        "description": "Natural language description of how to split (required for custom strategy)."
                    }
                },
                "required": ["strategy", "paradigm"]
            }),
        },
        ToolDefinition {
            name: "get_shards".into(),
            description: "Retrieve all ShardData entities for a given paradigm, ordered by shard_index. \
                          Each shard contains stored content blocks; any semantic facts or indexes must be \
                          created explicitly by the agent."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paradigm": {
                        "type": "string",
                        "description": "The paradigm identifier used when creating the shards."
                    }
                },
                "required": ["paradigm"]
            }),
        },
        ToolDefinition {
            name: "get_shard_content".into(),
            description: "Read a bounded content window from one ShardData entity by paradigm and shard_index. \
                          Use this after get_shards or a Context Projection Notice when the agent needs to inspect \
                          the projected content progressively without loading the whole block into context."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paradigm": {
                        "type": "string",
                        "description": "The paradigm identifier used when creating or projecting the shard."
                    },
                    "shard_index": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Zero-based shard index to read."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Character offset within the shard content. Default 0."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 12000,
                        "description": "Maximum characters to return. Default 4000, maximum 12000."
                    }
                },
                "required": ["paradigm", "shard_index"]
            }),
        },
        ToolDefinition {
            name: "search".into(),
            description: "Unified multi-source search. Picks the right chain by `kind`:\n\
                          - `web`  → public web (non-CJK: Bing → Mojeek → DDG; CJK: Baidu → cn.bing → Mojeek → DDG)\n\
                          - `wiki` → Wikipedia (en or zh, auto-detected from query)\n\
                          - `code` → GitHub repos (sorted by stars)\n\
                          - `news` → Hacker News stories (tech community signal)\n\n\
                          Use this as your only search tool — single tool, one mental model, results \
                          uniformly tagged with `kind` so you can tell web/wiki/repo/story apart.\n\n\
                          Returns: `{ success, query, kind, provider_used, result_count, results: [SearchHit] }` \
                          where each hit has a `kind` tag (`web` / `wiki` / `repo` / `story`) so \
                          you can branch on shape without re-querying. Search results are index \
                          snippets, not full-page fetches; do not claim you read the source body \
                          unless another tool actually fetched it."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["web", "wiki", "code", "news"],
                        "description": "Which source category to query. `web` is general open web, \
                                        `wiki` is encyclopedic, `code` is GitHub repos, `news` is \
                                        Hacker News. Required."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30,
                        "description": "Max results to return (default 8, max 30)."
                    }
                },
                "required": ["query", "kind"]
            }),
        },
        ToolDefinition {
            name: "fetch_url".into(),
            description: "Read the body of one explicit http(s) URL, usually after `search` has found candidate URLs. \
                          This is the tool that turns a search result URL into readable text. It may use an optional \
                          crawl4ai-compatible backend when BIG_HEAD_CRAWL4AI_CMD is configured; otherwise it uses \
                          native HTTP plus lightweight HTML extraction. Returns `{ success, url, final_url, backend, \
                          title, content_type, text, links, truncated, bytes_downloaded }`. The returned text is still \
                          subject to context projection if it is large; use get_shards/get_shard_content if you receive \
                          a projection notice."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The http:// or https:// URL to fetch."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["readable", "raw"],
                        "description": "readable extracts text from HTML; raw returns the response body as text. Default readable."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Maximum characters of text to return. Default 8000, max 30000."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8388608,
                        "description": "Maximum response bytes to download. Default 2097152, max 8388608."
                    },
                    "include_links": {
                        "type": "boolean",
                        "description": "Whether to include up to 50 extracted links for HTML pages. Default true."
                    },
                    "prefer_crawl4ai": {
                        "type": "boolean",
                        "description": "Use BIG_HEAD_CRAWL4AI_CMD backend when configured, falling back to native on failure. Default true."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 60,
                        "description": "Request/backend timeout in seconds. Default 20, max 60."
                    }
                },
                "required": ["url"]
            }),
        },
    ];
    for tool in &mut definitions {
        if matches!(
            tool.name.as_str(),
            "get_world_overview"
                | "query_entities"
                | "query_ecs"
                | "get_entity"
                | "get_component"
                | "get_shards"
        ) {
            let properties = tool.input_schema["properties"].as_object_mut().unwrap();
            properties.insert("output_offset".into(),serde_json::json!({"type":"integer","minimum":0,"description":"Character offset into this read result; default 0."}));
            properties.insert("output_limit".into(),serde_json::json!({"type":"integer","minimum":1,"maximum":8000,"description":"Maximum returned characters; default 4000. Large results are paged, not archived."}));
            properties.insert("expected_sha256".into(),serde_json::json!({"type":"string","description":"Pass the previous page sha256 to detect changes while paging."}));
        }
        if tool.name == "get_shard_content" {
            tool.input_schema["properties"]["entity_id"] = serde_json::json!({"type":"integer","minimum":0,"description":"Optional exact entity ID from inspect_shards/get_shards, required when a legacy group has duplicate indices. Must match paradigm and shard_index."});
        }
    }
    definitions
}

#[cfg(test)]
mod tests {
    #[test]
    fn shard_schema_exposes_strategy_inputs_and_existing_group_references() {
        let definition = super::ecs_tool_definitions()
            .into_iter()
            .find(|d| d.name == "shard_content")
            .unwrap();
        let props = &definition.input_schema["properties"];
        for name in ["ranges", "head_lines", "tail_lines", "content_ref"] {
            assert!(props.get(name).is_some(), "{name}");
        }
        assert_eq!(props["ranges"]["items"]["minItems"], 2);
        assert_eq!(props["ranges"]["items"]["maxItems"], 2);
        assert!(
            props["content_ref"]["description"]
                .as_str()
                .unwrap()
                .contains("paradigm:")
        );
    }
    use super::*;

    #[test]
    fn test_tool_definitions_valid_json_schema() {
        let tools = ecs_tool_definitions();
        assert_eq!(tools.len(), 27);
        for tool in &tools {
            assert!(!tool.name.is_empty());
            assert!(!tool.description.is_empty());
            assert_eq!(tool.input_schema["type"], "object");
            assert!(tool.input_schema.get("properties").is_some());
        }
    }

    #[test]
    fn test_all_defined_tools_have_dispatch() {
        let tools = ecs_tool_definitions();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        // Note: `render_view` is no longer in this list — it lives in
        // `big_head_view::render_view_tool_definition()` and is injected at
        // wiring time so the AI crate stays view-agnostic.
        for expected in [
            "inspect_runtime",
            "inspect_project",
            "get_world_overview",
            "query_entities",
            "get_entity",
            "get_component",
            "spawn_entity",
            "modify_component",
            "update_tick_action",
            "remove_component",
            "despawn_entity",
            "define_component",
            "define_system",
            "define_tool",
            "run_system",
            "execute_cli",
            "push_spark",
            "set_session_context",
            "push_ambient_signal",
            "shard_content",
            "get_shards",
            "get_shard_content",
            "search",
            "fetch_url",
        ] {
            assert!(names.contains(&expected), "missing tool: {expected}");
        }
    }
}
