//! One contract for agent tools, dry-run diagnosis, and Rhai host calls.
use big_head_world::application::{self, Catalog, LensDefinition};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefinitionRequest {
    pub definition: LensDefinition,
    pub expected_version: Option<String>,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

pub(crate) fn definition_schema() -> Value {
    object(
        json!({
            "id":{"type":"string","minLength":1},
            "version":{"type":"string","minLength":1,"description":"Canonical version is a string, e.g. \"1\"."},
            "purpose":{"type":"string","minLength":1},
            "reads":{"type":"array","maxItems":32,"items":{"type":"string","minLength":1}},
            "usage":object(json!({
                "tasks":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":256}},
                "tags":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":256}},
                "limitations":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":256}}
            }), &[]),
            "stage":{"$ref":"#/$defs/Stage"}
        }),
        &["id", "version", "purpose", "reads", "stage"],
    )
}

pub(crate) fn stage_definitions() -> Value {
    let variant = |kind: &str, props: Value, required: &[&str]| {
        let mut props = props.as_object().unwrap().clone();
        props.insert("kind".into(), json!({"type":"string","enum":[kind]}));
        let mut required = required.to_vec();
        required.push("kind");
        object(Value::Object(props), &required)
    };
    json!({"Stage":{"oneOf":[
        variant("identity",json!({}), &[]),
        variant("archetype_with",json!({"component":{"type":"string","minLength":1,"maxLength":256}}), &["component"]),
        variant("archetype_without",json!({"component":{"type":"string","minLength":1,"maxLength":256}}), &["component"]),
        variant("filter",json!({"pointer":{"type":"string"},"equals":{}}), &["pointer","equals"]),
        variant("project",json!({"fields":{"type":"object","maxProperties":64,"additionalProperties":{"type":"string"}}}), &["fields"]),
        variant("take",json!({"limit":{"type":"integer","minimum":0,"maximum":256}}), &["limit"]),
        variant("sequence",json!({"stages":{"type":"array","maxItems":128,"items":{"$ref":"#/$defs/Stage"}}}), &["stages"]),
        variant("parallel",json!({"branches":{"type":"object","maxProperties":128,"additionalProperties":{"$ref":"#/$defs/Stage"}}}), &["branches"]),
        variant("lens",json!({"id":{"type":"string","minLength":1},"version":{"type":"string","minLength":1}}), &["id","version"])
    ]}})
}

pub(crate) fn schema() -> Value {
    let mut schema = object(
        json!({"definition":definition_schema(),"expected_version":{"type":"string","description":"Update precondition; belongs outside definition."}}),
        &["definition"],
    );
    schema["$defs"] = stage_definitions();
    schema
}

fn version(value: &mut Value, path: &str, notes: &mut Vec<String>) {
    if let Some(n) = value.as_u64() {
        *value = json!(n.to_string());
        notes.push(format!("{path}: integer version normalized to string"));
    }
}

// Some tool providers serialize JSON arrays as objects with numeric keys and
// integer fields as strings. Normalize only fields whose contract explicitly
// requires an array/integer; ordinary objects remain strict and unchanged.
fn indexed_array(value: &mut Value, path: &str, notes: &mut Vec<String>) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let mut items = Vec::with_capacity(object.len());
    for (key, item) in object {
        let index = key
            .parse::<usize>()
            .map_err(|_| format!("{path} must be an array"))?;
        items.push((index, item.clone()));
    }
    items.sort_by_key(|(index, _)| *index);
    if items
        .iter()
        .enumerate()
        .any(|(expected, (actual, _))| *actual != expected)
    {
        return Err(format!(
            "{path} numeric array keys must be contiguous from 0"
        ));
    }
    *value = Value::Array(items.into_iter().map(|(_, item)| item).collect());
    notes.push(format!("{path}: numeric-key object normalized to array"));
    Ok(())
}

fn array_field(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    path: &str,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(value) = object.get_mut(key) {
        indexed_array(value, path, notes)?;
    }
    Ok(())
}

fn normalize_stage(
    stage: &mut Value,
    budget: &mut usize,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    *budget = budget.checked_sub(1).ok_or("Lens exceeds 128 stages")?;
    match stage.get("kind").and_then(Value::as_str) {
        Some("lens") => {
            if let Some(v) = stage.get_mut("version") {
                version(v, "stage.version", notes);
            }
        }
        Some("sequence") => {
            if let Some(stages) = stage.get_mut("stages") {
                indexed_array(stages, "stage.stages", notes)?;
            }
            if let Some(stages) = stage.get_mut("stages").and_then(Value::as_array_mut) {
                for stage in stages {
                    normalize_stage(stage, budget, notes)?;
                }
            }
        }
        Some("parallel") => {
            if let Some(branches) = stage.get_mut("branches").and_then(Value::as_object_mut) {
                for stage in branches.values_mut() {
                    normalize_stage(stage, budget, notes)?;
                }
            }
        }
        Some("take") => {
            if let Some(limit) = stage.get_mut("limit") {
                if let Some(text) = limit.as_str() {
                    let parsed = text
                        .parse::<u64>()
                        .map_err(|_| "take.limit must be an integer".to_string())?;
                    *limit = json!(parsed);
                    notes.push("stage.limit: numeric string normalized to integer".into());
                }
            }
        }
        _ => (),
    }
    Ok(())
}

pub(crate) fn normalize_definition(value: &Value) -> Result<(Value, Vec<String>), String> {
    if value.to_string().len() > 65536 {
        return Err("definition exceeds 64 KiB".into());
    }
    let mut value = value.clone();
    let mut notes = vec![];
    if let Some(object) = value.as_object_mut() {
        array_field(object, "reads", "definition.reads", &mut notes)?;
        if let Some(usage) = object.get_mut("usage").and_then(Value::as_object_mut) {
            array_field(usage, "tasks", "definition.usage.tasks", &mut notes)?;
            array_field(usage, "tags", "definition.usage.tags", &mut notes)?;
            array_field(
                usage,
                "limitations",
                "definition.usage.limitations",
                &mut notes,
            )?;
        }
    }
    if let Some(v) = value.get_mut("version") {
        version(v, "definition.version", &mut notes);
    }
    if let Some(stage) = value.get_mut("stage") {
        normalize_stage(stage, &mut 128, &mut notes)?;
    }
    Ok((value, notes))
}

pub(crate) fn parse(input: &Value) -> Result<(DefinitionRequest, Value, Vec<String>), String> {
    if input.to_string().len() > 70000 {
        return Err("Lens request exceeds 70 KiB".into());
    }
    let mut normalized = input.clone();
    let (definition, mut notes) =
        normalize_definition(input.get("definition").ok_or("definition required")?)?;
    normalized["definition"] = definition;
    if let Some(v) = normalized.get_mut("expected_version") {
        version(v, "expected_version", &mut notes);
    }
    let request = serde_json::from_value(normalized.clone()).map_err(|e| format!("Lens contract: {e}. expected_version belongs outside definition; freshness fields belong to binding; every stage requires kind; take.limit must be an integer. Use describe_lens_contract for a dry run."))?;
    Ok((request, normalized, notes))
}

pub(crate) fn validate_request(
    request: &DefinitionRequest,
    catalog: &Catalog,
) -> Result<(), String> {
    if let Some(old) = catalog.get(&request.definition.id) {
        if request.expected_version.as_deref() != Some(old.version.as_str())
            || request.definition.version == old.version
        {
            return Err("update requires matching expected_version and a new version".into());
        }
    } else if request.expected_version.is_some() || catalog.len() >= 128 {
        return Err("invalid create precondition or catalog full".into());
    }
    let mut trial = catalog.clone();
    trial.insert(request.definition.id.clone(), request.definition.clone());
    for id in trial.keys() {
        application::validate(id, &trial)?;
    }
    Ok(())
}

pub(crate) fn diagnose(input: Option<&Value>, catalog: &Catalog) -> Value {
    let Some(input) = input else {
        return json!({"schema":schema(),"example":{"definition":{"id":"demo.context","version":"1","purpose":"Observe topics without insights","reads":["Topic"],"stage":{"kind":"archetype_without","component":"TopicInsight"}}},"mutated":false});
    };
    match parse(input) {
        Ok((request, normalized, notes)) => {
            let error = validate_request(&request, catalog).err();
            json!({"valid":error.is_none(),"errors":error.into_iter().collect::<Vec<_>>(),"normalized_input":normalized,"normalizations":notes,"mutated":false})
        }
        Err(error) => {
            json!({"valid":false,"errors":[error],"normalized_input":null,"mutated":false,
            "guidance":{"expected_version":"top-level update precondition, not part of definition","source_updated_at_pointer":"belongs in define_lens_binding.binding","take.limit":"JSON integer, not a numeric string","stage.kind":"required discriminator; call describe_lens_contract({}) for the schema"}})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Value {
        json!({"definition":{"id":"demo","version":1,"purpose":"test","reads":[],"stage":{"kind":"take","limit":3}}})
    }
    #[test]
    fn schema_covers_all_stages_and_contract_diagnoses_common_failures() {
        let s = schema();
        assert_eq!(
            s["properties"]["definition"]["properties"]["version"]["type"],
            "string"
        );
        assert_eq!(s["$defs"]["Stage"]["oneOf"].as_array().unwrap().len(), 9);
        let good = diagnose(Some(&request()), &Catalog::new());
        assert_eq!(good["valid"], true);
        assert_eq!(good["normalized_input"]["definition"]["version"], "1");
        let mut string_limit = request();
        string_limit["definition"]["stage"]["limit"] = json!("3");
        let normalized = diagnose(Some(&string_limit), &Catalog::new());
        assert_eq!(normalized["valid"], true);
        assert_eq!(
            normalized["normalized_input"]["definition"]["stage"]["limit"],
            3
        );
        let mut indexed_arrays = request();
        indexed_arrays["definition"]["reads"] = json!({"0":"TickAction"});
        let normalized = diagnose(Some(&indexed_arrays), &Catalog::new());
        assert_eq!(normalized["valid"], true);
        assert_eq!(
            normalized["normalized_input"]["definition"]["reads"],
            json!(["TickAction"])
        );
        for (pointer, value) in [
            ("/definition/stage", json!({"limit":3})),
            ("/definition/version", json!(1.5)),
        ] {
            let mut bad = request();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert_eq!(diagnose(Some(&bad), &Catalog::new())["valid"], false);
        }
        for key in ["expected_version", "source_updated_at_pointer"] {
            let mut bad = request();
            bad["definition"][key] = json!("1");
            assert_eq!(diagnose(Some(&bad), &Catalog::new())["valid"], false);
        }
    }
    #[test]
    fn doctor_checks_update_precondition_and_nested_reference_versions() {
        let (first, _, _) = parse(&request()).unwrap();
        let catalog = Catalog::from([("demo".into(), first.definition)]);
        let mut update = request();
        update["definition"]["version"] = json!(2);
        assert_eq!(diagnose(Some(&update), &catalog)["valid"], false);
        update["expected_version"] = json!(1);
        assert_eq!(diagnose(Some(&update), &catalog)["valid"], true);
        let child = json!({"definition":{"id":"child","version":1,"purpose":"nested","reads":[],"stage":{"kind":"lens","id":"demo","version":1}}});
        assert_eq!(diagnose(Some(&child), &catalog)["valid"], true);
        assert_eq!(catalog["demo"].version, "1");
    }
}
