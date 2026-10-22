//! Persistent, frame-scoped interpretations over immutable evidence.
//!
//! A Lens selects and projects a bounded working set. This module materializes
//! that observation as an interpretation result, records relative relations and
//! provenance as first-class ECS entities, and reuses an exact dependency match.

use bevy_ecs::{component::Component, world::World};
use big_head_ai::{ToolCall, ToolDefinition};
use big_head_ecs::ComponentActionRegistry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const MAX_TEXT_BYTES: usize = 4 * 1024;
const MAX_FACT_BYTES: usize = 64 * 1024;
const MAX_FRAME_BYTES: usize = 16 * 1024;
const DEFAULT_INSPECTION_LIMIT: usize = 50;
const MAX_INSPECTION_LIMIT: usize = 200;

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct FactRecord {
    pub fact_id: String,
    pub statement: String,
    pub payload: Value,
    pub source: String,
    pub observed_at: u64,
    pub epistemic_status: String,
    pub version: String,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InterpretationFrame {
    pub frame_id: String,
    pub task: String,
    pub purpose: String,
    pub scope: String,
    pub parameters: Value,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterpretationInput {
    pub source_id: String,
    pub source_version: String,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InterpretationRun {
    pub run_id: String,
    pub frame_id: String,
    pub result_id: String,
    pub lens_id: String,
    pub lens_version: String,
    #[serde(default)]
    pub method_id: Option<String>,
    #[serde(default)]
    pub method_version: Option<String>,
    #[serde(default)]
    pub method_hash: Option<String>,
    pub inputs: Vec<InterpretationInput>,
    pub cache_key: String,
    pub cache_hit: bool,
    pub started_at: u64,
    pub completed_at: u64,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InterpretationResult {
    pub result_id: String,
    pub frame_id: String,
    pub producer_run_id: String,
    pub lens_id: String,
    pub lens_version: String,
    #[serde(default)]
    pub method_id: Option<String>,
    #[serde(default)]
    pub method_version: Option<String>,
    #[serde(default)]
    pub method_hash: Option<String>,
    pub output: Value,
    pub epistemic_status: String,
    pub created_at: u64,
}

/// A relation is an entity so its frame, derivation and lifecycle remain explicit.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct RelationAssertion {
    pub relation_id: String,
    pub result_id: String,
    pub frame_id: String,
    pub from_ref: String,
    pub predicate: String,
    pub to: Value,
    pub relation_class: String,
    pub source_run_id: String,
    pub epistemic_status: String,
    pub valid_from: u64,
    pub supersedes: Option<String>,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct MaterializationState {
    pub cache_key: String,
    pub frame_fingerprint: String,
    pub frame_id: String,
    pub result_id: String,
    pub lens_id: String,
    pub lens_version: String,
    #[serde(default)]
    pub method_id: Option<String>,
    #[serde(default)]
    pub method_version: Option<String>,
    #[serde(default)]
    pub method_hash: Option<String>,
    pub dependencies: Vec<InterpretationInput>,
    pub valid: bool,
    pub hit_count: u64,
    pub created_at: u64,
    pub last_used_at: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FrameRequest {
    task: String,
    purpose: String,
    scope: String,
    #[serde(default = "empty_object")]
    parameters: Value,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RelationProjection {
    predicate: String,
    #[serde(default = "default_subject_pointer")]
    subject_pointer: String,
    object_pointer: String,
}

fn default_subject_pointer() -> String {
    "/source_id".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterializeRequest {
    lens_id: String,
    #[serde(default)]
    method_id: Option<String>,
    frame: FrameRequest,
    #[serde(default)]
    relation: Option<RelationProjection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordInterpretationRequest {
    frame: FrameRequest,
    evidence_fact_ids: Vec<String>,
    reading: crate::interpretation_method::Reading,
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "record_fact".into(),
            description: "Append an observed or independently verified occurrence with its source. Record what happened or what a person said, not the agent's recommendation, verdict, hypothesis, or explanation; use record_interpretation for those. Corrections are new facts, never rewrites.".into(),
            input_schema: json!({
                "type":"object",
                "properties":{
                    "statement":{"type":"string","minLength":1,"maxLength":4096},
                    "payload":{"type":"object","description":"Structured fields addressable by Lens JSON pointers. Do not JSON-stringify this object."},
                    "source":{"type":"string","minLength":1,"maxLength":4096},
                    "epistemic_status":{"type":"string","enum":["observed","verified"]}
                },
                "required":["statement","payload","source"],
                "additionalProperties":false
            }),
        },
        ToolDefinition {
            name: "record_interpretation".into(),
            description: "Record a one-off, frame-scoped judgment from existing observed/verified FactRecord ids without defining a Lens or Rhai method. The reading is derived, not a new fact; reasons and relations must cite supplied evidence ids. Repeating identical input is a cache hit and does not advance the observation stream.".into(),
            input_schema: json!({
                "type":"object",
                "properties":{
                    "frame":{"type":"object","properties":{
                        "task":{"type":"string"},"purpose":{"type":"string"},"scope":{"type":"string"},"parameters":{"type":"object"}
                    },"required":["task","purpose","scope"],"additionalProperties":false},
                    "evidence_fact_ids":{"type":"array","minItems":1,"maxItems":32,"uniqueItems":true,"items":{"type":"string"}},
                    "reading":{"type":"object","properties":{
                        "verdict":{"type":"string"},
                        "reasons":{"type":"array","items":{"type":"object","properties":{
                            "text":{"type":"string"},"evidence":{"type":"array","items":{"type":"string"}}
                        },"required":["text","evidence"],"additionalProperties":false}},
                        "blockers":{"type":"array","items":{"type":"string"}},
                        "unknowns":{"type":"array","items":{"type":"string"}},
                        "relations":{"type":"array","items":{"type":"object","properties":{
                            "from_ref":{"type":"string"},"predicate":{"type":"string"},"to":{},
                            "evidence":{"type":"array","items":{"type":"string"}}
                        },"required":["from_ref","predicate","to","evidence"],"additionalProperties":false}}
                    },"required":["verdict","reasons","blockers","unknowns"],"additionalProperties":false}
                },
                "required":["frame","evidence_fact_ids","reading"],
                "additionalProperties":false
            }),
        },
        ToolDefinition {
            name: "materialize_interpretation".into(),
            description: "Run an existing read-only Lens inside an explicit task frame. Optional method_id evaluates a pure Rhai interpretation over the bounded Lens output and Frame before persisting. An exact Lens/frame/method/input match reuses the result.".into(),
            input_schema: json!({
                "type":"object",
                "properties":{
                    "lens_id":{"type":"string","minLength":1},
                    "method_id":{"type":"string","minLength":1},
                    "frame":{
                        "type":"object",
                        "properties":{
                            "task":{"type":"string","minLength":1,"maxLength":4096},
                            "purpose":{"type":"string","minLength":1,"maxLength":4096},
                            "scope":{"type":"string","minLength":1,"maxLength":4096},
                            "parameters":{"type":"object"}
                        },
                        "required":["task","purpose","scope"],
                        "additionalProperties":false
                    },
                    "relation":{
                        "type":"object",
                        "properties":{
                            "predicate":{"type":"string","minLength":1,"maxLength":256},
                            "subject_pointer":{"type":"string","description":"JSON pointer into each Lens output item; defaults to /source_id."},
                            "object_pointer":{"type":"string","minLength":1,"description":"JSON pointer into each Lens output item."}
                        },
                        "required":["predicate","object_pointer"],
                        "additionalProperties":false
                    }
                },
                "required":["lens_id","frame"],
                "additionalProperties":false
            }),
        },
        ToolDefinition {
            name: "select_interpretation_path".into(),
            description: "Ask the local Laya/MLX reflex model to choose among 1..5 existing Lens definitions for an explicit interpretation frame, or route to the main agent/defer. Advisory and read-only: it neither runs the Lens nor persists an interpretation. Use deterministic cache reuse directly; use this only when the appropriate existing Lens is uncertain, then call materialize_interpretation explicitly.".into(),
            input_schema: json!({
                "type":"object",
                "properties":{
                    "frame":{
                        "type":"object",
                        "properties":{
                            "task":{"type":"string","minLength":1,"maxLength":4096},
                            "purpose":{"type":"string","minLength":1,"maxLength":4096},
                            "scope":{"type":"string","minLength":1,"maxLength":4096},
                            "parameters":{"type":"object"}
                        },
                        "required":["task","purpose","scope"],
                        "additionalProperties":false
                    },
                    "state":{"type":"object","description":"Compact observations relevant to choosing an interpretation path; unknowns should be explicit."},
                    "candidate_lens_ids":{"type":"array","minItems":1,"maxItems":5,"uniqueItems":true,"items":{"type":"string","minLength":1}}
                },
                "required":["frame","state","candidate_lens_ids"],
                "additionalProperties":false
            }),
        },
        ToolDefinition {
            name: "inspect_interpretations".into(),
            description: "Read-only summary of interpretation, interaction experience and observation records. Pass result_id, experience_id, or observation_id to inspect one record.".into(),
            input_schema: json!({
                "type":"object",
                "properties":{
                    "result_id":{"type":"string"},
                    "observation_id":{"type":"string"},
                    "experience_id":{"type":"string"},
                    "offset":{"type":"integer","minimum":0},
                    "limit":{"type":"integer","minimum":1,"maximum":200,"default":50}
                },
                "additionalProperties":false
            }),
        },
    ]
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectPathRequest {
    frame: FrameRequest,
    state: serde_json::Map<String, Value>,
    candidate_lens_ids: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct LayaSelection {
    routes: BTreeMap<String, (String, String)>,
    frame: Value,
}

pub(crate) fn prepare_laya_selection(
    world: &mut World,
    input: &Value,
) -> Result<(Value, HashSet<String>, LayaSelection), String> {
    let request: SelectPathRequest =
        serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    validate_frame(&request.frame)?;
    if !(1..=5).contains(&request.candidate_lens_ids.len()) {
        return Err("candidate_lens_ids must contain 1..5 existing Lens ids".into());
    }
    if serde_json::to_vec(&request.state)
        .map_err(|e| e.to_string())?
        .len()
        > 4096
    {
        return Err("interpretation routing state exceeds 4096 bytes".into());
    }
    let unique: BTreeSet<_> = request.candidate_lens_ids.iter().collect();
    if unique.len() != request.candidate_lens_ids.len() {
        return Err("candidate_lens_ids must be unique".into());
    }

    let catalog = crate::lens_runtime::definition_catalog(world)?;
    let mut routes = BTreeMap::new();
    let mut candidates = Vec::new();
    let mut lens_state = Vec::new();
    // Route labels are positional, and small local choice models can prefer the
    // first label. Keep the mapping independent of caller-provided ordering so
    // semantically identical candidate sets produce the same Laya request.
    let mut candidate_lens_ids = request.candidate_lens_ids.clone();
    candidate_lens_ids.sort();
    for (index, lens_id) in candidate_lens_ids.iter().enumerate() {
        let definition = catalog
            .get(lens_id)
            .ok_or_else(|| format!("unknown candidate Lens '{lens_id}'"))?;
        let route = format!("lens_{index}");
        routes.insert(
            route.clone(),
            (definition.id.clone(), definition.version.clone()),
        );
        // Laya renders candidate descriptions inside a 48-token option budget.
        // Keep options structural; full Lens semantics live in the bounded state.
        candidates.push(json!({
            "operation":route,
            "description":format!("Use candidate {route} from candidate_lenses.")
        }));
        lens_state.push(json!({
            "route":route,
            "lens_id":definition.id,
            "version":definition.version,
            "purpose":definition.purpose,
            "tasks":definition.usage.tasks,
            "tags":definition.usage.tags,
            "limitations":definition.usage.limitations
        }));
    }
    candidates.push(json!({
        "operation":"main_agent",
        "description":"Escalate when no candidate Lens fits the frame."
    }));
    let frame = serde_json::to_value(&request.frame).map_err(|e| e.to_string())?;
    let laya_request = json!({
        "goal":"Choose the existing Lens that best fits this interpretation frame. Choose main_agent when no candidate expresses the required viewpoint; defer when evidence is insufficient.",
        "state":{
            "frame":frame,
            "observations":request.state,
            "candidate_lenses":lens_state
        },
        "candidates":candidates
    });
    if serde_json::to_vec(&laya_request["state"])
        .map_err(|e| e.to_string())?
        .len()
        > 8192
    {
        return Err("Lens metadata and routing state exceed Laya's 8192-byte budget".into());
    }
    let mut choices: HashSet<String> = routes.keys().cloned().collect();
    choices.insert("main_agent".into());
    choices.insert("defer".into());
    Ok((laya_request, choices, LayaSelection { routes, frame }))
}

pub(crate) fn finish_laya_selection(
    mut response: Value,
    selection: LayaSelection,
) -> Result<Value, String> {
    let operation = response["operation"]
        .as_str()
        .ok_or("Laya response has no operation")?;
    let route = if let Some((lens_id, lens_version)) = selection.routes.get(operation) {
        json!({
            "kind":"existing_lens",
            "lens_id":lens_id,
            "lens_version":lens_version,
            "next_tool":"materialize_interpretation"
        })
    } else {
        match operation {
            "main_agent" => json!({
                "kind":"main_agent",
                "reason":"No candidate Lens was selected; broader interpretation or a reviewed Lens definition is needed."
            }),
            "defer" => json!({
                "kind":"defer",
                "reason":"Evidence is insufficient for the bounded interpretation choice."
            }),
            _ => return Err("Laya selected an unmapped interpretation route".into()),
        }
    };
    response["decision_kind"] = json!("interpretation_path");
    response["interpretation_frame"] = selection.frame;
    response["interpretation_route"] = route;
    response["materialized"] = json!(false);
    Ok(response)
}

pub fn dispatch(
    world: &mut World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> Result<Value, String> {
    match call.name.as_str() {
        "record_fact" => record_fact(world, &call.input),
        "record_interpretation" => record_interpretation(world, &call.input),
        "materialize_interpretation" => materialize(world, registry, &call.input),
        "inspect_interpretations" => inspect(world, &call.input),
        _ => Err("unknown interpretation tool".into()),
    }
}

fn record_fact(world: &mut World, input: &Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        statement: String,
        payload: Value,
        source: String,
        #[serde(default = "observed_status")]
        epistemic_status: String,
    }
    fn observed_status() -> String {
        "observed".into()
    }

    let request: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    bounded_text("statement", &request.statement, MAX_TEXT_BYTES)?;
    bounded_text("source", &request.source, MAX_TEXT_BYTES)?;
    if !["observed", "verified"].contains(&request.epistemic_status.as_str()) {
        return Err("record_fact requires observed or verified evidence; use record_interpretation for judgments".into());
    }
    if !request.payload.is_object() {
        return Err(
            "fact payload must be a JSON object, not a JSON-encoded string or scalar".into(),
        );
    }
    if serde_json::to_vec(&request.payload)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_FACT_BYTES
    {
        return Err("fact payload exceeds 64 KiB; store large evidence in ShardData".into());
    }
    let observed_at = now();
    let fact_id = uuid::Uuid::new_v4().to_string();
    let version = digest_json(&json!({
        "fact_id":fact_id,
        "statement":request.statement,
        "payload":request.payload,
        "source":request.source,
        "observed_at":observed_at,
        "epistemic_status":request.epistemic_status
    }))?;
    let fact = FactRecord {
        fact_id: fact_id.clone(),
        statement: request.statement,
        payload: request.payload,
        source: request.source,
        observed_at,
        epistemic_status: request.epistemic_status,
        version: version.clone(),
    };
    let entity = world.spawn(fact).id();
    Ok(json!({"fact_id":fact_id,"version":version,"entity_id":entity.index().index()}))
}

fn record_interpretation(world: &mut World, input: &Value) -> Result<Value, String> {
    const DIRECT_LENS_ID: &str = "direct.fact_records";
    let request: RecordInterpretationRequest =
        serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    validate_frame(&request.frame)?;
    bounded_text("verdict", &request.reading.verdict, 128)?;
    if request.evidence_fact_ids.is_empty() || request.evidence_fact_ids.len() > 32 {
        return Err("evidence_fact_ids must contain 1..32 FactRecord ids".into());
    }
    if request.reading.reasons.is_empty() {
        return Err("interpretation requires at least one evidence-backed reason".into());
    }
    if request.reading.reasons.len() > 64
        || request.reading.blockers.len() > 64
        || request.reading.unknowns.len() > 64
        || request.reading.relations.len() > 128
    {
        return Err("interpretation output has too many entries".into());
    }
    let output = serde_json::to_value(&request.reading).map_err(|e| e.to_string())?;
    if serde_json::to_vec(&output)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_FACT_BYTES
    {
        return Err("interpretation output exceeds 64 KiB".into());
    }
    let requested: BTreeSet<_> = request.evidence_fact_ids.iter().cloned().collect();
    if requested.len() != request.evidence_fact_ids.len() {
        return Err("evidence_fact_ids must be unique".into());
    }
    let found: BTreeMap<_, _> = world
        .query::<&FactRecord>()
        .iter(world)
        .filter(|fact| requested.contains(&fact.fact_id))
        .map(|fact| {
            (
                fact.fact_id.clone(),
                (fact.version.clone(), fact.epistemic_status.clone()),
            )
        })
        .collect();
    if found.len() != requested.len() {
        return Err("interpretation references an unknown FactRecord".into());
    }
    if found
        .values()
        .any(|(_, status)| status != "observed" && status != "verified")
    {
        return Err("interpretation evidence must be observed or verified FactRecords".into());
    }
    let dependencies: Vec<_> = found
        .iter()
        .map(|(id, (version, _))| InterpretationInput {
            source_id: id.clone(),
            source_version: version.clone(),
        })
        .collect();
    let (relations, _) = validate_reading(world, &request.reading, &dependencies, &[])?;
    let frame_value = serde_json::to_value(&request.frame).map_err(|e| e.to_string())?;
    let cache_key = digest_json(&json!({
        "kind":DIRECT_LENS_ID,"frame":frame_value,"evidence":dependencies,"reading":output
    }))?;
    let timestamp = now();
    let prior = world
        .query::<&InterpretationRun>()
        .iter(world)
        .find(|run| run.lens_id == DIRECT_LENS_ID && run.cache_key == cache_key)
        .cloned();
    if let Some(prior) = prior {
        let exists = world
            .query::<&InterpretationResult>()
            .iter(world)
            .any(|result| result.result_id == prior.result_id);
        if exists {
            let run_id = uuid::Uuid::new_v4().to_string();
            world.spawn(InterpretationRun {
                run_id: run_id.clone(),
                frame_id: prior.frame_id.clone(),
                result_id: prior.result_id.clone(),
                lens_id: DIRECT_LENS_ID.into(),
                lens_version: "1".into(),
                method_id: None,
                method_version: None,
                method_hash: None,
                inputs: dependencies,
                cache_key: cache_key.clone(),
                cache_hit: true,
                started_at: timestamp,
                completed_at: timestamp,
            });
            return Ok(
                json!({"cache_hit":true,"cache_key":cache_key,"frame_id":prior.frame_id,
                "result_id":prior.result_id,"run_id":run_id,"relations_created":0}),
            );
        }
    }

    let frame_id = uuid::Uuid::new_v4().to_string();
    let run_id = uuid::Uuid::new_v4().to_string();
    let result_id = uuid::Uuid::new_v4().to_string();
    let scope = request.frame.scope.clone();
    world.spawn(InterpretationFrame {
        frame_id: frame_id.clone(),
        task: request.frame.task,
        purpose: request.frame.purpose,
        scope: request.frame.scope,
        parameters: request.frame.parameters,
        created_at: timestamp,
    });
    world.spawn(InterpretationResult {
        result_id: result_id.clone(),
        frame_id: frame_id.clone(),
        producer_run_id: run_id.clone(),
        lens_id: DIRECT_LENS_ID.into(),
        lens_version: "1".into(),
        method_id: None,
        method_version: None,
        method_hash: None,
        output,
        epistemic_status: "derived".into(),
        created_at: timestamp,
    });
    world.spawn(InterpretationRun {
        run_id: run_id.clone(),
        frame_id: frame_id.clone(),
        result_id: result_id.clone(),
        lens_id: DIRECT_LENS_ID.into(),
        lens_version: "1".into(),
        method_id: None,
        method_version: None,
        method_hash: None,
        inputs: dependencies.clone(),
        cache_key: cache_key.clone(),
        cache_hit: false,
        started_at: timestamp,
        completed_at: timestamp,
    });
    for dependency in &dependencies {
        world.spawn(RelationAssertion {
            relation_id: uuid::Uuid::new_v4().to_string(),
            result_id: result_id.clone(),
            frame_id: frame_id.clone(),
            from_ref: result_id.clone(),
            predicate: "derived_from".into(),
            to: Value::String(dependency.source_id.clone()),
            relation_class: "provenance".into(),
            source_run_id: run_id.clone(),
            epistemic_status: "recorded".into(),
            valid_from: timestamp,
            supersedes: None,
        });
    }
    for (from_ref, predicate, to) in &relations {
        world.spawn(RelationAssertion {
            relation_id: uuid::Uuid::new_v4().to_string(),
            result_id: result_id.clone(),
            frame_id: frame_id.clone(),
            from_ref: from_ref.clone(),
            predicate: predicate.clone(),
            to: to.clone(),
            relation_class: "relative".into(),
            source_run_id: run_id.clone(),
            epistemic_status: "derived".into(),
            valid_from: timestamp,
            supersedes: None,
        });
    }
    crate::interpretation_stream::enqueue(world, &scope, &result_id);
    Ok(
        json!({"cache_hit":false,"cache_key":cache_key,"frame_id":frame_id,
        "result_id":result_id,"run_id":run_id,
        "relations_created":dependencies.len()+relations.len()}),
    )
}

fn materialize(
    world: &mut World,
    registry: &ComponentActionRegistry,
    input: &Value,
) -> Result<Value, String> {
    let request: MaterializeRequest =
        serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    validate_frame(&request.frame)?;
    if request.lens_id.trim().is_empty() {
        return Err("lens_id must be nonempty".into());
    }
    if let Some(relation) = &request.relation {
        bounded_text("predicate", &relation.predicate, 256)?;
        validate_pointer("subject_pointer", &relation.subject_pointer)?;
        validate_pointer("object_pointer", &relation.object_pointer)?;
    }

    let catalog = crate::lens_runtime::definition_catalog(world)?;
    let definition = catalog.get(&request.lens_id).ok_or("unknown Lens")?;
    let lens_version = definition.version.clone();
    let observed = crate::lens_runtime::observe(world, registry, &request.lens_id, &catalog)?;
    let output = observed
        .get("data")
        .and_then(Value::as_array)
        .ok_or("Lens receipt has no data array")?;
    let dependencies = parse_inputs(&observed)?;
    let method = request
        .method_id
        .as_deref()
        .map(|id| crate::interpretation_method::find(world, id))
        .transpose()?;
    if method
        .as_ref()
        .is_some_and(|method| method.lens_id != request.lens_id)
    {
        return Err("interpretation method belongs to a different Lens".into());
    }
    if method.is_some() && request.relation.is_some() {
        return Err("relation projection cannot be combined with an interpretation method".into());
    }
    let frame_value = serde_json::to_value(&request.frame).map_err(|e| e.to_string())?;
    let frame_fingerprint = digest_json(&frame_value)?;
    let mut cache_input = json!({
        "lens_id":request.lens_id,
        "lens_version":lens_version,
        "lens_cache_key":observed["cache_key"],
        "frame":frame_value,
        "relation":request.relation
    });
    if let Some(method) = &method {
        cache_input["method"] =
            json!({"id":method.id,"version":method.version,"hash":method.definition_hash});
    }
    let cache_key = digest_json(&cache_input)?;
    let timestamp = now();

    let states: Vec<_> = world
        .query::<(bevy_ecs::entity::Entity, &MaterializationState)>()
        .iter(world)
        .map(|(entity, state)| (entity, state.clone()))
        .collect();
    let matching: Vec<_> = states
        .iter()
        .filter(|(_, state)| state.valid && state.cache_key == cache_key)
        .collect();
    if matching.len() > 1 {
        return Err("duplicate materialization cache key; interpretation blocked".into());
    }
    if let Some((state_entity, state)) = matching.first() {
        let result_exists = world
            .query::<&InterpretationResult>()
            .iter(world)
            .any(|result| result.result_id == state.result_id);
        if result_exists {
            let run_id = uuid::Uuid::new_v4().to_string();
            let run = InterpretationRun {
                run_id: run_id.clone(),
                frame_id: state.frame_id.clone(),
                result_id: state.result_id.clone(),
                lens_id: request.lens_id,
                lens_version,
                method_id: method.as_ref().map(|method| method.id.clone()),
                method_version: method.as_ref().map(|method| method.version.clone()),
                method_hash: method.as_ref().map(|method| method.definition_hash.clone()),
                inputs: dependencies,
                cache_key: cache_key.clone(),
                cache_hit: true,
                started_at: timestamp,
                completed_at: timestamp,
            };
            let run_entity = world.spawn(run).id();
            let mut current = world
                .get_mut::<MaterializationState>(*state_entity)
                .unwrap();
            current.hit_count = current.hit_count.saturating_add(1);
            current.last_used_at = timestamp;
            return Ok(json!({
                "cache_hit":true,
                "cache_key":cache_key,
                "frame_id":state.frame_id,
                "result_id":state.result_id,
                "run_id":run_id,
                "run_entity_id":run_entity.index().index(),
                "input_count":output.len(),
                "relations_created":0
            }));
        }
    }

    let (result_output, semantic_relations, evidence_facts) = if let Some(method) = &method {
        let reading = crate::interpretation_method::evaluate(method, &frame_value, output)?;
        let (relations, evidence_facts) = validate_reading(world, &reading, &dependencies, output)?;
        (
            serde_json::to_value(reading).map_err(|e| e.to_string())?,
            relations,
            evidence_facts,
        )
    } else {
        (
            Value::Array(output.clone()),
            build_semantic_relations(output, request.relation.as_ref())?,
            Vec::new(),
        )
    };
    for (entity, state) in &states {
        if state.valid
            && state.lens_id == request.lens_id
            && state.method_id == method.as_ref().map(|method| method.id.clone())
            && state.frame_fingerprint == frame_fingerprint
            && state.cache_key != cache_key
        {
            world
                .get_mut::<MaterializationState>(*entity)
                .unwrap()
                .valid = false;
        }
    }

    let frame_id = uuid::Uuid::new_v4().to_string();
    let run_id = uuid::Uuid::new_v4().to_string();
    let result_id = uuid::Uuid::new_v4().to_string();
    let observation_scope = request.frame.scope.clone();
    let frame = InterpretationFrame {
        frame_id: frame_id.clone(),
        task: request.frame.task,
        purpose: request.frame.purpose,
        scope: request.frame.scope,
        parameters: request.frame.parameters,
        created_at: timestamp,
    };
    let result = InterpretationResult {
        result_id: result_id.clone(),
        frame_id: frame_id.clone(),
        producer_run_id: run_id.clone(),
        lens_id: request.lens_id.clone(),
        lens_version: lens_version.clone(),
        method_id: method.as_ref().map(|method| method.id.clone()),
        method_version: method.as_ref().map(|method| method.version.clone()),
        method_hash: method.as_ref().map(|method| method.definition_hash.clone()),
        output: result_output,
        epistemic_status: "derived".into(),
        created_at: timestamp,
    };
    let run = InterpretationRun {
        run_id: run_id.clone(),
        frame_id: frame_id.clone(),
        result_id: result_id.clone(),
        lens_id: request.lens_id.clone(),
        lens_version: lens_version.clone(),
        method_id: method.as_ref().map(|method| method.id.clone()),
        method_version: method.as_ref().map(|method| method.version.clone()),
        method_hash: method.as_ref().map(|method| method.definition_hash.clone()),
        inputs: dependencies.clone(),
        cache_key: cache_key.clone(),
        cache_hit: false,
        started_at: timestamp,
        completed_at: timestamp,
    };
    let state = MaterializationState {
        cache_key: cache_key.clone(),
        frame_fingerprint,
        frame_id: frame_id.clone(),
        result_id: result_id.clone(),
        lens_id: request.lens_id,
        lens_version,
        method_id: method.as_ref().map(|method| method.id.clone()),
        method_version: method.as_ref().map(|method| method.version.clone()),
        method_hash: method.as_ref().map(|method| method.definition_hash.clone()),
        dependencies: dependencies.clone(),
        valid: true,
        hit_count: 0,
        created_at: timestamp,
        last_used_at: timestamp,
    };
    let frame_entity = world.spawn(frame).id();
    let result_entity = world.spawn(result).id();
    let run_entity = world.spawn(run).id();
    let mut relation_count = 0usize;
    for dependency in &dependencies {
        world.spawn(RelationAssertion {
            relation_id: uuid::Uuid::new_v4().to_string(),
            result_id: result_id.clone(),
            frame_id: frame_id.clone(),
            from_ref: result_id.clone(),
            predicate: "derived_from".into(),
            to: Value::String(dependency.source_id.clone()),
            relation_class: "provenance".into(),
            source_run_id: run_id.clone(),
            epistemic_status: "recorded".into(),
            valid_from: timestamp,
            supersedes: None,
        });
        relation_count += 1;
    }
    for fact_id in evidence_facts {
        world.spawn(RelationAssertion {
            relation_id: uuid::Uuid::new_v4().to_string(),
            result_id: result_id.clone(),
            frame_id: frame_id.clone(),
            from_ref: result_id.clone(),
            predicate: "derived_from".into(),
            to: Value::String(fact_id),
            relation_class: "provenance".into(),
            source_run_id: run_id.clone(),
            epistemic_status: "recorded".into(),
            valid_from: timestamp,
            supersedes: None,
        });
        relation_count += 1;
    }
    for (from_ref, predicate, to) in semantic_relations {
        world.spawn(RelationAssertion {
            relation_id: uuid::Uuid::new_v4().to_string(),
            result_id: result_id.clone(),
            frame_id: frame_id.clone(),
            from_ref,
            predicate,
            to,
            relation_class: "relative".into(),
            source_run_id: run_id.clone(),
            epistemic_status: "derived".into(),
            valid_from: timestamp,
            supersedes: None,
        });
        relation_count += 1;
    }
    let state_entity = world.spawn(state).id();
    if method.is_some() {
        crate::interpretation_stream::enqueue(world, &observation_scope, &result_id);
    }
    Ok(json!({
        "cache_hit":false,
        "cache_key":cache_key,
        "frame_id":frame_id,
        "result_id":result_id,
        "run_id":run_id,
        "frame_entity_id":frame_entity.index().index(),
        "result_entity_id":result_entity.index().index(),
        "run_entity_id":run_entity.index().index(),
        "materialization_entity_id":state_entity.index().index(),
        "input_count":dependencies.len(),
        "relations_created":relation_count
    }))
}

fn inspect(world: &mut World, input: &Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        result_id: Option<String>,
        observation_id: Option<String>,
        experience_id: Option<String>,
        #[serde(default)]
        offset: usize,
        #[serde(default = "default_inspection_limit")]
        limit: usize,
    }
    fn default_inspection_limit() -> usize {
        DEFAULT_INSPECTION_LIMIT
    }
    let request: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if [
        request.result_id.is_some(),
        request.observation_id.is_some(),
        request.experience_id.is_some(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
        > 1
    {
        return Err("inspect one result_id, observation_id or experience_id at a time".into());
    }
    if !(1..=MAX_INSPECTION_LIMIT).contains(&request.limit) {
        return Err("inspection limit must be 1..200".into());
    }
    let facts = world.query::<&FactRecord>().iter(world).count();
    let frames = world.query::<&InterpretationFrame>().iter(world).count();
    let (runs, cache_hit_runs) = world
        .query::<&InterpretationRun>()
        .iter(world)
        .fold((0usize, 0usize), |(count, hits), run| {
            (count + 1, hits + usize::from(run.cache_hit))
        });
    let results = world.query::<&InterpretationResult>().iter(world).count();
    let (streams, pending_results) = world
        .query::<&crate::interpretation_stream::InterpretationStream>()
        .iter(world)
        .fold((0usize, 0usize), |(count, pending), stream| {
            (count + 1, pending + stream.pending_result_ids.len())
        });
    let observations = world
        .query::<&crate::interpretation_stream::InterpretationObservation>()
        .iter(world)
        .count();
    let experiences = world
        .query::<&crate::experience::InteractionExperience>()
        .iter(world)
        .count();
    let experience_readings = world
        .query::<&crate::experience::ExperienceReading>()
        .iter(world)
        .count();
    let pending_experiences: usize = world
        .query::<&crate::interpretation_stream::InterpretationStream>()
        .iter(world)
        .map(|stream| stream.pending_experience_ids.len())
        .sum();
    let (relations, relative_relations, provenance_relations) =
        world.query::<&RelationAssertion>().iter(world).fold(
            (0usize, 0usize, 0usize),
            |(count, relative, provenance), relation| {
                (
                    count + 1,
                    relative + usize::from(relation.relation_class == "relative"),
                    provenance + usize::from(relation.relation_class == "provenance"),
                )
            },
        );
    let (materializations, active_materializations, cache_hits) = world
        .query::<&MaterializationState>()
        .iter(world)
        .fold((0usize, 0usize, 0u64), |(count, active, hits), state| {
            (
                count + 1,
                active + usize::from(state.valid),
                hits.saturating_add(state.hit_count),
            )
        });
    let summary = json!({
        "facts":facts,
        "frames":frames,
        "runs":runs,
        "cache_hit_runs":cache_hit_runs,
        "results":results,
        "streams":streams,
        "pending_observation_results":pending_results,
        "observations":observations,
        "experiences":experiences,
        "experience_readings":experience_readings,
        "pending_observation_experiences":pending_experiences,
        "relations":relations,
        "relative_relations":relative_relations,
        "provenance_relations":provenance_relations,
        "materializations":materializations,
        "active_materializations":active_materializations,
        "invalid_materializations":materializations.saturating_sub(active_materializations),
        "cache_hits":cache_hits
    });
    if let Some(experience_id) = request.experience_id {
        let experience = world
            .query::<&crate::experience::InteractionExperience>()
            .iter(world)
            .find(|item| item.experience_id == experience_id)
            .cloned()
            .ok_or("unknown interaction experience")?;
        return Ok(json!({"summary":summary,"experience":experience}));
    }
    if let Some(observation_id) = request.observation_id {
        let observation = world
            .query::<&crate::interpretation_stream::InterpretationObservation>()
            .iter(world)
            .find(|observation| observation.observation_id == observation_id)
            .cloned()
            .ok_or("unknown interpretation observation")?;
        return Ok(json!({"summary":summary,"observation":observation}));
    }
    let Some(result_id) = request.result_id else {
        return Ok(json!({"summary":summary}));
    };
    let result = world
        .query::<&InterpretationResult>()
        .iter(world)
        .find(|result| result.result_id == result_id)
        .cloned()
        .ok_or("unknown interpretation result")?;
    let frame = world
        .query::<&InterpretationFrame>()
        .iter(world)
        .find(|frame| frame.frame_id == result.frame_id)
        .cloned();
    let result_runs_total = world
        .query::<&InterpretationRun>()
        .iter(world)
        .filter(|run| run.result_id == result_id)
        .count();
    let result_runs: Vec<_> = world
        .query::<&InterpretationRun>()
        .iter(world)
        .filter(|run| run.result_id == result_id)
        .skip(request.offset)
        .take(request.limit)
        .cloned()
        .collect();
    let result_relations_total = world
        .query::<&RelationAssertion>()
        .iter(world)
        .filter(|relation| relation.result_id == result_id)
        .count();
    let result_relations: Vec<_> = world
        .query::<&RelationAssertion>()
        .iter(world)
        .filter(|relation| relation.result_id == result_id)
        .skip(request.offset)
        .take(request.limit)
        .cloned()
        .collect();
    Ok(json!({
        "summary":summary,
        "frame":frame,
        "result":result,
        "runs":result_runs,
        "relations":result_relations,
        "page":{
            "offset":request.offset,
            "limit":request.limit,
            "runs_total":result_runs_total,
            "runs_complete":request.offset.saturating_add(result_runs.len()) >= result_runs_total,
            "relations_total":result_relations_total,
            "relations_complete":request.offset.saturating_add(result_relations.len()) >= result_relations_total
        }
    }))
}

fn validate_frame(frame: &FrameRequest) -> Result<(), String> {
    for (name, value) in [
        ("task", frame.task.as_str()),
        ("purpose", frame.purpose.as_str()),
        ("scope", frame.scope.as_str()),
    ] {
        bounded_text(name, value, MAX_TEXT_BYTES)?;
    }
    if !frame.parameters.is_object() {
        return Err("frame parameters must be a JSON object".into());
    }
    if serde_json::to_vec(&frame.parameters)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_FRAME_BYTES
    {
        return Err("frame parameters exceed 16 KiB".into());
    }
    Ok(())
}

fn bounded_text(name: &str, value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{name} must be nonempty"));
    }
    if value.len() > max {
        return Err(format!("{name} exceeds {max} bytes"));
    }
    Ok(())
}

fn validate_pointer(name: &str, pointer: &str) -> Result<(), String> {
    if pointer.starts_with('/') && pointer.len() <= 512 {
        Ok(())
    } else {
        Err(format!(
            "{name} must be a JSON pointer of at most 512 bytes"
        ))
    }
}

fn parse_inputs(receipt: &Value) -> Result<Vec<InterpretationInput>, String> {
    let inputs = receipt
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or("Lens receipt has no inputs")?;
    let mut parsed = Vec::with_capacity(inputs.len());
    for input in inputs {
        parsed.push(InterpretationInput {
            source_id: input
                .get("source_id")
                .and_then(Value::as_str)
                .ok_or("Lens input has no source_id")?
                .into(),
            source_version: input
                .get("version")
                .and_then(Value::as_str)
                .ok_or("Lens input has no version")?
                .into(),
        });
    }
    parsed.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(parsed)
}

fn validate_reading(
    world: &mut World,
    reading: &crate::interpretation_method::Reading,
    inputs: &[InterpretationInput],
    output: &[Value],
) -> Result<(Vec<(String, String, Value)>, Vec<String>), String> {
    let mut allowed: HashSet<String> = inputs.iter().map(|input| input.source_id.clone()).collect();
    let referenced_facts: BTreeSet<String> = output
        .iter()
        .filter_map(|item| item.get("value"))
        .filter_map(Value::as_object)
        .flat_map(|value| value.iter())
        .filter(|(key, _)| key == &"evidence_fact_id" || key.ends_with("_evidence_fact_id"))
        .filter_map(|(_, value)| value.as_str().map(str::to_owned))
        .collect();
    let existing_facts: HashSet<String> = world
        .query::<&FactRecord>()
        .iter(world)
        .filter(|fact| referenced_facts.contains(&fact.fact_id))
        .map(|fact| fact.fact_id.clone())
        .collect();
    if existing_facts.len() != referenced_facts.len() {
        return Err("Lens output references an unknown evidence FactRecord".into());
    }
    allowed.extend(existing_facts.iter().cloned());
    let check_evidence = |refs: &[String]| -> Result<(), String> {
        if refs.is_empty() || refs.iter().any(|reference| !allowed.contains(reference)) {
            return Err(
                "interpretation evidence must reference a Lens input or its FactRecord".into(),
            );
        }
        Ok(())
    };
    let check_text = |text: &str| -> Result<(), String> {
        if text.trim().is_empty() || text.len() > 4096 {
            return Err("interpretation text must contain 1..4096 bytes".into());
        }
        Ok(())
    };
    for reason in &reading.reasons {
        check_text(&reason.text)?;
        check_evidence(&reason.evidence)?;
    }
    for text in reading.blockers.iter().chain(&reading.unknowns) {
        check_text(text)?;
    }
    let mut relations = Vec::with_capacity(reading.relations.len());
    for relation in &reading.relations {
        check_text(&relation.from_ref)?;
        check_text(&relation.predicate)?;
        check_evidence(&relation.evidence)?;
        if relation.to.is_null() || relation.to.is_array() || relation.to.is_object() {
            return Err("interpretation relation target must be a non-null scalar".into());
        }
        relations.push((
            relation.from_ref.clone(),
            relation.predicate.clone(),
            relation.to.clone(),
        ));
    }
    Ok((relations, existing_facts.into_iter().collect()))
}

fn build_semantic_relations(
    output: &[Value],
    projection: Option<&RelationProjection>,
) -> Result<Vec<(String, String, Value)>, String> {
    let Some(projection) = projection else {
        return Ok(vec![]);
    };
    let mut unique = BTreeMap::<String, (String, String, Value)>::new();
    for item in output {
        let subject = item.pointer(&projection.subject_pointer).ok_or_else(|| {
            format!(
                "subject_pointer missing in Lens output: {}",
                projection.subject_pointer
            )
        })?;
        let from_ref = match subject {
            Value::String(value) => value.clone(),
            Value::Number(value) => value.to_string(),
            _ => return Err("relation subject must resolve to a string or number".into()),
        };
        let to = item.pointer(&projection.object_pointer).ok_or_else(|| {
            format!(
                "object_pointer missing in Lens output: {}",
                projection.object_pointer
            )
        })?;
        if to.is_null() {
            return Err(format!(
                "object_pointer resolved to null in Lens output: {}",
                projection.object_pointer
            ));
        }
        let to = to.clone();
        let key = digest_json(&json!([from_ref, projection.predicate, to]))?;
        unique.insert(key, (from_ref, projection.predicate.clone(), to));
    }
    Ok(unique.into_values().collect())
}

fn digest_json(value: &Value) -> Result<String, String> {
    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|e| e.to_string())?;
    Ok(big_head_save::persistence::digest(&bytes))
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

fn now() -> u64 {
    big_head_ecs::execution::now_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, input: Value) -> ToolCall {
        ToolCall {
            id: format!("test-{name}"),
            name: name.into(),
            input,
        }
    }

    #[test]
    fn direct_interpretation_separates_judgment_from_fact_and_deduplicates() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let before = rt.world.entities().len();
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call(
                    "record_fact",
                    json!({
                        "statement":"Publish tonight", "payload":{"verdict":"publish"},
                        "source":"agent advice", "epistemic_status":"asserted"
                    })
                )
            )
            .unwrap_err()
            .contains("use record_interpretation")
        );
        assert_eq!(rt.world.entities().len(), before);
        let fact = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "record_fact",
                json!({
                    "statement":"Build command exited 0", "payload":{"exit_code":0},
                    "source":"npm run build", "epistemic_status":"verified"
                }),
            ),
        )
        .unwrap();
        let fact_id = fact["fact_id"].as_str().unwrap();
        let input = json!({
            "frame":{"task":"Choose tonight's work","purpose":"Advise, not certify release",
                "scope":"garden","parameters":{"audience":"friends"}},
            "evidence_fact_ids":[fact_id],
            "reading":{"verdict":"preview_candidate",
                "reasons":[{"text":"Build exited cleanly","evidence":[fact_id]}],
                "blockers":[],"unknowns":["Browser smoke test not observed"]}
        });
        let first = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("record_interpretation", input.clone()),
        )
        .unwrap();
        assert_eq!(first["cache_hit"], false);
        let inspected = inspect(&mut rt.world, &json!({"result_id":first["result_id"]})).unwrap();
        assert_eq!(inspected["result"]["epistemic_status"], "derived");
        assert_eq!(
            inspected["result"]["output"]["verdict"],
            "preview_candidate"
        );
        assert_eq!(inspected["relations"][0]["to"], fact_id);
        let second = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("record_interpretation", input.clone()),
        )
        .unwrap();
        assert_eq!(second["cache_hit"], true);
        assert_eq!(second["result_id"], first["result_id"]);
        let stream = rt
            .world
            .query::<&crate::interpretation_stream::InterpretationStream>()
            .iter(&rt.world)
            .next()
            .unwrap();
        assert_eq!(stream.pending_result_ids.len(), 1);

        let replay = input.clone();
        let mut invented = input;
        invented["reading"]["reasons"][0]["evidence"] = json!(["invented"]);
        let count = rt.world.entities().len();
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("record_interpretation", invented)
            )
            .unwrap_err()
            .contains("evidence must reference")
        );
        assert_eq!(rt.world.entities().len(), count);

        let directory =
            std::env::temp_dir().join(format!("direct-interpretation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let checkpoint = directory.join("checkpoint.json");
        rt.checkpoint(&checkpoint).unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&checkpoint).unwrap();
        let after_restore = dispatch(
            &mut restored.world,
            &restored.component_registry,
            &call("record_interpretation", replay),
        )
        .unwrap();
        assert_eq!(after_restore["cache_hit"], true);
        assert_eq!(after_restore["result_id"], first["result_id"]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn direct_interpretation_does_not_promote_legacy_assertion_to_evidence() {
        let mut rt = crate::AgentRuntime::new_for_test();
        rt.world.spawn(FactRecord {
            fact_id: "old-advice".into(),
            statement: "Agent recommended publishing".into(),
            payload: json!({"verdict":"publish"}),
            source: "legacy agent".into(),
            observed_at: 1,
            epistemic_status: "asserted".into(),
            version: "v1".into(),
        });
        let before = rt.world.entities().len();
        let error = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "record_interpretation",
                json!({
                    "frame":{"task":"Reconsider","purpose":"Advice","scope":"garden"},
                    "evidence_fact_ids":["old-advice"],
                    "reading":{"verdict":"publish","reasons":[{
                        "text":"Previous agent said publish","evidence":["old-advice"]
                    }],"blockers":[],"unknowns":[]}
                }),
            ),
        )
        .unwrap_err();
        assert!(error.contains("observed or verified"));
        assert_eq!(rt.world.entities().len(), before);
    }

    fn record(rt: &mut crate::AgentRuntime, object: &str, color: &str) {
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "record_fact",
                json!({
                    "statement":format!("{object} is {color}"),
                    "payload":{"object":object,"color":color},
                    "source":"interpretation-demo"
                }),
            ),
        )
        .unwrap();
    }

    fn define_color_lens(rt: &mut crate::AgentRuntime) {
        crate::tools::lens_app::dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "define_lens_app",
                json!({"definition":{
                    "id":"demo.color",
                    "version":"1",
                    "purpose":"Project explicit colors from immutable facts",
                    "reads":["FactRecord"],
                    "stage":{"kind":"project","fields":{
                        "object":"/FactRecord/payload/object",
                        "color":"/FactRecord/payload/color"
                    }}
                }}),
            ),
        )
        .unwrap();
    }

    fn define_object_lens(rt: &mut crate::AgentRuntime) {
        crate::tools::lens_app::dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "define_lens_app",
                json!({"definition":{
                    "id":"demo.object",
                    "version":"1",
                    "purpose":"Project explicit objects from immutable facts",
                    "reads":["FactRecord"],
                    "stage":{"kind":"project","fields":{
                        "object":"/FactRecord/payload/object"
                    }}
                }}),
            ),
        )
        .unwrap();
    }

    fn materialize_color(rt: &mut crate::AgentRuntime) -> Value {
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "materialize_interpretation",
                json!({
                    "lens_id":"demo.color",
                    "frame":{
                        "task":"Find explicit colors",
                        "purpose":"Interpret the facts along the color dimension",
                        "scope":"all demo facts",
                        "parameters":{"only_explicit":true}
                    },
                    "relation":{
                        "predicate":"has_color",
                        "object_pointer":"/value/color"
                    }
                }),
            ),
        )
        .unwrap()
    }

    fn define_project_method(
        rt: &mut crate::AgentRuntime,
        script: &str,
        version: &str,
        expected_hash: Option<&str>,
    ) -> Value {
        let mut input = json!({"definition":{
            "id":"project.decision","version":version,"lens_id":"test.project",
            "verdicts":["continue","release_unknown"],"script":script
        }});
        if let Some(hash) = expected_hash {
            input["expected_hash"] = json!(hash);
        }
        crate::interpretation_method::define(&mut rt.world, &input).unwrap()
    }

    fn project_script() -> &'static str {
        r#"
let project = facts[0].value;
let source = facts[0].source_id;
if frame.parameters.goal == "continue" {
    #{verdict:"continue", reasons:[#{text:"Build passed", evidence:[source]}],
      blockers:[], unknowns:["Tests unobserved"],
      relations:[#{from_ref:project.project_id, predicate:"can_continue", to:true, evidence:[source]}]}
} else {
    #{verdict:"release_unknown", reasons:[#{text:"Build passed", evidence:[source]}],
      blockers:[], unknowns:["Deployment target unobserved", "E2E unobserved"], relations:[]}
}
"#
    }

    #[test]
    fn frame_changes_project_reading_and_method_version_invalidates_cache() {
        let mut rt = crate::AgentRuntime::new_for_test();
        dispatch(&mut rt.world, &rt.component_registry, &call("record_fact", json!({
            "statement":"Project build passed", "payload":{"project_id":"garden","build_status":"passed"},
            "source":"test build", "epistemic_status":"verified"
        }))).unwrap();
        crate::tools::lens_app::dispatch(&mut rt.world, &rt.component_registry, &call("define_lens_app", json!({"definition":{
            "id":"test.project","version":"1","purpose":"Project build evidence","reads":["FactRecord"],
            "stage":{"kind":"project","fields":{"project_id":"/FactRecord/payload/project_id","build_status":"/FactRecord/payload/build_status"}}
        }}))).unwrap();
        let defined = define_project_method(&mut rt, project_script(), "1", None);
        let run = |rt: &mut crate::AgentRuntime, goal: &str| {
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call(
                    "materialize_interpretation",
                    json!({
                        "lens_id":"test.project","method_id":"project.decision",
                        "frame":{"task":"Project decision","purpose":"Choose next action","scope":"garden","parameters":{"goal":goal}}
                    }),
                ),
            )
        };

        let continue_result = run(&mut rt, "continue").unwrap();
        assert_eq!(continue_result["cache_hit"], false);
        let cached = run(&mut rt, "continue").unwrap();
        assert_eq!(cached["cache_hit"], true);
        assert_eq!(cached["result_id"], continue_result["result_id"]);
        let release_result = run(&mut rt, "release").unwrap();
        assert_eq!(release_result["cache_hit"], false);
        let stream = rt
            .world
            .query::<&crate::interpretation_stream::InterpretationStream>()
            .iter(&rt.world)
            .find(|stream| stream.scope == "garden")
            .unwrap();
        assert_eq!(stream.pending_result_ids.len(), 2);
        let continue_output = inspect(
            &mut rt.world,
            &json!({"result_id":continue_result["result_id"]}),
        )
        .unwrap();
        let release_output = inspect(
            &mut rt.world,
            &json!({"result_id":release_result["result_id"]}),
        )
        .unwrap();
        assert_eq!(continue_output["result"]["output"]["verdict"], "continue");
        assert_eq!(
            release_output["result"]["output"]["verdict"],
            "release_unknown"
        );
        assert_ne!(
            continue_output["result"]["output"],
            release_output["result"]["output"]
        );
        let source_id = &continue_output["result"]["output"]["reasons"][0]["evidence"][0];
        assert!(continue_output["relations"].as_array().unwrap().iter().any(
            |relation| relation["predicate"] == "derived_from" && &relation["to"] == source_id
        ));
        assert!(
            continue_output["relations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|relation| relation["predicate"] == "can_continue" && relation["to"] == true)
        );

        define_project_method(
            &mut rt,
            project_script(),
            "2",
            defined["definition_hash"].as_str(),
        );
        let methods =
            crate::interpretation_method::inspect(&mut rt.world, &json!({"id":"project.decision"}))
                .unwrap();
        let versions = methods["methods"].as_array().unwrap();
        assert_eq!(versions.len(), 2);
        assert!(versions.iter().any(|method| {
            method["version"] == "1"
                && method["active"] == false
                && method["definition_hash"] == defined["definition_hash"]
        }));
        assert!(
            versions
                .iter()
                .any(|method| { method["version"] == "2" && method["active"] == true })
        );
        let after_update = run(&mut rt, "continue").unwrap();
        assert_eq!(after_update["cache_hit"], false);
        assert_ne!(after_update["result_id"], continue_result["result_id"]);

        let directory =
            std::env::temp_dir().join(format!("interpretation-method-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let checkpoint = directory.join("checkpoint.json");
        rt.checkpoint(&checkpoint).unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&checkpoint).unwrap();
        assert_eq!(run(&mut restored, "continue").unwrap()["cache_hit"], true);
        let restored_methods = crate::interpretation_method::inspect(
            &mut restored.world,
            &json!({"id":"project.decision"}),
        )
        .unwrap();
        assert_eq!(restored_methods["methods"], methods["methods"]);
        assert_eq!(
            crate::interpretation_method::find(&mut restored.world, "project.decision")
                .unwrap()
                .version,
            "2"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn method_failure_does_not_create_interpretation_artifacts() {
        let mut rt = crate::AgentRuntime::new_for_test();
        record(&mut rt, "apple", "green");
        define_color_lens(&mut rt);
        let input = json!({"lens_id":"demo.color","method_id":"test.invalid",
            "frame":{"task":"Check","purpose":"Validate evidence","scope":"apple"}});
        let bad_script = r#"#{verdict:"ok", reasons:[#{text:"Unsupported", evidence:["invented"]}], blockers:[], unknowns:[]}"#;
        crate::interpretation_method::define(&mut rt.world, &json!({"definition":{
            "id":"test.invalid","version":"1","lens_id":"demo.color","script":bad_script,"verdicts":["ok"]
        }})).unwrap();
        let before = rt.world.entities().len();
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("materialize_interpretation", input.clone())
            )
            .unwrap_err()
            .contains("evidence must reference")
        );
        assert_eq!(rt.world.entities().len(), before);

        let hash = crate::interpretation_method::find(&mut rt.world, "test.invalid")
            .unwrap()
            .definition_hash;
        crate::interpretation_method::define(
            &mut rt.world,
            &json!({"definition":{
            "id":"test.invalid","version":"2","lens_id":"demo.color",
            "script":"get_component(1, \"FactRecord\")","verdicts":["ok"]
        },"expected_hash":hash}),
        )
        .unwrap();
        assert!(
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call("materialize_interpretation", input)
            )
            .unwrap_err()
            .contains("Function not found")
        );
        assert_eq!(rt.world.entities().len(), before);
    }

    #[test]
    fn explicit_fact_reference_adds_provenance_only_when_fact_exists() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let fact = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "record_fact",
                json!({
                    "statement":"Build passed", "payload":{"exit_code":0}, "source":"build command"
                }),
            ),
        )
        .unwrap();
        let fact_id = fact["fact_id"].as_str().unwrap();
        let input = vec![InterpretationInput {
            source_id: "project-1".into(),
            source_version: "v1".into(),
        }];
        let output =
            vec![json!({"source_id":"project-1","value":{"build_evidence_fact_id":fact_id}})];
        let reading = crate::interpretation_method::Reading {
            verdict: "continue".into(),
            reasons: vec![crate::interpretation_method::Reason {
                text: "Build passed".into(),
                evidence: vec![fact_id.into()],
            }],
            blockers: vec![],
            unknowns: vec![],
            relations: vec![],
        };
        let (_, facts) = validate_reading(&mut rt.world, &reading, &input, &output).unwrap();
        assert_eq!(facts, vec![fact_id]);
        let missing =
            vec![json!({"source_id":"project-1","value":{"build_evidence_fact_id":"missing"}})];
        assert!(
            validate_reading(&mut rt.world, &reading, &input, &missing)
                .unwrap_err()
                .contains("unknown evidence FactRecord")
        );
    }

    #[test]
    fn laya_routing_maps_only_registered_lenses_and_never_materializes() {
        let mut rt = crate::AgentRuntime::new_for_test();
        define_color_lens(&mut rt);
        let entities_before = rt.world.entities().len();
        let (request, choices, selection) = prepare_laya_selection(
            &mut rt.world,
            &json!({
                "frame":{
                    "task":"Find the relevant dimension",
                    "purpose":"Choose a bounded interpretation",
                    "scope":"demo facts"
                },
                "state":{"question":"Which colors occur?"},
                "candidate_lens_ids":["demo.color"]
            }),
        )
        .unwrap();
        assert_eq!(request["candidates"][0]["operation"], "lens_0");
        assert!(
            request["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["description"].as_str().unwrap().chars().count() < 64)
        );
        assert!(choices.contains("lens_0"));
        assert!(choices.contains("main_agent"));
        assert!(choices.contains("defer"));
        let result = finish_laya_selection(json!({"operation":"lens_0"}), selection).unwrap();
        assert_eq!(result["interpretation_route"]["kind"], "existing_lens");
        assert_eq!(result["interpretation_route"]["lens_id"], "demo.color");
        assert_eq!(result["materialized"], false);
        assert_eq!(rt.world.entities().len(), entities_before);
        assert_eq!(
            rt.world
                .query::<&InterpretationResult>()
                .iter(&rt.world)
                .count(),
            0
        );
    }

    #[test]
    fn laya_routing_rejects_unknown_or_duplicate_lenses() {
        let mut rt = crate::AgentRuntime::new_for_test();
        define_color_lens(&mut rt);
        let input = |ids: Value| {
            json!({
                "frame":{"task":"t","purpose":"p","scope":"s"},
                "state":{},
                "candidate_lens_ids":ids
            })
        };
        assert!(
            prepare_laya_selection(&mut rt.world, &input(json!(["missing"])))
                .unwrap_err()
                .contains("unknown candidate Lens")
        );
        assert!(
            prepare_laya_selection(&mut rt.world, &input(json!(["demo.color", "demo.color"])))
                .unwrap_err()
                .contains("must be unique")
        );
    }

    #[test]
    fn laya_routing_candidate_mapping_is_independent_of_input_order() {
        let mut rt = crate::AgentRuntime::new_for_test();
        define_color_lens(&mut rt);
        define_object_lens(&mut rt);
        let input = |ids: Value| {
            json!({
                "frame":{"task":"List objects and colors","purpose":"compare lenses","scope":"facts"},
                "state":{"required_fields":["object","color"]},
                "candidate_lens_ids":ids
            })
        };

        let (forward, forward_choices, forward_selection) =
            prepare_laya_selection(&mut rt.world, &input(json!(["demo.object", "demo.color"])))
                .unwrap();
        let (reversed, reversed_choices, reversed_selection) =
            prepare_laya_selection(&mut rt.world, &input(json!(["demo.color", "demo.object"])))
                .unwrap();

        assert_eq!(forward, reversed);
        assert_eq!(forward_choices, reversed_choices);
        assert_eq!(forward_selection.routes, reversed_selection.routes);
        assert_eq!(
            forward["state"]["candidate_lenses"][0]["lens_id"],
            "demo.color"
        );
    }

    #[test]
    fn record_fact_rejects_json_encoded_string_payload_atomically() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let error = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "record_fact",
                json!({
                    "statement":"apple is green",
                    "payload":"{\"object\":\"apple\",\"color\":\"green\"}",
                    "source":"interpretation-demo"
                }),
            ),
        )
        .unwrap_err();

        assert!(error.contains("payload must be a JSON object"));
        assert_eq!(rt.world.query::<&FactRecord>().iter(&rt.world).count(), 0);
    }

    #[test]
    fn frame_parameters_reject_transport_string_coercion() {
        let mut rt = crate::AgentRuntime::new_for_test();
        define_color_lens(&mut rt);
        let error = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "materialize_interpretation",
                json!({
                    "lens_id":"demo.color",
                    "frame":{
                        "task":"colors",
                        "purpose":"reject transport coercion",
                        "scope":"demo",
                        "parameters":""
                    }
                }),
            ),
        )
        .unwrap_err();

        assert_eq!(error, "frame parameters must be a JSON object");
        assert_eq!(
            rt.world
                .query::<&InterpretationFrame>()
                .iter(&rt.world)
                .count(),
            0
        );
    }

    #[test]
    fn facts_interpretations_relations_and_exact_cache_form_a_closed_loop() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for (object, color) in [
            ("apple", "green"),
            ("basketball", "blue"),
            ("football", "orange"),
        ] {
            record(&mut rt, object, color);
        }
        define_color_lens(&mut rt);

        let first = materialize_color(&mut rt);
        assert_eq!(first["cache_hit"], false);
        assert_eq!(first["input_count"], 3);
        assert_eq!(first["relations_created"], 6);
        let first_result = first["result_id"].as_str().unwrap().to_owned();

        let second = materialize_color(&mut rt);
        assert_eq!(second["cache_hit"], true);
        assert_eq!(second["result_id"], first["result_id"]);
        assert_eq!(
            rt.world
                .query::<&InterpretationFrame>()
                .iter(&rt.world)
                .count(),
            1
        );
        assert_eq!(
            rt.world
                .query::<&InterpretationResult>()
                .iter(&rt.world)
                .count(),
            1
        );
        assert_eq!(
            rt.world
                .query::<&InterpretationRun>()
                .iter(&rt.world)
                .count(),
            2
        );
        assert_eq!(
            rt.world
                .query::<&RelationAssertion>()
                .iter(&rt.world)
                .count(),
            6
        );

        record(&mut rt, "leaf", "green");
        let third = materialize_color(&mut rt);
        assert_eq!(third["cache_hit"], false);
        assert_eq!(third["input_count"], 4);
        assert_ne!(third["result_id"].as_str().unwrap(), first_result);
        assert_eq!(
            rt.world
                .query::<&InterpretationResult>()
                .iter(&rt.world)
                .count(),
            2
        );
        let states: Vec<_> = rt
            .world
            .query::<&MaterializationState>()
            .iter(&rt.world)
            .collect();
        assert_eq!(states.len(), 2);
        assert_eq!(states.iter().filter(|state| state.valid).count(), 1);

        let old = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call("inspect_interpretations", json!({"result_id":first_result})),
        )
        .unwrap();
        assert_eq!(old["result"]["output"].as_array().unwrap().len(), 3);
        assert_eq!(old["runs"].as_array().unwrap().len(), 2);
        assert_eq!(old["relations"].as_array().unwrap().len(), 6);
        assert_eq!(old["summary"]["active_materializations"], 1);
        assert_eq!(old["summary"]["invalid_materializations"], 1);
    }

    #[test]
    fn invalid_relation_projection_is_atomic() {
        let mut rt = crate::AgentRuntime::new_for_test();
        record(&mut rt, "apple", "green");
        define_color_lens(&mut rt);
        let before = rt.world.entities().len();
        let error = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "materialize_interpretation",
                json!({
                    "lens_id":"demo.color",
                    "frame":{"task":"bad","purpose":"bad pointer","scope":"demo"},
                    "relation":{"predicate":"has_color","object_pointer":"/value/missing"}
                }),
            ),
        )
        .unwrap_err();
        assert!(error.contains("object_pointer missing"));
        // Lens observation may add RuntimeIdentity to the source, but no interpretation
        // artifact is allowed to leak from a rejected materialization.
        assert_eq!(
            rt.world
                .query::<&InterpretationFrame>()
                .iter(&rt.world)
                .count(),
            0
        );
        assert_eq!(
            rt.world
                .query::<&InterpretationRun>()
                .iter(&rt.world)
                .count(),
            0
        );
        assert_eq!(
            rt.world
                .query::<&InterpretationResult>()
                .iter(&rt.world)
                .count(),
            0
        );
        assert_eq!(
            rt.world
                .query::<&RelationAssertion>()
                .iter(&rt.world)
                .count(),
            0
        );
        assert!(rt.world.entities().len() >= before);
    }

    #[test]
    fn null_relation_target_is_rejected_atomically() {
        let mut rt = crate::AgentRuntime::new_for_test();
        dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "record_fact",
                json!({
                    "statement":"book is an object with unknown color",
                    "payload":{"object":"book"},
                    "source":"interpretation-demo"
                }),
            ),
        )
        .unwrap();
        define_color_lens(&mut rt);

        let error = dispatch(
            &mut rt.world,
            &rt.component_registry,
            &call(
                "materialize_interpretation",
                json!({
                    "lens_id":"demo.color",
                    "frame":{"task":"colors","purpose":"reject unknown colors","scope":"demo"},
                    "relation":{"predicate":"has_color","object_pointer":"/value/color"}
                }),
            ),
        )
        .unwrap_err();

        assert!(error.contains("object_pointer resolved to null"));
        assert_eq!(
            rt.world
                .query::<&InterpretationResult>()
                .iter(&rt.world)
                .count(),
            0
        );
        assert_eq!(
            rt.world
                .query::<&RelationAssertion>()
                .iter(&rt.world)
                .count(),
            0
        );
        assert_eq!(
            rt.world
                .query::<&MaterializationState>()
                .iter(&rt.world)
                .count(),
            0
        );
    }

    #[test]
    fn materialization_cache_canonicalizes_parameter_key_order() {
        let mut rt = crate::AgentRuntime::new_for_test();
        record(&mut rt, "apple", "green");
        define_color_lens(&mut rt);
        let materialize = |rt: &mut crate::AgentRuntime, parameters: Value| {
            dispatch(
                &mut rt.world,
                &rt.component_registry,
                &call(
                    "materialize_interpretation",
                    json!({
                        "lens_id":"demo.color",
                        "frame":{
                            "task":"Find explicit colors",
                            "purpose":"Verify canonical caching",
                            "scope":"all demo facts",
                            "parameters":parameters
                        }
                    }),
                ),
            )
            .unwrap()
        };
        let first_parameters: Value =
            serde_json::from_str(r#"{"outer":{"beta":2,"alpha":1},"mode":"strict"}"#).unwrap();
        let reordered_parameters: Value =
            serde_json::from_str(r#"{"mode":"strict","outer":{"alpha":1,"beta":2}}"#).unwrap();

        let first = materialize(&mut rt, first_parameters);
        let reordered = materialize(&mut rt, reordered_parameters);
        let changed = materialize(
            &mut rt,
            json!({"mode":"strict","outer":{"alpha":1,"beta":3}}),
        );

        assert_eq!(first["cache_hit"], false);
        assert_eq!(reordered["cache_hit"], true);
        assert_eq!(reordered["cache_key"], first["cache_key"]);
        assert_eq!(reordered["result_id"], first["result_id"]);
        assert_eq!(changed["cache_hit"], false);
        assert_ne!(changed["cache_key"], first["cache_key"]);
        assert_ne!(changed["result_id"], first["result_id"]);
    }

    #[test]
    #[ignore = "manual interpretation cache performance measurement"]
    fn measure_interpretation_cold_and_hot_cache() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for index in 0..200 {
            record(&mut rt, &format!("object-{index}"), "green");
        }
        define_color_lens(&mut rt);

        let cold_started = std::time::Instant::now();
        let cold = materialize_color(&mut rt);
        let cold_elapsed = cold_started.elapsed();
        let hot_started = std::time::Instant::now();
        let hot = materialize_color(&mut rt);
        let hot_elapsed = hot_started.elapsed();

        assert_eq!(cold["cache_hit"], false);
        assert_eq!(hot["cache_hit"], true);
        assert_eq!(hot["result_id"], cold["result_id"]);
        println!(
            "interpretation_cache facts=200 cold_us={} hot_us={} ratio={:.2}",
            cold_elapsed.as_micros(),
            hot_elapsed.as_micros(),
            cold_elapsed.as_secs_f64() / hot_elapsed.as_secs_f64()
        );
    }

    #[test]
    fn materialized_result_and_cache_survive_restore() {
        let mut rt = crate::AgentRuntime::new_for_test();
        record(&mut rt, "apple", "green");
        define_color_lens(&mut rt);
        let first = materialize_color(&mut rt);
        let directory =
            std::env::temp_dir().join(format!("interpretation-demo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let checkpoint = directory.join("checkpoint.json");
        rt.checkpoint(&checkpoint).unwrap();

        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&checkpoint).unwrap();
        let second = materialize_color(&mut restored);
        assert_eq!(second["cache_hit"], true);
        assert_eq!(second["result_id"], first["result_id"]);
        assert_eq!(
            restored
                .world
                .query::<&InterpretationResult>()
                .iter(&restored.world)
                .count(),
            1
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
