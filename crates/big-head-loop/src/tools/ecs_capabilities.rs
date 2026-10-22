//! Discover actual ECS capabilities and hot-edit registered Rhai definitions.

use bevy_ecs::world::World;
use big_head_ai::{AgentAction, ToolCall, ToolDefinition};
use big_head_ecs::{ComponentActionRegistry, SystemDefinition, SystemKind, SystemRegistry};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::scripting;

#[cfg(test)]
#[path = "ecs_capabilities_tests.rs"]
mod tests;

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "inspect_ecs_capabilities".into(),
            description: "Read the live ECS operation model, component catalog or SystemRegistry (not ECS entities). Views: operations/components/systems. Use exact name for a tool contract, dynamic schema or paged system source and definition_hash. Lists are sorted and paginated. Registration does not mean scheduled/running. No component values or shard bodies are read.".into(),
            input_schema: json!({"type":"object","properties":{
                "view":{"type":"string","enum":["operations","components","systems"],"default":"operations"},
                "name":{"type":"string","description":"Exact registered name/id; omit to list"},
                "expected_definition_hash":{"type":"string","description":"Optional systems-detail guard, use across source pages"},
                "prefix":{"type":"string"},
                "offset":{"type":"integer","minimum":0},
                "limit":{"type":"integer","minimum":1,"maximum":32},
                "source_offset":{"type":"integer","minimum":0,"description":"Zero-based Unicode character offset, systems detail only"},
                "source_limit":{"type":"integer","minimum":1,"maximum":2000}
            },"additionalProperties":false}),
        },
        ToolDefinition {
            name: "update_system".into(),
            description: "Hot-replace an existing user Rhai system/tool, retaining its id and unspecified metadata. Inspect systems first; expected_definition_hash guards the complete definition. Syntax is checked before replacement; no script runs and no ECS state is migrated. Effective on the next invocation. Built-in guides and LLM stages cannot be replaced. Verify behavior separately with run_system and actual ECS observations.".into(),
            input_schema: json!({"type":"object","properties":{
                "id":{"type":"string"},"expected_definition_hash":{"type":"string"},
                "source":{"type":"string","description":"Complete replacement Rhai source"},
                "description":{"type":"string"},"is_tool":{"type":"boolean"},
                "reason":{"type":"string"}
            },"required":["id","expected_definition_hash","source","reason"],"additionalProperties":false}),
        },
        ToolDefinition {
            name: "remove_system".into(),
            description: "Remove a user Rhai system from SystemRegistry and future dynamic tool lists using its current definition_hash. Rejects built-ins and explicit tool_names dependents. Does not erase prior ECS effects, TickActions, audit records or references inside free-form code/text. Inspect and repair callers before removal; dependency checking is not whole-program analysis.".into(),
            input_schema: json!({"type":"object","properties":{
                "id":{"type":"string"},"expected_definition_hash":{"type":"string"},
                "reason":{"type":"string"}
            },"required":["id","expected_definition_hash","reason"],"additionalProperties":false}),
        },
    ]
}

fn default_view() -> String {
    "operations".into()
}
fn default_limit() -> usize {
    8
}
fn source_limit() -> usize {
    2000
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectRequest {
    #[serde(default = "default_view")]
    view: String,
    name: Option<String>,
    expected_definition_hash: Option<String>,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    source_offset: usize,
    #[serde(default = "source_limit")]
    source_limit: usize,
}

pub(crate) fn definition_hash(def: &SystemDefinition) -> String {
    // SystemDefinition has no unordered maps; include metadata, not just source.
    big_head_save::persistence::digest(&serde_json::to_vec(def).expect("serializable definition"))
}

pub(crate) fn protected(def: &SystemDefinition) -> bool {
    static GUIDES: std::sync::LazyLock<std::collections::HashSet<String>> =
        std::sync::LazyLock::new(|| {
            big_head_ai::all_builtin_guides()
                .into_iter()
                .map(|guide| guide.id)
                .collect()
        });
    matches!(def.kind, SystemKind::Llm) || GUIDES.contains(&def.id)
}

fn dependents(registry: &SystemRegistry, id: &str) -> Vec<String> {
    let mut result: Vec<_> = registry
        .iter()
        .filter(|def| def.id != id && def.tool_names.iter().any(|name| name == id))
        .map(|def| def.id.clone())
        .collect();
    result.sort();
    result
}

// These are execution semantics, not model-generated claims about task success.
fn operation(name: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match name {
        "inspect_activity" => (
            "observe_activity",
            "Optional run_id or scope; omit to list",
            "Inspect plan, revision, records and acceptance; Spark completion is not activity completion",
        ),
        "update_activity_plan" => (
            "revise_future_workflow",
            "Inspect current revision; pause active work; preserve completed steps; uncertain effects require recheck",
            "Read back revised plan and unchanged Draft/Paused/Blocked status. Updating never starts or resumes work",
        ),
        "create_activity" | "control_activity" => (
            "manage_activity",
            "Read guide_activity via systems source; local authorized execution only",
            "Create produces a draft. Explicit start/resume controls execution; completion requires independent acceptance checks",
        ),
        "laya_decide" | "laya_assess" => (
            "recommend_operation",
            "Local service running; compact observed state and available candidate tools",
            "Advisory only; confidence is not ECS-calibrated; verify any separately executed operation",
        ),
        "laya_run_spec" => (
            "bounded_decide_execute_verify",
            "Versioned DecisionSpec, compact observed state, local Laya service, and currently available declared tools",
            "Execute is available by default but still requires deterministic gates, a bounded existing action, and independent read-back",
        ),
        "inspect_laya_shadow" => (
            "observe",
            "No model call required",
            "Check drops, errors, skips and writer status; enabled is not proof of successful inference",
        ),
        "record_fact" => (
            "append_evidence",
            "Bounded observed/verified occurrence with explicit source",
            "Advice and verdicts belong in record_interpretation; corrections are new facts, not rewrites",
        ),
        "record_interpretation" => (
            "append_frame_scoped_reading",
            "Explicit frame and 1..32 existing observed/verified FactRecord ids",
            "Inspect the derived result and evidence; identical calls reuse the result and do not advance observation",
        ),
        "materialize_interpretation" => (
            "derive_frame_scoped_result",
            "Existing Lens plus explicit task frame and valid relation pointers",
            "Inspect result provenance and cache status; derived output is not an objective fact",
        ),
        "select_interpretation_path" => (
            "recommend_interpretation_path",
            "Local Laya service, explicit frame and 1..5 existing Lens ids",
            "Advisory only; explicitly materialize the selected Lens or escalate to the main agent",
        ),
        "inspect_interpretations" => (
            "observe_derivations",
            "Optional known result id",
            "Inspect frames, runs, source versions, relations and active cache state",
        ),
        "inspect_ecs_capabilities" => (
            "observe",
            "Choose a catalog and use exact registered names",
            "Check page coverage and live definition hashes",
        ),
        "inspect_world_shape" => (
            "observe",
            "Use existing component names",
            "Counts describe composition, not business correctness",
        ),
        "query_entities" | "query_ecs" | "get_entity" | "get_component" => (
            "observe",
            "Discover names and entity IDs before reading",
            "Values describe the world at this call, not all future states",
        ),
        "define_component" => (
            "register_schema",
            "New noncolliding name and explicit flat schema, {} for marker",
            "Inspect components; definition alone creates no entity",
        ),
        "undefine_component" => (
            "retire_schema",
            "Dynamic type with zero remaining instances",
            "Name removed; Bevy registration is retained, not reusable",
        ),
        "spawn_entity" => (
            "create_entity",
            "All component types registered and values schema-valid",
            "Read the returned entity and confirm intended composition",
        ),
        "modify_component" => (
            "upsert_component",
            "Known entity; partial update if present, full value if absent",
            "Read value and inspect composition; adding a component moves archetype",
        ),
        "remove_component" => (
            "remove_component",
            "Known entity and component",
            "Inspect composition; entity remains alive",
        ),
        "despawn_entity" => (
            "delete_entity",
            "Known entity, check callers/references first",
            "Entity no longer exists; external references are not automatically repaired",
        ),
        "define_system" | "define_tool" => (
            "register_behavior",
            "New system id and syntactically valid Rhai",
            "Inspect systems; registered is not executed or periodically scheduled",
        ),
        "update_system" => (
            "replace_behavior",
            "User Rhai system, current definition hash, valid new source",
            "Inspect new hash then separately execute and verify intended effects",
        ),
        "remove_system" => (
            "remove_behavior",
            "User Rhai system, current hash, no tool_names dependents",
            "Inspect registry; previous effects and free-form references remain",
        ),
        "run_system" => (
            "execute_behavior",
            "Existing Rhai system id; scripts can mutate ECS",
            "Tool success alone is not task success; inspect the intended output",
        ),
        "update_tick_action" => (
            "replace_scheduled_behavior",
            "Existing TickAction, expected script hash, valid Rhai",
            "Observe actual later executions in inspect_runtime/TickActionHealth",
        ),
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) fn inspect(
    world: &World,
    components: &ComponentActionRegistry,
    systems: &SystemRegistry,
    available_tools: &[ToolDefinition],
    input: &Value,
) -> Result<Value, String> {
    inspect_authorized(world, components, systems, available_tools, None, input)
}

pub(crate) fn inspect_authorized(
    world: &World,
    components: &ComponentActionRegistry,
    systems: &SystemRegistry,
    registered_tools: &[ToolDefinition],
    allowed_tools: Option<&[String]>,
    input: &Value,
) -> Result<Value, String> {
    let r: InspectRequest = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if !(1..=32).contains(&r.limit) || !(1..=2000).contains(&r.source_limit) {
        return Err("limit must be 1..32; source_limit must be 1..2000".into());
    }
    if let Some(expected) = &r.expected_definition_hash {
        if r.view != "systems" || r.name.is_none() {
            return Err("expected_definition_hash requires systems view and exact name".into());
        }
        let def = systems
            .get(r.name.as_deref().unwrap())
            .ok_or("system not found")?;
        if definition_hash(def) != *expected {
            return Err("definition changed while reading; inspect systems again".into());
        }
    }
    let matches =
        |name: &str| name.starts_with(&r.prefix) && r.name.as_deref().is_none_or(|n| name == n);
    let available = |name: &str| {
        registered_tools.iter().any(|tool| tool.name == name)
            && allowed_tools.is_none_or(|allowed| allowed.iter().any(|tool| tool == name))
    };
    let mut entries: Vec<(String, Value)> = match r.view.as_str() {
        "operations" => registered_tools.iter().filter_map(|tool| {
            let (effect, preconditions, verification) = operation(&tool.name).unwrap_or((
                "see_tool_contract", "Inspect the exact tool description and input_schema before use",
                "Tool availability is not proof of execution or business success"));
            if !matches(&tool.name) { return None; }
            let mut entry = json!({"name":tool.name,"effect":effect,"preconditions":preconditions,"verification":verification,
                "available_in_current_context":available(&tool.name),"description":tool.description});
            if r.name.is_some() {
                entry["description"] = json!(tool.description);
                entry["input_schema"] = tool.input_schema.clone();
            }
            Some((tool.name.clone(), entry))
        }).collect(),
        "components" => {
            let mut names: Vec<_> = components.rust_component_names().map(|name| (name, None)).collect();
            names.extend(components.dynamic_store().iter().map(|(name, schema)| (name, Some(schema))));
            names.into_iter().filter(|(name, _)| matches(name)).map(|(name, schema)| {
                let cid = components.lookup_component_id(world, name);
                let count: usize = cid.map(|id| world.archetypes().iter()
                    .filter(|a| a.components().contains(&id)).map(|a| a.entities().len()).sum()).unwrap_or(0);
                let mut entry = json!({"name":name,"kind":if schema.is_some(){"dynamic"}else{"rust"},
                    "instances":count,"schema_hot_migration":false});
                if let Some(schema) = schema {
                    entry["schema"] = json!(schema.fields);
                    entry["presence_only"] = json!(schema.fields.is_empty());
                } else {
                    entry["schema"] = Value::Null;
                    entry["schema_note"] = json!("Rust handler type; inspect existing values/guide, do not infer an empty schema");
                }
                (name.to_owned(), entry)
            }).collect()
        },
        "systems" => systems.iter().filter(|def| matches(&def.id)).map(|def| {
            let rhai = matches!(def.kind, SystemKind::RhaiScript { .. });
            let mut entry = json!({"name":def.id,"description":def.description.chars().take(500).collect::<String>(),
                "kind":if rhai {"rhai"} else {"llm"},"definition_hash":definition_hash(def),
                "is_tool":def.is_tool,"protected":protected(def),
                "can_update":!protected(def) && available("update_system"),
                "can_remove":!protected(def) && available("remove_system") && dependents(systems, &def.id).is_empty(),
                "can_run_system":rhai && available("run_system"),
                "direct_tool_available":def.is_tool && available(&def.id),
                "execution_status":"not_observed_by_catalog"});
            if r.name.is_some() {
                entry["description"] = json!(def.description);
                entry["max_iterations"] = json!(def.max_iterations);
                entry["tool_names"] = json!(def.tool_names);
                entry["explicit_dependents"] = json!(dependents(systems, &def.id));
                if let SystemKind::RhaiScript { source } = &def.kind {
                    let total = source.chars().count();
                    let text: String = source.chars().skip(r.source_offset).take(r.source_limit).collect();
                    let next = r.source_offset.saturating_add(text.chars().count());
                    entry["source"] = json!({"text":text,"offset":r.source_offset,"total_chars":total,
                        "next_offset":(next < total).then_some(next),"complete":r.source_offset == 0 && next >= total});
                }
            }
            (def.id.clone(), entry)
        }).collect(),
        _ => return Err("view must be operations, components or systems".into()),
    };
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by(|a, b| a.0 == b.0);
    if r.name.is_some() && entries.is_empty() {
        return Err(format!(
            "No exact {} entry for {:?}; list the catalog first",
            r.view, r.name
        ));
    }
    let total = entries.len();
    let page: Vec<_> = entries
        .into_iter()
        .skip(r.offset)
        .take(r.limit)
        .map(|(_, v)| v)
        .collect();
    let next = r.offset.saturating_add(page.len());
    Ok(
        json!({"model_version":1,"view":r.view,"read_only":true,"entries":page,
        "matching_total":total,"offset":r.offset,"next_offset":(next < total).then_some(next),
        "catalog_scope":"operations lists registered tools, including contracts not callable in this context. Check available_in_current_context; discovery never grants execution permission. Systems lists registered definitions with per-context permissions. Read guides through systems source without executing run_system.",
        "limits":{"rust_code_hotpatch_exposed":false,"schema_hot_migration":false,"dynamic_changed_filter":false},
        "method":"discover -> inspect current state -> choose legal operation -> execute -> verify actual outcome",
        "training_status":"operation contracts only; no trained model or automatic success labels"}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateRequest {
    id: String,
    expected_definition_hash: String,
    source: String,
    description: Option<String>,
    is_tool: Option<bool>,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveRequest {
    id: String,
    expected_definition_hash: String,
    reason: String,
}

fn checked_current(
    registry: &SystemRegistry,
    id: &str,
    expected: &str,
    reason: &str,
) -> Result<SystemDefinition, String> {
    if reason.trim().is_empty() {
        return Err("reason must be non-empty".into());
    }
    let current = registry
        .get(id)
        .ok_or_else(|| format!("system '{id}' not found"))?;
    if protected(current) {
        return Err("built-in guides and LLM stages cannot be hot-edited".into());
    }
    let hash = definition_hash(current);
    if hash != expected {
        return Err(format!(
            "definition changed: expected {expected}, current {hash}; inspect systems again"
        ));
    }
    Ok(current.clone())
}

pub(crate) fn mutate(
    registry: &mut SystemRegistry,
    engine: &rhai::Engine,
    call: &ToolCall,
    reserved_tool_names: &[String],
) -> Result<(Value, AgentAction), String> {
    match call.name.as_str() {
        "update_system" => {
            let r: UpdateRequest =
                serde_json::from_value(call.input.clone()).map_err(|e| e.to_string())?;
            let mut next =
                checked_current(registry, &r.id, &r.expected_definition_hash, &r.reason)?;
            scripting::check_syntax(engine, &r.source).map_err(|e| e.to_string())?;
            if let Some(is_tool) = r.is_tool {
                if is_tool && reserved_tool_names.iter().any(|name| name == &r.id) {
                    return Err("system id collides with a runtime tool name".into());
                }
                if !is_tool && next.is_tool && !dependents(registry, &r.id).is_empty() {
                    return Err("system is referenced by tool_names; update callers before disabling its tool exposure".into());
                }
                next.is_tool = is_tool;
            }
            if let Some(description) = r.description {
                next.description = description;
            }
            next.kind = SystemKind::RhaiScript { source: r.source };
            let after_hash = definition_hash(&next);
            registry.replace(next)?;
            Ok((
                json!({"id":r.id,"status":"updated","before_hash":r.expected_definition_hash,"definition_hash":after_hash,
                "effective":"next invocation","executed":false,"ecs_state_migrated":false,
                "behavior_verified":false,"reason":r.reason}),
                AgentAction::UpdateSystem {
                    id: r.id,
                    before_hash: r.expected_definition_hash,
                    after_hash,
                },
            ))
        }
        "remove_system" => {
            let r: RemoveRequest =
                serde_json::from_value(call.input.clone()).map_err(|e| e.to_string())?;
            checked_current(registry, &r.id, &r.expected_definition_hash, &r.reason)?;
            let refs = dependents(registry, &r.id);
            if !refs.is_empty() {
                return Err(format!(
                    "system is referenced by tool_names of {}; update callers first",
                    refs.join(", ")
                ));
            }
            registry.remove(&r.id)?;
            Ok((
                json!({"id":r.id,"status":"removed","before_hash":r.expected_definition_hash,"executed":false,
                "ecs_effects_reverted":false,"dependency_check":"explicit tool_names only; code/text references are not analyzed",
                "reason":r.reason}),
                AgentAction::RemoveSystem {
                    id: r.id,
                    before_hash: r.expected_definition_hash,
                },
            ))
        }
        _ => Err("unknown system lifecycle operation".into()),
    }
}
