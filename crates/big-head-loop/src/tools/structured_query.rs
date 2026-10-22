//! Typed JSON query frontend for agents that should not need to compose DSL strings.

use std::collections::HashSet;
use std::fmt::Write;

use bevy_ecs::component::ComponentId;
use bevy_ecs::world::World;
use big_head_ai::{ToolCall, ToolDefinition};
use big_head_query::{CompareOp, DynamicQuery, ValueFilter, execute_dynamic_query_with_reader};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::registry::ComponentActionRegistry;

use super::helpers::{query_reader, snapshot_reader};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    select: Vec<String>,
    #[serde(default)]
    with: Vec<String>,
    #[serde(default)]
    without: Vec<String>,
    #[serde(default)]
    filters: Vec<Filter>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    component: String,
    path: Vec<String>,
    op: String,
    value: Value,
}

pub(super) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "query_ecs".into(),
        description: "Read ECS entities with structured JSON criteria. Prefer this over query_entities when filtering fields: component names, field paths, operators and JSON values are separate fields, so no query DSL has to be guessed. select=['*'] returns all components. This is read-only and supports the standard output_offset/output_limit paging fields.".into(),
        input_schema: json!({
            "type":"object",
            "properties":{
                "select":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string"}},
                "with":{"type":"array","maxItems":16,"items":{"type":"string"}},
                "without":{"type":"array","maxItems":16,"items":{"type":"string"}},
                "filters":{"type":"array","maxItems":16,"items":{
                    "type":"object",
                    "properties":{
                        "component":{"type":"string"},
                        "path":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"string"}},
                        "op":{"type":"string","enum":["eq","ne","gt","lt","gte","lte","contains"]},
                        "value":{}
                    },
                    "required":["component","path","op","value"],
                    "additionalProperties":false
                }},
                "output_offset":{"type":"integer","minimum":0},
                "output_limit":{"type":"integer","minimum":1,"maximum":8000},
                "expected_sha256":{"type":"string"}
            },
            "required":["select"],
            "additionalProperties":false
        }),
    }
}

fn resolve(world: &World, names: &[String]) -> Result<Vec<ComponentId>, String> {
    names
        .iter()
        .map(|name| big_head_query::resolve_component_id(world, name).map_err(|e| e.to_string()))
        .collect()
}

fn compare_op(value: &str) -> CompareOp {
    match value {
        "eq" => CompareOp::Eq,
        "ne" => CompareOp::Ne,
        "gt" => CompareOp::Gt,
        "lt" => CompareOp::Lt,
        "gte" => CompareOp::Ge,
        "lte" => CompareOp::Le,
        "contains" => CompareOp::Contains,
        _ => unreachable!("schema enum validation"),
    }
}

fn validate_field_path(
    registry: &ComponentActionRegistry,
    component: &str,
    path: &[String],
) -> Result<(), String> {
    if path.iter().any(|part| {
        part.is_empty()
            || !part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    }) {
        return Err(format!("Invalid field path for component '{component}'"));
    }
    let short = component.rsplit("::").next().unwrap_or(component);
    if let Some((_, schema)) = registry
        .dynamic_store()
        .iter()
        .find(|(name, _)| name.rsplit("::").next().unwrap_or(name) == short)
        && !schema.fields.contains_key(&path[0])
    {
        return Err(format!(
            "Unknown field '{}.{}'; inspect the live dynamic schema first",
            component, path[0]
        ));
    }
    Ok(())
}

pub(super) fn dispatch(
    world: &World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool) {
    let request: Request = match serde_json::from_value(call.input.clone()) {
        Ok(value) => value,
        Err(error) => return (format!("query_ecs: {error}"), true),
    };
    if request.select.is_empty() || request.select.len() > 16 || request.filters.len() > 16 {
        return (
            "query_ecs: select must contain 1..16 names and filters at most 16".into(),
            true,
        );
    }
    let fetch_all = request.select.len() == 1 && request.select[0] == "*";
    if request.select.iter().any(|name| name == "*") && !fetch_all {
        return ("query_ecs: '*' must be the only select entry".into(), true);
    }

    let fetch = if fetch_all {
        Vec::new()
    } else {
        match resolve(world, &request.select) {
            Ok(ids) => ids,
            Err(error) => return (format!("query_ecs: {error}"), true),
        }
    };
    let mut with = match resolve(world, &request.with) {
        Ok(ids) => ids,
        Err(error) => return (format!("query_ecs: {error}"), true),
    };
    let without = match resolve(world, &request.without) {
        Ok(ids) => ids,
        Err(error) => return (format!("query_ecs: {error}"), true),
    };
    let without_set: HashSet<_> = without.iter().copied().collect();
    if with.iter().any(|id| without_set.contains(id)) {
        return (
            "query_ecs: a component cannot appear in both with and without".into(),
            true,
        );
    }

    let mut value_filters = Vec::new();
    for filter in request.filters {
        if let Err(error) = validate_field_path(registry, &filter.component, &filter.path) {
            return (format!("query_ecs: {error}"), true);
        }
        let component_id = match big_head_query::resolve_component_id(world, &filter.component) {
            Ok(id) => id,
            Err(error) => return (format!("query_ecs: {error}"), true),
        };
        if without_set.contains(&component_id) {
            return (
                format!(
                    "query_ecs: filtered component '{}' is excluded",
                    filter.component
                ),
                true,
            );
        }
        if !with.contains(&component_id) {
            with.push(component_id);
        }
        value_filters.push(ValueFilter {
            component_id,
            component_name: filter.component,
            field_path: filter.path,
            op: compare_op(&filter.op),
            literal: filter.value,
        });
    }

    let query = DynamicQuery {
        fetch,
        with,
        without,
        value_filters,
    };
    let string_reader = query_reader(registry.dynamic_store());
    let json_reader = snapshot_reader(registry.dynamic_store());
    let results =
        execute_dynamic_query_with_reader(world, &query, Some(&string_reader), Some(&json_reader));
    if results.is_empty() {
        return ("No entities match the structured query.".into(), false);
    }
    let mut output = String::new();
    for result in results {
        writeln!(output, "Entity {}:", result.entity.index().index()).unwrap();
        for (name, value) in result.values {
            let short = name.rsplit("::").next().unwrap_or(&name);
            writeln!(
                output,
                "  {}: {}",
                short,
                value.as_deref().unwrap_or("(opaque)")
            )
            .unwrap();
        }
    }
    (output, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DynamicSchema, FieldType, runtime::AgentRuntime};
    use std::collections::BTreeMap;

    fn call(input: Value) -> ToolCall {
        ToolCall {
            id: "structured-query-test".into(),
            name: "query_ecs".into(),
            input,
        }
    }

    #[test]
    fn filters_dynamic_fields_without_dsl_text() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .define_dynamic_component(
                "ObservedFact",
                DynamicSchema {
                    fields: BTreeMap::from([
                        ("label".into(), FieldType::String),
                        ("score".into(), FieldType::Integer),
                    ]),
                },
            )
            .unwrap();
        for (label, score) in [("low", 1), ("high", 8)] {
            let entity = runtime.world.spawn_empty().id();
            runtime
                .insert_dynamic(entity, "ObservedFact", json!({"label":label,"score":score}))
                .unwrap();
        }

        let (output, is_error) = dispatch(
            &runtime.world,
            &runtime.component_registry,
            &call(json!({
                "select":["ObservedFact"],
                "filters":[{"component":"ObservedFact","path":["score"],"op":"gte","value":5}]
            })),
        );
        assert!(!is_error, "{output}");
        assert!(output.contains("high"));
        assert!(!output.contains("low"));
    }

    #[test]
    fn rejects_unknown_dynamic_field_before_scanning() {
        let mut runtime = AgentRuntime::new_for_test();
        runtime
            .define_dynamic_component(
                "ObservedFact",
                DynamicSchema {
                    fields: BTreeMap::from([("score".into(), FieldType::Integer)]),
                },
            )
            .unwrap();
        let (output, is_error) = dispatch(
            &runtime.world,
            &runtime.component_registry,
            &call(json!({
                "select":["ObservedFact"],
                "filters":[{"component":"ObservedFact","path":["source_id"],"op":"eq","value":"x"}]
            })),
        );
        assert!(is_error);
        assert!(output.contains("Unknown field 'ObservedFact.source_id'"));
    }
}
