//! Dispatch logic for the `render_view` LLM tool.
//!
//! This module owns **all** runtime logic for turning a `render_view` tool call
//! into a [`ViewCommand`] on the [`ViewChannel`]. The `big-head-loop` crate only
//! needs to know that a `ViewToolHandler` exists and to call [`ViewToolHandler::dispatch`]
//! when the LLM invokes the `render_view` tool.
//!
//! Why this lives here, not in `big-head-loop`:
//! - The handler imports `ViewCommand` (the view protocol type). Keeping the
//!   import here means `big-head-loop` does not need to know any view-specific
//!   protocol types.
//! - Future view tools (e.g. `view_state`, `list_panels`, `ui_query`) can be
//!   added as additional methods on `ViewToolHandler` without touching
//!   `big-head-loop`.

use big_head_ai::{ToolCall, ToolDefinition};
use big_head_state::{AgentActivity, StateHook};

use crate::channel::ViewChannel;
use crate::display::{DisplayIntent, display_spec};
use crate::protocol::ViewCommand;
use crate::status::StatusCommand;

/// Returns the `ToolDefinition` for the `render_view` tool, suitable for
/// inclusion in the LLM's tool list.
///
/// Lives in `big-head-view` (not in `big-head-ai`) because the schema is
/// tied to the [`ViewCommand`] protocol. The AI crate only sees
/// `Vec<ToolDefinition>` — the view-specific schema is injected at wiring time.
pub fn render_view_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "render_view".into(),
        description: "Push a browser view panel through the external view server. \
                      Use this when your response contains structured data better shown as \
                      a Table / Chart / Card / Form / Inspector / A2UI panel than as markdown. \
                      If you tell the user that you updated, rendered, refreshed, or will update \
                      a browser view/mind-view, you MUST actually call this tool before your final \
                      reply. Describing an intended visual design is not a render update.\n\n\
                      Unified protocol:\n\
                      - `surface: spatial3d` — floating panel in the 3D canvas (default)\n\
                      - `surface: html2d` — flat DOM panel grid for ordinary UI/dashboard/form work\n\
                      - For Mind View, prefer the dedicated `render_mind_view` tool. The generic fallback is \
                        `id: mind-view` + `surface: html2d` + `kind: html`. Mind View is its own top-level tab. \
                        By default scripts/styles/canvas/svg are stripped from the readable layer. Only use \
                        `data-big-head-mind-runtime=\"raw\"` on html/body when you provide a complete Canvas/WebGL/Three.js \
                        visual runtime; raw runtime fills the Mind View page and must not carry long prose. \
                        Use normal html2d panels for readable text/tables/forms. All rendered UI must avoid overlapping text/buttons/charts; \
                        prefer normal flow, grid, or flex over absolute positioning.\n\
                      - `op: spawn`  — create a new panel\n\
                      - `op: update` — patch existing panel content. Preferred shape: \
                        {\"op\":\"update\",\"id\":\"mind-view\",\"patch\":{\"patch\":\"full\",\"content\":{\"kind\":\"html\",\"markup\":\"...\"}}}. \
                        Shorthand {\"op\":\"update\",\"id\":\"mind-view\",\"content\":{...}} is also accepted.\n\
                      - `op: move`   — animate panel to new position\n\
                      - `op: despawn` — remove a panel\n\
                      - `op: highlight` — pulse/outline/shake effect\n\n\
                      For ECS runtime panels, pass `intent` and let Rust choose the stable id, surface, \
                      kind, title, and position. Example: {\"op\":\"spawn\",\"intent\":\"inspect_tickactions\", \
                      \"content\":{\"kind\":\"inspector\",\"component\":\"TickAction\",\"entries\":[]}}. \
                      When `inspect_component_view` is available, prefer that tool for a single ECS component; \
                      it reads the world, bounds fields, derives status, and renders this contract in one call. \
                      Supported intents: inspect_tickactions, inspect_sparks, inspect_context, inspect_world, \
                      inspect_tools, inspect_view, visualize_relation, choose_next, track_work.\n\n\
                      `kind` (template-driven, LLM fills data):\n\
                      - `table`  → columns + rows\n\
                      - `chart`  → bar/line/pie/scatter series\n\
                      - `card`   → single KPI (label + value + unit + trend)\n\
                      - `form`   → collect user input\n\
                      - `html`   → escape hatch for complex layout\n\
                      - `scene3d`→ entity relationship graph\n\
                      - `a2ui`   → raw A2UI v0.9 adjacency list (full control)\n\n\
                      - `inspector` → bounded ECS component snapshot with summary, entity status, and fields. \
                        Use stable ids such as inspector-tickactions, inspector-sparks, inspector-context, \
                        inspector-tools, and inspector-view. Run guide_view_system before choosing one.\n\n\
                      Normal `kind: html` panels are rendered inside a Shadow DOM with sanitization and \
                      safe layout CSS. Your CSS cannot affect the app shell, and inline/scripted layout may be stripped. \
                      Prefer table/chart/card/form/a2ui for structured UI; use html only for rich text fragments.\n\n\
                      **Interaction handler** (a2ui components / form panels):\n\
                      Each component's `props` may carry a `handler` field that\n\
                      routes the click to either:\n\
                      - `{\"type\": \"rhai\", \"tool_id\": \"...\"}` — runs a Rhai\n\
                        system registered via `define_tool` directly, no LLM\n\
                        cycle. Use for deterministic actions (mark done, jump,\n\
                        delete).\n\
                      - default (no `handler`) — pushes a `view event` spark\n\
                        and waits for the LLM. Use when the agent should\n\
                        reason about the data in context.\n\n\
                      For `spatial3d`, `position` is 3D coordinates [x, y, z] in the floating scene. \
                      For `html2d`, `position` is accepted for protocol compatibility but ignored by \
                      the flat DOM renderer."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["spawn", "update", "move", "scale", "despawn", "highlight", "batch"],
                    "description": "View command operation."
                },
                "id": {
                    "type": "string",
                    "description": "Panel ID (unique). Required unless intent supplies a stable panel id. Use 'mind-view' for the dedicated programmable 2D WebGL/HTML surface."
                },
                "intent": {
                    "type": "string",
                    "enum": ["inspect_tickactions", "inspect_sparks", "inspect_context", "inspect_world", "inspect_tools", "inspect_view", "debug_view", "visualize_relation", "choose_next", "track_work"],
                    "description": "Optional stable display contract. Rust expands this into id/surface/kind/title/position before ViewCommand deserialization."
                },
                "surface": {
                    "type": "string",
                    "enum": ["spatial3d", "html2d"],
                    "description": "Where to render the panel. spatial3d is the existing 3D floating canvas; html2d is a flat DOM panel grid for normal UI. Defaults to spatial3d."
                },
                "kind": {
                    "type": "string",
                    "enum": ["table", "chart", "card", "form", "html", "scene3d", "a2ui", "inspector"],
                    "description": "Panel type: table (rows), chart (bar/line/pie), card (KPI), form (user input), html (raw markup), scene3d (3D graph), a2ui (raw A2UI format), inspector (typed ECS runtime snapshot)."
                },
                "title": {
                    "type": "string",
                    "description": "Panel title for spawn op."
                },
                "position": {
                    "type": "array",
                    "items": {"type": "number"},
                    "minItems": 3,
                    "maxItems": 3,
                    "description": "3D position [x, y, z] for spawn / move ops."
                },
                "rotation": {
                    "type": "array",
                    "items": {"type": "number"},
                    "minItems": 3,
                    "maxItems": 3,
                    "description": "Optional 3D rotation [rx, ry, rz]."
                },
                "duration_ms": {
                    "type": "integer",
                    "description": "Optional animation duration in ms (move / highlight)."
                },
                "content": {
                    "type": "object",
                    "description": "Panel content object. REQUIRED: a 'kind' field inside content to select the variant. For mind-view use {\"kind\":\"html\",\"markup\":\"<!doctype html>...\"}.",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["table", "chart", "card", "form", "html", "scene3d", "a2ui", "inspector"],
                            "description": "Content variant selector."
                        }
                    },
                    "required": ["kind"],
                    "examples": [
                        {"kind": "card", "label": "CPU", "value": 85, "unit": "%", "trend": {"direction": "up", "magnitude": 2.3}},
                        {"kind": "html", "markup": "<p>Hello</p>"},
                        {"kind": "table", "columns": ["A", "B"], "rows": [[1, 2], [3, 4]]},
                        {"kind": "chart", "chart_type": "line", "series": [{"name": "X", "data": [{"x": 0, "y": 1}]}]},
                        {"kind": "form", "fields": [{"name": "email", "label": "Email", "field_type": "text", "placeholder": "you@x.com"}]},
                        {"kind": "scene3d", "nodes": [{"entity_id": 1, "label": "A", "position": [0, 0, 0]}], "edges": [{"from": 1, "to": 2, "label": "relates"}]},
                        {"kind": "a2ui", "catalog_id": "default", "components": [{"id": "t1", "component": "Text", "text": "Hi"}, {"id": "btn1", "component": "Button", "label": "Click"}]},
                        {"kind": "inspector", "component": "TickAction", "summary": [{"label": "active", "value": 3, "tone": "success"}], "entries": [{"entity_id": 42, "label": "weather_query", "status": "healthy", "fields": [{"label": "remaining", "value": 120}]}]}
                    ]
                },
                "patch": {
                    "type": "object",
                    "description": "Required for update op unless top-level content is supplied. Full replace shape: {\"patch\":\"full\",\"content\":{\"kind\":\"html\",\"markup\":\"...\"}}. Data-model shape: {\"patch\":\"data_model\",\"path\":\"/rows/0/1\",\"value\":\"new\"}.",
                    "properties": {
                        "patch": {
                            "type": "string",
                            "enum": ["full", "data_model"],
                            "description": "Patch variant selector."
                        },
                        "content": {
                            "type": "object",
                            "description": "Required when patch is full."
                        },
                        "path": {
                            "type": "string",
                            "description": "JSON pointer path for data_model patch."
                        },
                        "value": {
                            "description": "New value for data_model patch."
                        }
                    },
                    "required": ["patch"]
                }
            },
            "required": ["op"]
        }),
    }
}

/// A narrow, reliable tool for replacing the dedicated Mind View.
///
/// `render_view` intentionally exposes the full panel protocol. That is useful
/// for arbitrary panels, but unnecessarily error-prone when the destination is
/// always the same HTML runtime. This tool keeps the model-facing contract to
/// the two values it actually owns: a title and markup.
pub fn render_mind_view_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "render_mind_view".into(),
        description: "Replace the dedicated Mind View tab with HTML. Prefer this over render_view whenever the target is mind-view. Pass only markup and an optional title/runtime mode; Rust supplies the stable panel id, html2d surface, html kind, position, and spawn/upsert command. The default runtime is raw, which enables scripts/canvas/WebGL inside the sandboxed full-page iframe. Use runtime=readable only for static HTML without scripts. A successful tool result means the command was published and persisted to ECS ViewState. Browser parse/runtime failures arrive asynchronously as a view event with type=mind_runtime_error; fix the reported source location and render again.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title shown above the Mind View. Defaults to Mind View."
                },
                "markup": {
                    "type": "string",
                    "minLength": 1,
                    "description": "HTML document or fragment to render. Canvas/WebGL code may include style and script tags."
                },
                "runtime": {
                    "type": "string",
                    "enum": ["raw", "readable"],
                    "description": "raw (default) runs the complete HTML in the sandboxed full-page iframe; readable strips executable visual elements and shows static content."
                }
            },
            "required": ["markup"],
            "additionalProperties": false
        }),
    }
}

/// Materialize the compact `render_mind_view` input as a canonical
/// [`ViewCommand::Spawn`] JSON value. Re-spawning the stable id is an upsert in
/// both the browser snapshot and ECS persistence, so callers do not need to
/// discover whether the panel already exists.
pub fn normalize_mind_view_input(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "render_mind_view input must be a JSON object".to_string())?;
    let markup = obj
        .get("markup")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "render_mind_view requires a non-empty string field 'markup'".to_string())?;
    let title = obj
        .get("title")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Mind View");
    let runtime = obj
        .get("runtime")
        .and_then(|value| value.as_str())
        .unwrap_or("raw");
    let markup = match runtime {
        "raw" => ensure_raw_mind_runtime(markup),
        "readable" => markup.to_string(),
        other => {
            return Err(format!(
                "render_mind_view runtime must be 'raw' or 'readable', got '{other}'"
            ));
        }
    };

    Ok(serde_json::json!({
        "op": "spawn",
        "id": "mind-view",
        "surface": "html2d",
        "kind": "html",
        "title": title,
        "position": [0.0, 0.0, 0.0],
        "content": {
            "kind": "html",
            "markup": markup,
        }
    }))
}

fn ensure_raw_mind_runtime(markup: &str) -> String {
    let lower = markup.to_ascii_lowercase();
    if lower.contains("data-big-head-mind-runtime=\"raw\"")
        || lower.contains("data-big-head-mind-runtime='raw'")
    {
        return markup.to_string();
    }

    if let Some(start) = lower.find("<html")
        && let Some(relative_end) = lower[start..].find('>')
    {
        let end = start + relative_end;
        let mut marked = String::with_capacity(markup.len() + 38);
        marked.push_str(&markup[..end]);
        marked.push_str(" data-big-head-mind-runtime=\"raw\"");
        marked.push_str(&markup[end..]);
        return marked;
    }

    format!(
        "<!doctype html><html data-big-head-mind-runtime=\"raw\"><head><meta charset=\"utf-8\"></head><body>{markup}</body></html>"
    )
}

/// Returns the `ToolDefinition` for lightweight ambient status surfaces such
/// as a macOS menu bar popover.
pub fn render_status_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "render_status".into(),
        description: "Update the lightweight Big Head status surface. \
                      Use this for always-visible task awareness: current work, \
                      progress, blocked/done state, and short descriptions. \
                      This is not a rich panel renderer and does not trigger an \
                      agent cycle; it only publishes state for external UI clients \
                      such as a macOS menu bar app.\n\n\
                      Operations:\n\
                      - `set_state`: replace the whole status snapshot\n\
                      - `upsert_item`: insert or replace one task/status item by id\n\
                      - `remove_item`: remove one item by id\n\
                      - `append_message`: append one chat transcript message\n\
                      - `upsert_document`: insert or replace one desktop Notes document\n\
                      - `remove_document`: remove one desktop Notes document\n\
                      - `clear`: clear all status items\n\n\
                      Desktop group hints: use `group: choice` for mutually \
                      exclusive user choices, `group: checklist` for small \
                      toggleable steps, and `group: progress` for related \
                      progress rows. Native desktop clients may render these \
                      groups with specialized controls. Choice clicks send \
                      `status_event` action `choose`; checklist rows send \
                      action `toggle`.\n\n\
                      Item state must be one of: pending, running, blocked, done, failed, hidden. \
                      Progress is a JSON number from 0.0 to 1.0, not a string."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["set_state", "upsert_item", "remove_item", "append_message", "upsert_document", "remove_document", "clear"]
                },
                "state": {
                    "type": "object",
                    "description": "Required for set_state.",
                    "properties": {
                        "title": {"type": "string"},
                        "subtitle": {"type": "string"},
                        "summary": {"type": "string"},
                        "items": {
                            "type": "array",
                            "items": {"$ref": "#/$defs/status_item"}
                        },
                        "messages": {
                            "type": "array",
                            "items": {"$ref": "#/$defs/status_message"}
                        },
                        "documents": {
                            "type": "array",
                            "items": {"$ref": "#/$defs/status_document"}
                        }
                    }
                },
                "item": {
                    "$ref": "#/$defs/status_item",
                    "description": "Required for upsert_item."
                },
                "id": {
                    "type": "string",
                    "description": "Required for remove_item."
                },
                "message": {
                    "$ref": "#/$defs/status_message",
                    "description": "Required for append_message."
                },
                "document": {
                    "$ref": "#/$defs/status_document",
                    "description": "Required for upsert_document."
                }
            },
            "required": ["op"],
            "$defs": {
                "status_item": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "title": {"type": "string"},
                        "state": {
                            "type": "string",
                            "enum": ["pending", "running", "blocked", "done", "failed", "hidden"]
                        },
                        "control": {
                            "type": "string",
                            "enum": ["item", "checkbox", "progress"],
                            "description": "Optional native menu bar control. item=clickable row, checkbox=toggle row, progress=progress row. Omit for default item."
                        },
                        "description": {"type": "string"},
                        "progress": {
                            "type": "number",
                            "minimum": 0.0,
                            "maximum": 1.0,
                            "description": "0.0 to 1.0. Omit when unknown."
                        },
                        "checked": {
                            "type": "boolean",
                            "description": "Checkbox value when control=checkbox."
                        },
                        "group": {
                            "type": "string",
                            "description": "Optional grouping key. Desktop clients understand `choice`, `checklist`, and `progress`; other values are treated as ordinary rows."
                        },
                        "priority": {"type": "integer"},
                        "updated_at": {
                            "type": "integer",
                            "description": "Unix timestamp seconds."
                        }
                    },
                    "required": ["id", "title", "state"]
                },
                "status_message": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "role": {
                            "type": "string",
                            "enum": ["user", "assistant", "system", "error"]
                        },
                        "text": {
                            "type": "string",
                            "description": "Markdown-like chat text."
                        },
                        "created_at": {
                            "type": "integer",
                            "description": "Unix timestamp seconds."
                        }
                    },
                    "required": ["id", "role", "text"]
                },
                "status_document": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "title": {"type": "string"},
                        "subtitle": {"type": "string"},
                        "text": {
                            "type": "string",
                            "description": "Markdown-like document text for desktop Notes mode."
                        },
                        "kind": {
                            "type": "string",
                            "description": "Optional document kind, e.g. note, summary, insight, source."
                        },
                        "references": {
                            "type": "array",
                            "description": "Optional source/entity/shard/topic references shown in desktop Notes mode.",
                            "items": {"$ref": "#/$defs/status_document_reference"}
                        },
                        "updated_at": {
                            "type": "integer",
                            "description": "Unix timestamp seconds."
                        }
                    },
                    "required": ["id", "title", "text"]
                },
                "status_document_reference": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "description": "Reference kind, e.g. entity, source, shard, topic, document."
                        },
                        "id": {"type": "string"},
                        "title": {"type": "string"},
                        "description": {"type": "string"}
                    },
                    "required": ["kind", "id", "title"]
                }
            },
            "examples": [
                {
                    "op": "upsert_item",
                    "item": {
                        "id": "resume-v5",
                        "title": "Resume v5 polish",
                        "state": "running",
                        "control": "progress",
                        "description": "Filling missing facts and checking PDF output",
                        "progress": 0.65,
                        "priority": 10
                    }
                },
                {
                    "op": "upsert_item",
                    "item": {
                        "id": "prep-outline",
                        "title": "Outline next slice",
                        "state": "pending",
                        "control": "checkbox",
                        "group": "checklist",
                        "checked": false,
                        "priority": 20
                    }
                }
            ]
        }),
    }
}

// Returns the prompt patch that teaches the LLM when to use the
// `render_view` tool. Caller is expected to append this to the system prompt
// at wiring time.
//
// Loaded from `prompts/view-agent.md` so the markdown (with embedded JSON
// examples containing quote characters) doesn't fight with Rust string syntax.
pub const VIEW_AGENT_PROMPT_PATCH: &str = include_str!("../prompts/view-agent.md");

/// Result of dispatching a `render_view` tool call: `(output_message, is_error)`.
pub type DispatchResult = (String, bool);

/// Owns the runtime glue for the `render_view` LLM tool.
///
/// Constructed by `AgentRuntime` (or its wiring layer) and passed into the
/// `RuntimeToolExecutor`. All view-specific dispatch logic stays inside this
/// struct — the executor's `match` arm just calls [`Self::dispatch`].
#[derive(Clone)]
pub struct ViewToolHandler {
    pub channel: Option<ViewChannel>,
    pub state_hook: Option<StateHook>,
}

impl ViewToolHandler {
    pub fn new(channel: Option<ViewChannel>, state_hook: Option<StateHook>) -> Self {
        Self {
            channel,
            state_hook,
        }
    }

    /// Dispatch a `render_view` tool call.
    ///
    /// Returns `(output, is_error)`. The executor pipes this into the standard
    /// `ToolCallResult` path; the LLM sees the output as a normal tool result.
    pub fn dispatch(&self, call: &ToolCall) -> DispatchResult {
        let input = normalize_view_command_input(call.input.clone());
        let cmd: Result<ViewCommand, _> = serde_json::from_value(input.clone());
        match cmd {
            Ok(view_cmd) => {
                if let Some(ref ch) = self.channel {
                    if ch.tx.send(view_cmd).is_err() {
                        return ("View channel closed; command was not delivered.".into(), true);
                    }
                    if let Some(ref h) = self.state_hook {
                        h.emit(AgentActivity::ActionApplied {
                            action: format!("RenderView({:?})", call.input.get("op")),
                        });
                    }
                    (
                        format!("View command sent: {:?}", call.input.get("op")),
                        false,
                    )
                } else {
                    (
                        "View server not running. Start it via AgentRuntime::set_view_channel \
                         + ViewServer::run_with_global_sink."
                            .into(),
                        true,
                    )
                }
            }
            Err(e) => {
                let received_keys: Vec<&str> = input
                    .as_object()
                    .map(|m| m.keys().map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                let content_keys: Vec<&str> = input
                    .get("content")
                    .and_then(|c| c.as_object())
                    .map(|m| m.keys().map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                let patch_keys: Vec<&str> = input
                    .get("patch")
                    .and_then(|c| c.as_object())
                    .map(|m| m.keys().map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                let hint = if !received_keys.contains(&"op") {
                    "top-level 'op' field is required (one of: spawn, update, move, scale, despawn, highlight, batch)"
                } else if input.get("op").and_then(|op| op.as_str()) == Some("update")
                    && !patch_keys.contains(&"patch")
                {
                    "update requires patch envelope: {\"patch\":{\"patch\":\"full\",\"content\":{\"kind\":\"html\",\"markup\":\"...\"}}}; shorthand top-level content is also accepted"
                } else if !content_keys.is_empty() && !content_keys.contains(&"kind") {
                    "content object is missing required 'kind' field (one of: table, chart, card, form, html, scene3d, a2ui, inspector)"
                } else {
                    "check that arrays use [...] not {...}, and all required fields are present"
                };
                (
                    format!(
                        "render_view: deserialize failed: {e}\n\
                         Received top-level keys: {received_keys:?}\n\
                         Content keys: {content_keys:?}\n\
                         Patch keys: {patch_keys:?}\n\
                         Hint: {hint}\n\
                         Correct mind-view update example: {{\"op\":\"update\",\"id\":\"mind-view\",\"patch\":{{\"patch\":\"full\",\"content\":{{\"kind\":\"html\",\"markup\":\"<h2>Hello</h2>\"}}}}}}\n\
                         Correct a2ui example: {{\"op\":\"spawn\",\"id\":\"p1\",\"surface\":\"html2d\",\"kind\":\"a2ui\",\"title\":\"Hi\",\"position\":[0,1.5,0],\"content\":{{\"kind\":\"a2ui\",\"catalog_id\":\"default\",\"components\":[{{\"id\":\"t1\",\"component\":\"Text\",\"text\":\"Hi\"}}]}}}}"
                    ),
                    true,
                )
            }
        }
    }

    /// Dispatch a `render_status` tool call.
    pub fn dispatch_status(&self, call: &ToolCall) -> DispatchResult {
        let cmd: Result<StatusCommand, _> = serde_json::from_value(call.input.clone());
        match cmd {
            Ok(status_cmd) => {
                if let Some(ref ch) = self.channel {
                    ch.send_status(status_cmd);
                    if let Some(ref h) = self.state_hook {
                        h.emit(AgentActivity::ActionApplied {
                            action: format!("RenderStatus({:?})", call.input.get("op")),
                        });
                    }
                    (
                        format!("Status command sent: {:?}", call.input.get("op")),
                        false,
                    )
                } else {
                    (
                        "View server not running. Start it with --view-server to publish status."
                            .into(),
                        true,
                    )
                }
            }
            Err(e) => {
                let keys: Vec<&str> = call
                    .input
                    .as_object()
                    .map(|m| m.keys().map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                (
                    format!(
                        "render_status: deserialize failed: {e}\n\
                         Received keys: {keys:?}\n\
                         Hint: use op=set_state/upsert_item/remove_item/append_message/clear. \
                         For upsert_item, item must contain id/title/state; progress must be a number 0.0..1.0."
                    ),
                    true,
                )
            }
        }
    }

    /// Trait-level entrypoint used by `ToolExecutor::render_view`.
    ///
    /// Quiet variant of [`Self::dispatch`] — bad input is logged but not returned
    /// as an error (preserves the previous trait-method behavior).
    pub fn render_view_trait(&self, view: serde_json::Value) {
        if let Some(ref ch) = self.channel {
            match serde_json::from_value::<ViewCommand>(normalize_view_command_input(view)) {
                Ok(cmd) => ch.send(cmd),
                Err(e) => log::warn!("render_view (trait): bad input: {e}"),
            }
        }
    }
}

/// Expand display intents and accepted shorthand into the canonical
/// [`ViewCommand`] JSON shape.
///
/// Callers that persist view state should persist this materialized value,
/// not the original tool input, so the browser snapshot and ECS `ViewState`
/// keep the same stable panel id and metadata.
pub fn normalize_view_command_input(mut input: serde_json::Value) -> serde_json::Value {
    let Some(obj) = input.as_object_mut() else {
        return input;
    };

    apply_display_intent(obj);

    if obj.get("op").and_then(|op| op.as_str()) != Some("update") {
        return input;
    }

    if !obj.contains_key("patch") {
        if let Some(content) = obj.remove("content") {
            obj.insert(
                "patch".into(),
                serde_json::json!({
                    "patch": "full",
                    "content": content,
                }),
            );
        }
        return input;
    }

    let top_level_content = obj.get("content").cloned();
    let Some(patch_obj) = obj.get_mut("patch").and_then(|patch| patch.as_object_mut()) else {
        return input;
    };
    if !patch_obj.contains_key("patch") && patch_obj.contains_key("content") {
        patch_obj.insert("patch".into(), serde_json::Value::String("full".into()));
    }
    if patch_obj.get("patch").and_then(|patch| patch.as_str()) == Some("full")
        && !patch_obj.contains_key("content")
    {
        if let Some(content) = top_level_content {
            patch_obj.insert("content".into(), content);
        }
    }
    input
}

fn apply_display_intent(obj: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(raw_intent) = obj.remove("intent") else {
        return;
    };
    let Ok(intent) = serde_json::from_value::<DisplayIntent>(raw_intent.clone()) else {
        // Leave invalid values in place so the deserialization error reports
        // the original input instead of silently choosing an arbitrary panel.
        obj.insert("intent".into(), raw_intent);
        return;
    };

    let spec = display_spec(intent);
    obj.entry("id")
        .or_insert_with(|| serde_json::json!(spec.panel_id));

    if obj.get("op").and_then(|op| op.as_str()) != Some("spawn") {
        return;
    }

    // `intent` is authoritative for panel identity and placement. Content is
    // still supplied by the caller because it is a bounded live ECS snapshot.
    obj.insert("id".into(), serde_json::json!(spec.panel_id));
    obj.insert("surface".into(), serde_json::json!(spec.surface));
    obj.insert("kind".into(), serde_json::json!(spec.kind));
    obj.insert("title".into(), serde_json::json!(spec.title));
    obj.entry("position")
        .or_insert_with(|| serde_json::json!([0.0, 0.0, 0.0]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use big_head_ai::ToolCall;
    use serde_json::json;

    fn call(input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: "render_view".into(),
            input,
        }
    }

    #[test]
    fn mind_view_input_materializes_stable_raw_panel() {
        let input = json!({
            "title": "Runtime map",
            "markup": "<!doctype html><html><body><canvas id=\"c\"></canvas></body></html>"
        });
        let normalized = normalize_mind_view_input(&input).expect("valid mind view input");

        assert_eq!(normalized["op"], "spawn");
        assert_eq!(normalized["id"], "mind-view");
        assert_eq!(normalized["surface"], "html2d");
        assert_eq!(normalized["kind"], "html");
        assert_eq!(normalized["title"], "Runtime map");
        assert!(
            normalized["content"]["markup"]
                .as_str()
                .unwrap()
                .contains("<html data-big-head-mind-runtime=\"raw\">")
        );

        serde_json::from_value::<ViewCommand>(normalized)
            .expect("materialized input must match the browser protocol");
    }

    #[test]
    fn mind_view_readable_runtime_does_not_add_raw_marker() {
        let normalized = normalize_mind_view_input(&json!({
            "markup": "<article>Readable</article>",
            "runtime": "readable"
        }))
        .expect("valid readable mind view");

        assert_eq!(normalized["title"], "Mind View");
        assert_eq!(
            normalized["content"]["markup"],
            "<article>Readable</article>"
        );
    }

    #[test]
    fn mind_view_input_rejects_empty_markup() {
        let error = normalize_mind_view_input(&json!({"markup": "  "}))
            .expect_err("empty markup should be rejected");
        assert!(error.contains("non-empty"));
    }

    #[test]
    fn dispatch_without_channel_returns_error() {
        let h = ViewToolHandler::new(None, None);
        // Minimal valid Spawn: op + id + kind + content.kind (PanelContent is internally tagged).
        let input = json!({
            "op": "spawn",
            "id": "p1",
            "kind": "card",
            "title": "t",
            "position": [0.0, 0.0, 0.0],
            "content": {"kind": "card", "label": "x", "value": 1}
        });
        let (out, is_err) = h.dispatch(&call(input));
        assert!(is_err);
        assert!(out.contains("not running"), "got: {out}");
    }

    #[tokio::test]
    async fn dispatch_with_channel_forwards_command() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "spawn",
            "id": "p1",
            "kind": "card",
            "title": "t",
            "position": [0.0, 0.0, 0.0],
            "content": {"kind": "card", "label": "x", "value": 1}
        });
        let (out, is_err) = h.dispatch(&call(input));
        assert!(!is_err, "expected ok, got: {out}");
        assert!(out.contains("spawn"), "got: {out}");

        // The sink must have received the command.
        let cmd = sink.recv_command().await.expect("sink got command");
        match cmd {
            ViewCommand::Spawn { id, .. } => assert_eq!(id, "p1"),
            _ => panic!("expected Spawn command in sink"),
        }
    }

    #[tokio::test]
    async fn dispatch_spawn_expands_display_intent_contract() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "spawn",
            "intent": "inspect_tickactions",
            "content": {
                "kind": "inspector",
                "component": "TickAction",
                "summary": [{"label": "active", "value": 2, "tone": "success"}],
                "entries": []
            }
        });

        let (out, is_err) = h.dispatch(&call(input));
        assert!(!is_err, "expected ok, got: {out}");

        let cmd = sink.recv_command().await.expect("sink got command");
        match cmd {
            ViewCommand::Spawn {
                id,
                surface,
                kind,
                title,
                position,
                content,
                ..
            } => {
                assert_eq!(id, "inspector-tickactions");
                assert_eq!(surface, crate::protocol::ViewSurface::Html2D);
                assert_eq!(kind, crate::protocol::PanelKind::Inspector);
                assert_eq!(title, "TickAction Inspector");
                assert_eq!(position, [0.0, 0.0, 0.0]);
                assert!(matches!(
                    content,
                    crate::protocol::PanelContent::Inspector { .. }
                ));
            }
            other => panic!("expected Spawn command in sink, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_update_uses_intent_panel_id_with_content_shorthand() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "update",
            "intent": "inspect_sparks",
            "content": {
                "kind": "inspector",
                "component": "Spark",
                "entries": []
            }
        });

        let (out, is_err) = h.dispatch(&call(input));
        assert!(!is_err, "expected ok, got: {out}");

        let cmd = sink.recv_command().await.expect("sink got command");
        match cmd {
            ViewCommand::Update { id, patch } => {
                assert_eq!(id, "inspector-sparks");
                assert!(matches!(
                    patch,
                    crate::protocol::ContentPatch::Full {
                        content: crate::protocol::PanelContent::Inspector { .. }
                    }
                ));
            }
            other => panic!("expected Update command in sink, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_update_accepts_top_level_content_shorthand() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "update",
            "id": "mind-view",
            "content": {"kind": "html", "markup": "<h2>Hello</h2>"}
        });

        let (out, is_err) = h.dispatch(&call(input));
        assert!(!is_err, "expected ok, got: {out}");

        let cmd = sink.recv_command().await.expect("sink got command");
        match cmd {
            ViewCommand::Update { id, patch } => {
                assert_eq!(id, "mind-view");
                match patch {
                    crate::protocol::ContentPatch::Full { content } => match content {
                        crate::protocol::PanelContent::Html { markup } => {
                            assert!(markup.contains("Hello"));
                        }
                        other => panic!("expected Html content, got {other:?}"),
                    },
                    other => panic!("expected full patch, got {other:?}"),
                }
            }
            other => panic!("expected Update command in sink, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_update_accepts_patch_content_without_variant() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "update",
            "id": "mind-view",
            "patch": {
                "content": {"kind": "html", "markup": "<p>ok</p>"}
            }
        });

        let (out, is_err) = h.dispatch(&call(input));
        assert!(!is_err, "expected ok, got: {out}");

        let cmd = sink.recv_command().await.expect("sink got command");
        match cmd {
            ViewCommand::Update { patch, .. } => match patch {
                crate::protocol::ContentPatch::Full { content } => match content {
                    crate::protocol::PanelContent::Html { markup } => {
                        assert!(markup.contains("ok"));
                    }
                    other => panic!("expected Html content, got {other:?}"),
                },
                other => panic!("expected full patch, got {other:?}"),
            },
            other => panic!("expected Update command in sink, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_update_accepts_full_patch_with_top_level_content() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "update",
            "id": "mind-view",
            "content": {"kind": "html", "markup": "<p>top-level content</p>"},
            "patch": {"patch": "full"}
        });

        let (out, is_err) = h.dispatch(&call(input));
        assert!(!is_err, "expected ok, got: {out}");

        let cmd = sink.recv_command().await.expect("sink got command");
        match cmd {
            ViewCommand::Update { patch, .. } => match patch {
                crate::protocol::ContentPatch::Full { content } => match content {
                    crate::protocol::PanelContent::Html { markup } => {
                        assert!(markup.contains("top-level content"));
                    }
                    other => panic!("expected Html content, got {other:?}"),
                },
                other => panic!("expected full patch, got {other:?}"),
            },
            other => panic!("expected Update command in sink, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_status_with_channel_forwards_command() {
        let (channel, sink) = ViewChannel::new();
        let h = ViewToolHandler::new(Some(channel), None);
        let input = json!({
            "op": "upsert_item",
            "item": {
                "id": "task",
                "title": "Task",
                "state": "running",
                "progress": 0.25
            }
        });
        let (out, is_err) = h.dispatch_status(&call(input));
        assert!(!is_err, "expected ok, got: {out}");
        assert!(out.contains("upsert_item"), "got: {out}");

        let cmd = sink
            .recv_status_command()
            .await
            .expect("sink got status command");
        match cmd {
            StatusCommand::UpsertItem { item } => {
                assert_eq!(item.id, "task");
                assert_eq!(item.progress, Some(0.25));
            }
            _ => panic!("expected UpsertItem status command"),
        }
    }

    #[test]
    fn dispatch_bad_input_returns_error() {
        let h = ViewToolHandler::new(None, None);
        // No "op" field — fails to deserialize as ViewCommand.
        let (out, is_err) = h.dispatch(&call(json!({"id": "p1"})));
        assert!(is_err);
        assert!(
            out.contains("top-level 'op' field is required"),
            "got: {out}"
        );
    }
}

#[cfg(test)]
mod _debug_serde {
    use crate::protocol::ViewCommand;

    #[test]
    fn test_a2ui_spawn() {
        let json = r#"{"op":"spawn","id":"p1","kind":"a2ui","title":"Hi","position":[0.0,1.5,0.0],"content":{"kind":"a2ui","catalog_id":"default","components":[{"id":"t1","component":"Text","text":"Hi"}]}}"#;
        let r: Result<ViewCommand, _> = serde_json::from_str(json);
        match r {
            Ok(_) => println!("A2UI Spawn OK"),
            Err(e) => panic!("A2UI Spawn ERR: {}", e),
        }
    }

    #[test]
    fn test_a2ui_update_full() {
        let json = r#"{"id":"rhai-button-demo","op":"update","patch":{"patch":"full","content":{"kind":"a2ui","catalog_id":"default","components":[{"id":"t1","component":"Text","text":"Hi"}]}}}"#;
        let r: Result<ViewCommand, _> = serde_json::from_str(json);
        match r {
            Ok(cmd) => println!("A2UI Update OK: {:?}", cmd),
            Err(e) => panic!("A2UI Update ERR: {}", e),
        }
    }
}
