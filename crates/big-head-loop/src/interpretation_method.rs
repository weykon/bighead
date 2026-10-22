//! Pure, bounded methods that turn a Lens working set and a Frame into a reading.

use bevy_ecs::{component::Component, world::World};
use rhai::{Dynamic, Engine, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_SCRIPT_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InterpretationMethod {
    pub id: String,
    pub version: String,
    pub lens_id: String,
    pub script: String,
    pub verdicts: Vec<String>,
    pub definition_hash: String,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefineRequest {
    definition: MethodDefinition,
    #[serde(default)]
    expected_hash: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MethodDefinition {
    pub id: String,
    pub version: String,
    pub lens_id: String,
    pub script: String,
    pub verdicts: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reason {
    pub text: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelativeRelation {
    pub from_ref: String,
    pub predicate: String,
    pub to: Value,
    pub evidence: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reading {
    pub verdict: String,
    pub reasons: Vec<Reason>,
    pub blockers: Vec<String>,
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub relations: Vec<RelativeRelation>,
}

pub fn definitions() -> Vec<big_head_ai::ToolDefinition> {
    vec![
        big_head_ai::ToolDefinition {
            name: "define_interpretation_method".into(),
            description: "Define or replace a pure Rhai interpretation method. It receives frame and facts (bounded Lens output), returns verdict/reasons/blockers/unknowns/relations, and cannot access ECS. Updating an existing method requires expected_hash.".into(),
            input_schema: json!({"type":"object","properties":{
                "definition":{"type":"object","properties":{
                    "id":{"type":"string"},"version":{"type":"string"},"lens_id":{"type":"string"},
                    "script":{"type":"string"},"verdicts":{"type":"array","items":{"type":"string"}}
                },"required":["id","version","lens_id","script","verdicts"],"additionalProperties":false},
                "expected_hash":{"type":"string"}
            },"required":["definition"],"additionalProperties":false}),
        },
        big_head_ai::ToolDefinition {
            name: "inspect_interpretation_methods".into(),
            description: "List registered interpretation methods and their definition hashes; optionally select one id.".into(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"}},"additionalProperties":false}),
        },
    ]
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(30_000)
        .set_max_call_levels(16)
        .set_max_expr_depths(32, 16)
        .set_max_string_size(16_384)
        .set_max_array_size(256)
        .set_max_map_size(128)
        .set_module_resolver(rhai::module_resolvers::StaticModuleResolver::new());
    engine.disable_symbol("eval");
    engine.disable_symbol("import");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine
}

pub fn validate_definition(definition: &MethodDefinition) -> Result<(), String> {
    for (name, value) in [
        ("id", &definition.id),
        ("version", &definition.version),
        ("lens_id", &definition.lens_id),
    ] {
        if value.trim().is_empty() || value.len() > 128 {
            return Err(format!("{name} must contain 1..128 bytes"));
        }
    }
    if definition.script.trim().is_empty() || definition.script.len() > MAX_SCRIPT_BYTES {
        return Err("script must contain 1..16384 bytes".into());
    }
    if definition.verdicts.is_empty()
        || definition.verdicts.len() > 16
        || definition
            .verdicts
            .iter()
            .any(|v| v.trim().is_empty() || v.len() > 128)
    {
        return Err("verdicts must contain 1..16 nonempty values".into());
    }
    let unique: std::collections::HashSet<_> = definition.verdicts.iter().collect();
    if unique.len() != definition.verdicts.len() {
        return Err("verdicts must be unique".into());
    }
    engine()
        .compile(&definition.script)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn define(world: &mut World, input: &Value) -> Result<Value, String> {
    let request: DefineRequest =
        serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    validate_definition(&request.definition)?;
    let catalog = crate::lens_runtime::definition_catalog(world)?;
    if !catalog.contains_key(&request.definition.lens_id) {
        return Err("unknown Lens for interpretation method".into());
    }
    let bytes = serde_json::to_vec(&request.definition).map_err(|e| e.to_string())?;
    let hash = big_head_save::persistence::digest(&bytes);
    let versions: Vec<_> = world
        .query::<(bevy_ecs::entity::Entity, &InterpretationMethod)>()
        .iter(world)
        .filter(|(_, method)| method.id == request.definition.id)
        .map(|(entity, method)| {
            (
                entity,
                method.definition_hash.clone(),
                method.version.clone(),
                method.active,
            )
        })
        .collect();
    let active: Vec<_> = versions
        .iter()
        .filter(|(_, _, _, active)| *active)
        .collect();
    if active.len() > 1 {
        return Err("duplicate active interpretation method id".into());
    }
    if versions
        .iter()
        .any(|(_, _, version, _)| version == &request.definition.version)
    {
        return Err("interpretation method version already exists".into());
    }
    if let Some((entity, old_hash, _, _)) = active.first() {
        if request.expected_hash.as_deref() != Some(old_hash) {
            return Err("interpretation method hash changed; inspect before updating".into());
        }
        world
            .get_mut::<InterpretationMethod>(*entity)
            .unwrap()
            .active = false;
    } else {
        if !versions.is_empty() {
            return Err("interpretation method has no active version".into());
        }
        if request.expected_hash.is_some() {
            return Err("expected_hash supplied for a new method".into());
        }
    }
    let entity = world
        .spawn(InterpretationMethod {
            id: request.definition.id.clone(),
            version: request.definition.version,
            lens_id: request.definition.lens_id,
            script: request.definition.script,
            verdicts: request.definition.verdicts,
            definition_hash: hash.clone(),
            active: true,
        })
        .id();
    Ok(
        json!({"id":request.definition.id,"entity_id":entity.index().index(),"definition_hash":hash}),
    )
}

pub fn inspect(world: &mut World, input: &Value) -> Result<Value, String> {
    let id = input.get("id").and_then(Value::as_str);
    let methods: Vec<_> = world
        .query::<&InterpretationMethod>()
        .iter(world)
        .filter(|method| id.is_none_or(|id| method.id == id))
        .map(|method| {
            json!({"id":method.id,"version":method.version,"lens_id":method.lens_id,
            "verdicts":method.verdicts,"definition_hash":method.definition_hash,"active":method.active})
        })
        .collect();
    Ok(json!({"methods":methods}))
}

pub fn find(world: &mut World, id: &str) -> Result<InterpretationMethod, String> {
    let methods: Vec<_> = world
        .query::<&InterpretationMethod>()
        .iter(world)
        .filter(|method| method.id == id && method.active)
        .cloned()
        .collect();
    match methods.as_slice() {
        [method] => Ok(method.clone()),
        [] => Err("unknown interpretation method".into()),
        _ => Err("duplicate active interpretation method id".into()),
    }
}

pub fn evaluate(
    method: &InterpretationMethod,
    frame: &Value,
    facts: &[Value],
) -> Result<Reading, String> {
    let engine = engine();
    let ast = engine.compile(&method.script).map_err(|e| e.to_string())?;
    let mut scope = Scope::new();
    scope.push_dynamic(
        "frame",
        rhai::serde::to_dynamic(frame).map_err(|e| e.to_string())?,
    );
    scope.push_dynamic(
        "facts",
        rhai::serde::to_dynamic(facts).map_err(|e| e.to_string())?,
    );
    let result = engine
        .eval_ast_with_scope::<Dynamic>(&mut scope, &ast)
        .map_err(|e| format!("interpretation method: {e}"))?;
    let value: Value = rhai::serde::from_dynamic(&result).map_err(|e| e.to_string())?;
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_OUTPUT_BYTES {
        return Err("interpretation output exceeds 64 KiB".into());
    }
    let reading: Reading =
        serde_json::from_value(value).map_err(|e| format!("invalid interpretation output: {e}"))?;
    if !method.verdicts.contains(&reading.verdict) {
        return Err("interpretation verdict is not declared by the method".into());
    }
    if reading.reasons.len() > 64
        || reading.blockers.len() > 64
        || reading.unknowns.len() > 64
        || reading.relations.len() > 128
    {
        return Err("interpretation output has too many entries".into());
    }
    Ok(reading)
}
