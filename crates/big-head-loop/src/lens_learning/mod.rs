//! Agent-directed Lens learning. Candidates are isolated until verified adoption.
mod discovery;
#[cfg(test)]
mod tests;
mod trial;

use bevy_ecs::{entity::Entity, world::World};
use big_head_ai::{ToolCall, ToolDefinition};
use big_head_ecs::{
    ComponentActionRegistry,
    execution::DispatchContext,
    lens::{LensApplication, LensCandidate, LensCandidateStatus, LensFeedback, LensFeedbackKind},
};
use big_head_world::application::{self, LensDefinition};
use serde_json::{Value, json};

const MAX_CANDIDATES: usize = 32;
const MAX_FEEDBACK: usize = 256;

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    let mut definitions = vec![
        tool(
            "discover_lenses",
            "Find published Lenses by query task words OR explicit components. Component anchors match reads and presence dependencies (including referenced Lenses) and rank before lexical matches. Call at any wave after learning what components are relevant. Bounded suggestions, not applicability guarantees; does not execute or activate a Lens.",
            json!({"query":{"type":"string"},"components":{"type":"array","maxItems":32,"items":{"type":"string"}},"limit":{"type":"integer","minimum":1,"maximum":8}}),
            &[],
        ),
        tool(
            "record_lens_feedback",
            "Record an agent assessment of a published Lens: useful, gap, or correction. Preserve the actual user's correction in note; runtime links current Spark identity. This is an assessment, not verified fact. No automatic revision or Spark.",
            json!({"lens_id":{"type":"string"},"kind":{"type":"string","enum":["useful","gap","correction"]},"note":{"type":"string"}}),
            &["lens_id", "kind", "note"],
        ),
        tool(
            "propose_lens_candidate",
            "Propose a new or revised read-only Lens using a NEW definition.id; published definitions/bindings are untouched. Same LensDefinition DSL as define_lens_app, optional usage={tasks:[],tags:[],limitations:[]}. Optional based_on={id,version}; reason required; feedback_ids link recorded feedback for that baseline. Updating an unadopted candidate requires expected_revision and clears its trial. No automatic activation.",
            json!({"definition":{"type":"object"},"based_on":{"type":"object","properties":{"id":{"type":"string"},"version":{"type":"string"}},"required":["id","version"],"additionalProperties":false},"reason":{"type":"string"},"feedback_ids":{"type":"array","items":{"type":"string"},"maxItems":8},"expected_revision":{"type":"integer","minimum":1}}),
            &["definition", "reason"],
        ),
        tool(
            "trial_lens_candidate",
            "Test a candidate and baseline on the same captured ECS inputs. checks={min_facts:1,required_fields:[JSON pointers relative to each output value],max_output_bytes:2048}; at least one output is required. Measures bytes/fact count/nulls, verifies required non-null fields, records failure. Structural success is not semantic quality. No publishing or binding change.",
            json!({"candidate_id":{"type":"string"},"checks":{"type":"object","properties":{"min_facts":{"type":"integer","minimum":1,"maximum":256},"required_fields":{"type":"array","items":{"type":"string"}},"max_output_bytes":{"type":"integer","minimum":1,"maximum":65536}},"additionalProperties":false}}),
            &["candidate_id"],
        ),
        tool(
            "adopt_lens_candidate",
            "Publish the candidate as a new Lens after a passing trial. Requires trial_id and agent reason explaining task suitability; reruns checks and rejects changed source/catalog evidence. Old Lens and all bindings remain unchanged. This does not authorize new actions or enable persistent observation; configure a binding separately only when user intent calls for it.",
            json!({"candidate_id":{"type":"string"},"trial_id":{"type":"string"},"reason":{"type":"string"}}),
            &["candidate_id", "trial_id", "reason"],
        ),
        tool(
            "inspect_lens_learning",
            "Inspect candidate status/trial summary and agent-authored feedback. candidate_id selects full candidate definition; details=true also includes latest trial output. Only latest trial per candidate is retained. lens_id filters feedback. limit defaults 16, max 32.",
            json!({"candidate_id":{"type":"string"},"lens_id":{"type":"string"},"details":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":32}}),
            &[],
        ),
    ];
    definitions[2].input_schema["properties"]["definition"] =
        crate::lens_contract::definition_schema();
    definitions[2].input_schema["$defs"] = crate::lens_contract::stage_definitions();
    definitions
}

fn text<'a>(value: &'a Value, key: &str, max: usize) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= max)
        .ok_or_else(|| format!("{key} requires a nonempty string of at most {max} bytes"))
}

fn limit(input: &Value, default: usize, max: usize) -> Result<usize, String> {
    match input.get("limit") {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| *n > 0 && *n <= max as u64)
            .map(|n| n as usize)
            .ok_or_else(|| format!("limit must be 1..{max}")),
    }
}

fn candidates(world: &mut World) -> Result<Vec<(Entity, LensCandidate)>, String> {
    let mut entries: Vec<_> = world
        .query::<(Entity, &LensCandidate)>()
        .iter(world)
        .map(|(e, c)| (e, c.clone()))
        .collect();
    entries.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    if entries.len() > MAX_CANDIDATES || entries.windows(2).any(|p| p[0].1.id == p[1].1.id) {
        return Err("duplicate candidates or candidate catalog exceeds 32 entries".into());
    }
    Ok(entries)
}

fn feedback(world: &mut World, id: Option<&str>, limit: usize) -> Vec<LensFeedback> {
    let mut notes: Vec<_> = world
        .query::<&LensFeedback>()
        .iter(world)
        .filter(|f| id.is_none_or(|id| id == f.lens_id))
        .cloned()
        .collect();
    notes.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    notes.truncate(limit);
    notes
}

pub(crate) fn task_context(world: &mut World, query: &str) -> String {
    // Discovery is bounded metadata lookup, not another model invocation.
    let suggestions = match crate::lens_runtime::definition_catalog(world) {
        Ok(catalog) => discovery::context(&catalog, &query.chars().take(2000).collect::<String>()),
        Err(error) => format!(
            "\nLens capability discovery unavailable: {}\n",
            error.chars().take(256).collect::<String>()
        ),
    };
    format!(
        "\n## Lens Workflow\nReuse relevant observations first. For repeated exploration, an actual context gap or user correction: discover_lenses, record_lens_feedback, propose_lens_candidate, trial_lens_candidate, then adopt_lens_candidate only after reviewing evidence and limitations. Do not create a Lens for every task. Prefer user intent over jargon; never ask users to specify ECS fields. A trial checks structure, not truth. Adoption never enables a binding; persistent observation requires explicit intent. Do not push Sparks to improve Lenses.\n{suggestions}"
    )
}

pub(crate) fn dispatch(
    world: &mut World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> Result<String, String> {
    if call.input.to_string().len() > 70000 {
        return Err("Lens learning input exceeds budget".into());
    }
    let result = match call.name.as_str() {
        "discover_lenses" => {
            let query = if call.input.get("query").is_some() {
                text(&call.input, "query", 4096)?
            } else {
                ""
            };
            let components: Vec<String> =
                serde_json::from_value(call.input.get("components").cloned().unwrap_or(json!([])))
                    .map_err(|e| format!("components: {e}"))?;
            if components.len() > 32
                || components
                    .iter()
                    .any(|s| s.trim().is_empty() || s.len() > 256)
                || (query.is_empty() && components.is_empty())
            {
                return Err(
                    "provide query or 1..32 nonempty component names (<=256 bytes each)".into(),
                );
            }
            let catalog = crate::lens_runtime::definition_catalog(world)?;
            let found = discovery::find(&catalog, query, &components, limit(&call.input, 5, 8)?);
            let notes: Vec<_> = found
                .iter()
                .flat_map(|f| feedback(world, f["id"].as_str(), 2))
                .collect();
            json!({"matches":found,"feedback":notes,"feedback_is":"agent assessment, not verified truth"})
        }
        "record_lens_feedback" => record_feedback(world, &call.input)?,
        "propose_lens_candidate" => propose(world, &call.input)?,
        "trial_lens_candidate" => test_candidate(world, registry, &call.input)?,
        "adopt_lens_candidate" => adopt(world, registry, &call.input)?,
        "inspect_lens_learning" => inspect(world, &call.input)?,
        _ => return Err("unknown Lens learning tool".into()),
    };
    Ok(result.to_string())
}

fn record_feedback(world: &mut World, input: &Value) -> Result<Value, String> {
    let id = text(input, "lens_id", 128)?;
    let note = text(input, "note", 1024)?;
    let kind: LensFeedbackKind =
        serde_json::from_value(input.get("kind").cloned().ok_or("kind required")?)
            .map_err(|e| e.to_string())?;
    let catalog = crate::lens_runtime::definition_catalog(world)?;
    let definition = catalog.get(id).ok_or("unknown published Lens")?;
    let context = world.get_resource::<DispatchContext>();
    let source_id = context.map(|c| c.source_id.clone()).unwrap_or_default();
    let correlation_id = context
        .map(|c| c.correlation_id.clone())
        .unwrap_or_default();
    let previous: Vec<_> = world
        .query::<&LensFeedback>()
        .iter(world)
        .cloned()
        .collect();
    if let Some(old) = previous.iter().find(|f| {
        f.lens_id == id
            && f.lens_version == definition.version
            && f.note == note
            && f.source_id == source_id
            && f.kind == kind
    }) {
        return Ok(json!({"feedback":old,"reused":true}));
    }
    if previous.len() >= MAX_FEEDBACK {
        return Err("Lens feedback store full (256); archive explicitly before adding more".into());
    }
    let entry = LensFeedback {
        id: uuid::Uuid::new_v4().to_string(),
        lens_id: id.into(),
        lens_version: definition.version.clone(),
        kind,
        note: note.into(),
        source_id,
        correlation_id,
        created_at: big_head_ecs::execution::now_secs(),
    };
    world.spawn(entry.clone());
    Ok(json!({"feedback":entry,"reused":false}))
}

fn propose(world: &mut World, input: &Value) -> Result<Value, String> {
    let raw = input.get("definition").ok_or("definition required")?;
    let (raw, _) = crate::lens_contract::normalize_definition(raw)?;
    let definition: LensDefinition =
        serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
    if definition.id.trim().is_empty() || definition.id.len() > 128 {
        return Err("candidate id requires 1..128 bytes".into());
    }
    let reason = text(input, "reason", 1024)?.to_string();
    let feedback_ids: Vec<String> =
        serde_json::from_value(input.get("feedback_ids").cloned().unwrap_or(json!([])))
            .map_err(|e| e.to_string())?;
    if feedback_ids.len() > 8 {
        return Err("at most 8 feedback references".into());
    }
    let mut catalog = crate::lens_runtime::definition_catalog(world)?;
    if catalog.contains_key(&definition.id) {
        return Err("use a NEW Lens id; candidates cannot overwrite published definitions".into());
    }
    let (base_id, base_version) = if let Some(base) = input.get("based_on") {
        let id = text(base, "id", 128)?;
        let version = text(base, "version", 128)?;
        if catalog.get(id).is_none_or(|d| d.version != version) {
            return Err("baseline version mismatch".into());
        }
        (Some(id.to_string()), Some(version.to_string()))
    } else {
        (None, None)
    };
    let notes = feedback(world, base_id.as_deref(), MAX_FEEDBACK);
    for id in &feedback_ids {
        if !notes.iter().any(|f| {
            &f.id == id
                && Some(&f.lens_id) == base_id.as_ref()
                && Some(&f.lens_version) == base_version.as_ref()
        }) {
            return Err("feedback reference must exist and match the baseline Lens/version".into());
        }
    }
    catalog.insert(definition.id.clone(), definition.clone());
    application::validate(&definition.id, &catalog)?;
    let entries = candidates(world)?;
    let old = entries.iter().find(|(_, c)| c.id == definition.id);
    let revision = match old {
        Some((_, c)) => {
            if c.status == LensCandidateStatus::Adopted
                || input.get("expected_revision").and_then(Value::as_u64) != Some(c.revision)
            {
                return Err(
                    "candidate update requires matching expected_revision and unadopted status"
                        .into(),
                );
            }
            c.revision
                .checked_add(1)
                .ok_or("candidate revision overflow")?
        }
        None => {
            if input.get("expected_revision").is_some() || entries.len() >= MAX_CANDIDATES {
                return Err("invalid candidate create precondition or catalog full".into());
            }
            1
        }
    };
    let candidate = LensCandidate {
        id: definition.id.clone(),
        definition_json: raw.to_string(),
        base_id,
        base_version,
        reason,
        feedback_ids,
        revision,
        status: LensCandidateStatus::Proposed,
        trial_json: String::new(),
        adoption_reason: String::new(),
    };
    if let Some((entity, _)) = old {
        world.entity_mut(*entity).insert(candidate.clone());
    } else {
        world.spawn(candidate.clone());
    }
    Ok(json!({"candidate_id":candidate.id,"revision":revision,"status":candidate.status}))
}

fn candidate(world: &mut World, input: &Value) -> Result<(Entity, LensCandidate), String> {
    let id = text(input, "candidate_id", 128)?;
    candidates(world)?
        .into_iter()
        .find(|(_, c)| c.id == id)
        .ok_or_else(|| "unknown candidate".into())
}

fn test_candidate(
    world: &mut World,
    registry: &ComponentActionRegistry,
    input: &Value,
) -> Result<Value, String> {
    let (entity, mut candidate) = candidate(world, input)?;
    if candidate.status == LensCandidateStatus::Adopted {
        return Err("candidate already adopted; run the published Lens".into());
    }
    let checks: trial::Checks =
        serde_json::from_value(input.get("checks").cloned().unwrap_or(json!({})))
            .map_err(|e| e.to_string())?;
    checks.validate()?;
    let mut result = trial::run(world, registry, &candidate, checks.clone())
        .unwrap_or_else(|e| trial::Trial::failed(candidate.revision, checks.clone(), e));
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > 524288
    {
        result = trial::Trial::failed(
            candidate.revision,
            checks,
            "trial receipt exceeds 512 KiB; narrow candidate".into(),
        );
    }
    candidate.status = if result.passed {
        LensCandidateStatus::Passed
    } else {
        LensCandidateStatus::Failed
    };
    candidate.trial_json = serde_json::to_string(&result).map_err(|e| e.to_string())?;
    world.entity_mut(entity).insert(candidate);
    Ok(result.summary())
}

fn adopt(
    world: &mut World,
    registry: &ComponentActionRegistry,
    input: &Value,
) -> Result<Value, String> {
    let (entity, mut candidate) = candidate(world, input)?;
    let trial_id = text(input, "trial_id", 128)?;
    let reason = text(input, "reason", 1024)?;
    let previous: trial::Trial =
        serde_json::from_str(&candidate.trial_json).map_err(|_| "candidate has no valid trial")?;
    if candidate.status == LensCandidateStatus::Adopted && previous.id == trial_id {
        let (entries, catalog) = crate::lens_runtime::catalog(world)?;
        if entries.iter().any(|(_, app)| {
            app.id == candidate.id && app.definition_json == candidate.definition_json
        }) {
            return Ok(
                json!({"lens_id":candidate.id,"version":catalog[&candidate.id].version,"status":"adopted","bindings_changed":false,"trial_id":previous.id,"reused":true}),
            );
        }
        return Err("adopted definition changed or disappeared; inspect before retrying".into());
    }
    if candidate.status != LensCandidateStatus::Passed
        || !previous.passed
        || previous.id != trial_id
        || previous.candidate_revision != candidate.revision
    {
        return Err(
            "adoption requires the latest passing trial for this candidate revision".into(),
        );
    }
    let fresh = trial::run(world, registry, &candidate, previous.checks.clone())?;
    if !fresh.passed || fresh.evidence_key != previous.evidence_key {
        return Err("trial evidence changed; run a new trial and review before adoption".into());
    }
    let mut catalog = crate::lens_runtime::definition_catalog(world)?;
    if catalog.len() >= 128 {
        return Err("published Lens catalog full".into());
    }
    let definition: LensDefinition =
        serde_json::from_str(&candidate.definition_json).map_err(|e| e.to_string())?;
    catalog.insert(candidate.id.clone(), definition);
    for id in catalog.keys() {
        application::validate(id, &catalog)?;
    }
    world.spawn(LensApplication {
        id: candidate.id.clone(),
        definition_json: candidate.definition_json.clone(),
        last_run_json: String::new(),
    });
    candidate.status = LensCandidateStatus::Adopted;
    candidate.adoption_reason = reason.into();
    world.entity_mut(entity).insert(candidate.clone());
    Ok(
        json!({"lens_id":candidate.id,"version":catalog[&candidate.id].version,"status":"adopted","bindings_changed":false,"trial_id":previous.id}),
    )
}

fn inspect(world: &mut World, input: &Value) -> Result<Value, String> {
    let selected = input.get("candidate_id").and_then(Value::as_str);
    let details = input
        .get("details")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if details && selected.is_none() {
        return Err("details=true requires candidate_id to bound trial output".into());
    }
    let entries = candidates(world)?;
    if selected.is_some_and(|id| !entries.iter().any(|(_, c)| c.id == id)) {
        return Err("unknown candidate".into());
    }
    let result: Vec<_> = entries.iter().filter(|(_,c)|selected.is_none_or(|id|c.id == id)).take(limit(input,16,32)?).map(|(_,c)| {
        let trial: Option<trial::Trial> = serde_json::from_str(&c.trial_json).ok();
        json!({"id":c.id,"revision":c.revision,"status":c.status,"based_on":c.base_id,"reason":c.reason,"feedback_ids":c.feedback_ids,"adoption_reason":c.adoption_reason,
            "definition":if selected.is_some() {serde_json::from_str::<Value>(&c.definition_json).unwrap_or(Value::Null)} else {Value::Null},
            "trial":if details {serde_json::to_value(&trial).unwrap()} else {trial.as_ref().map(|t|t.summary()).unwrap_or(Value::Null)}})
    }).collect();
    Ok(
        json!({"candidates":result,"feedback":feedback(world,input.get("lens_id").and_then(Value::as_str),limit(input,16,32)?)}),
    )
}
