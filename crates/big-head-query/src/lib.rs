use bevy_ecs::component::{ComponentId, ComponentInfo};
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::world::World;
use bevy_reflect::ReflectFromPtr;
mod filters;
use filters::{parse_with_without_keyword, split_filter_tokens};

/// Comparison operator for value filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    Contains,
}

/// A field-level predicate: `Component.field op literal`.
#[derive(Debug, Clone)]
pub struct ValueFilter {
    pub component_id: ComponentId,
    /// The component short name that appeared on the LHS; kept for
    /// error messages and round-tripping.
    pub component_name: String,
    /// Path into the component's serialized JSON. For a flat struct
    /// like `Position { x, y }` a filter `Position.x` has
    /// `field_path == ["x"]`.
    pub field_path: Vec<String>,
    pub op: CompareOp,
    pub literal: serde_json::Value,
}

/// A dynamic query constructed from component IDs at runtime.
#[derive(Debug, Clone)]
pub struct DynamicQuery {
    /// Components to fetch (return their values).
    pub fetch: Vec<ComponentId>,
    /// Components that must be present (filter).
    pub with: Vec<ComponentId>,
    /// Components that must NOT be present (negative filter).
    pub without: Vec<ComponentId>,
    /// Field-level predicates applied to each candidate entity.
    pub value_filters: Vec<ValueFilter>,
}

/// Result of executing a dynamic query for one entity.
#[derive(Debug)]
pub struct DynamicQueryResult {
    pub entity: Entity,
    pub values: Vec<(String, Option<String>)>,
}

/// Errors from query parsing.
#[derive(Debug)]
pub enum QueryError {
    UnknownComponent(String),
    ParseError(String),
    InvalidOperator(String),
    InvalidFieldPath(String),
    InvalidLiteral(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::UnknownComponent(name) => write!(f, "Unknown component: {}", name),
            QueryError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            QueryError::InvalidOperator(tok) => write!(f, "Invalid operator in `{}`", tok),
            QueryError::InvalidFieldPath(tok) => write!(f, "Invalid field path in `{}`", tok),
            QueryError::InvalidLiteral(tok) => write!(f, "Invalid literal `{}`", tok),
        }
    }
}

/// Parse a query string into a `DynamicQuery`.
///
/// Syntax:
/// ```text
/// "Position, Health where Enemy and !Dead"
/// "* where Player"
/// "AgentIntent"
/// "ToolHistory where ToolHistory.max_entries > 100"
/// "Session where WorkspaceRef.id == 'agent-ide'"
/// ```
///
/// - Component names before `where` are fetched (their values returned)
/// - `*` means fetch all components on matching entities
/// - After `where`: component names are required filters, `!Name` is a
///   negative filter, and `Component.field op literal` is a value filter
/// - Bevy-style keyword form: `WITH(A) AND WITH(B) AND WITHOUT(C)` is
///   accepted as a more readable alias for `A and B and !C`. `WITH(...)` /
///   `WITHOUT(...)` may also hold a comma-separated list, e.g.
///   `WITH(Health, Position)`. Keywords are case-insensitive and resolve
///   to the same `with` / `without` buckets as the short form.
/// - Operators: `==`, `!=`, `>=`, `<=`, `>`, `<`, `contains`
/// - Literals: single-/double-quoted strings, numbers, `true`, `false`, `null`
/// - Names are matched by suffix (e.g. "Health" matches "my_crate::Health")
pub fn parse_query(world: &World, query_str: &str) -> Result<DynamicQuery, QueryError> {
    let query_str = query_str.trim();
    if query_str.is_empty() {
        return Err(QueryError::ParseError("Empty query string".into()));
    }

    // Split on "where" (case-insensitive)
    let (fetch_part, filter_part) =
        if let Some(idx) = query_str.to_ascii_lowercase().find(" where ") {
            (&query_str[..idx], Some(&query_str[idx + 7..]))
        } else {
            (query_str, None)
        };

    let components = world.components();

    // Parse fetch part
    let fetch_names: Vec<&str> = fetch_part
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let fetch_all = fetch_names.len() == 1 && fetch_names[0] == "*";

    let mut fetch = Vec::new();
    if !fetch_all {
        for name in &fetch_names {
            let id = resolve_component_id_from(components.iter_registered(), name)?;
            fetch.push(id);
        }
    }

    // Parse filter part
    let mut with = Vec::new();
    let mut without = Vec::new();
    let mut value_filters = Vec::new();

    if let Some(filter_str) = filter_part {
        // Paren-aware tokenizer: split on ` and ` / `,` but only at the
        // top level so `WITH(A, B)` stays one token until we handle the
        // keyword form explicitly below.
        let filter_tokens = split_filter_tokens(filter_str)?;

        for token in filter_tokens {
            // 0. Bevy-style keyword form: `WITH(name1, name2)` /
            //    `WITHOUT(name1, name2)` (case-insensitive). These are pure
            //    aliases for the short `name` / `!name` forms — they route
            //    into the same with / without buckets.
            if let Some((negated, names)) = parse_with_without_keyword(token)? {
                for name in names {
                    let id = resolve_component_id_from(components.iter_registered(), &name)?;
                    if negated {
                        without.push(id);
                    } else {
                        with.push(id);
                    }
                }
                continue;
            }

            // 1. value filter (check for operators first — highest priority)
            if let Some((lhs, op, rhs)) = split_on_operator(&token) {
                let (comp_name, field_path) = parse_lhs(lhs)?;
                let component_id = resolve_component_id(world, &comp_name)?;
                let literal = parse_literal(rhs)?;
                value_filters.push(ValueFilter {
                    component_id,
                    component_name: comp_name,
                    field_path,
                    op,
                    literal,
                });
                continue;
            }

            // 2. negation
            if let Some(name) = token.strip_prefix('!') {
                let name = name.trim();
                let id = resolve_component_id_from(components.iter_registered(), name)?;
                without.push(id);
                continue;
            }

            // 3. presence
            let id = resolve_component_id_from(components.iter_registered(), token)?;
            with.push(id);
        }
    }

    // A value filter on a component implicitly requires its presence so the
    // archetype scan picks up candidate entities.
    for vf in &value_filters {
        if !with.contains(&vf.component_id) {
            with.push(vf.component_id);
        }
    }

    Ok(DynamicQuery {
        fetch: if fetch_all { Vec::new() } else { fetch },
        with,
        without,
        value_filters,
    })
}

/// Scan a token for a comparison operator. Two-char operators take
/// priority so `>=` is not mis-parsed as `>` followed by `=`.
fn split_on_operator(token: &str) -> Option<(&str, CompareOp, &str)> {
    if let Some(idx) = token.find(" contains ") {
        let lhs = token[..idx].trim();
        let rhs = token[idx + " contains ".len()..].trim();
        if !lhs.is_empty() && !rhs.is_empty() {
            return Some((lhs, CompareOp::Contains, rhs));
        }
    }

    // Ordered: two-char operators first.
    let two_char_ops: &[(&str, CompareOp)] = &[
        ("==", CompareOp::Eq),
        ("!=", CompareOp::Ne),
        (">=", CompareOp::Ge),
        ("<=", CompareOp::Le),
    ];
    let one_char_ops: &[(&str, CompareOp)] = &[(">", CompareOp::Gt), ("<", CompareOp::Lt)];

    for (pat, op) in two_char_ops {
        if let Some(idx) = token.find(pat) {
            let lhs = token[..idx].trim();
            let rhs = token[idx + pat.len()..].trim();
            if !lhs.is_empty() && !rhs.is_empty() {
                return Some((lhs, *op, rhs));
            }
        }
    }
    for (pat, op) in one_char_ops {
        if let Some(idx) = token.find(pat) {
            let lhs = token[..idx].trim();
            let rhs = token[idx + pat.len()..].trim();
            if !lhs.is_empty() && !rhs.is_empty() {
                return Some((lhs, *op, rhs));
            }
        }
    }
    None
}

/// Split `Component.field.sub` into `("Component", ["field", "sub"])`.
/// The component name must be followed by at least one field segment.
fn parse_lhs(lhs: &str) -> Result<(String, Vec<String>), QueryError> {
    let mut parts = lhs.split('.').map(|s| s.trim()).filter(|s| !s.is_empty());
    let component = parts
        .next()
        .ok_or_else(|| QueryError::InvalidFieldPath(lhs.to_string()))?;
    let field_path: Vec<String> = parts.map(|s| s.to_string()).collect();
    if field_path.is_empty() {
        return Err(QueryError::InvalidFieldPath(lhs.to_string()));
    }
    Ok((component.to_string(), field_path))
}

/// Parse a literal on the right-hand side of a value filter.
/// Supported: `'string'`, `"string"`, numbers, `true`, `false`, `null`.
fn parse_literal(rhs: &str) -> Result<serde_json::Value, QueryError> {
    let rhs = rhs.trim();
    if rhs.is_empty() {
        return Err(QueryError::InvalidLiteral(rhs.to_string()));
    }

    // String literal
    if (rhs.starts_with('\'') && rhs.ends_with('\'') && rhs.len() >= 2)
        || (rhs.starts_with('"') && rhs.ends_with('"') && rhs.len() >= 2)
    {
        let inner = &rhs[1..rhs.len() - 1];
        return Ok(serde_json::Value::String(inner.to_string()));
    }

    // Bool / null
    match rhs {
        "true" => return Ok(serde_json::Value::Bool(true)),
        "false" => return Ok(serde_json::Value::Bool(false)),
        "null" => return Ok(serde_json::Value::Null),
        _ => {}
    }

    // Number
    if let Ok(n) = rhs.parse::<i64>() {
        return Ok(serde_json::Value::Number(n.into()));
    }
    if let Ok(f) = rhs.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Ok(serde_json::Value::Number(n));
        }
    }

    Err(QueryError::InvalidLiteral(rhs.to_string()))
}

/// A callback that knows how to read dynamic component values as strings.
///
/// Callers (typically `agent-ide-runtime`) provide this so dynamic
/// components can participate in query results without creating a
/// crate-level dependency on the runtime.
pub type DynamicValueReader<'a> = &'a dyn Fn(&World, Entity, ComponentId) -> Option<String>;

/// A callback that knows how to read dynamic component values as JSON.
///
/// Used by value filters so they can compare structured field values
/// (numbers, strings, bools) rather than debug-formatted strings.
pub type DynamicJsonReader<'a> =
    &'a dyn Fn(&World, Entity, ComponentId) -> Option<serde_json::Value>;

/// Execute a dynamic query against the world.
pub fn execute_dynamic_query(world: &World, query: &DynamicQuery) -> Vec<DynamicQueryResult> {
    execute_dynamic_query_with_reader(world, query, None, None)
}

/// Execute a dynamic query against the world, consulting `dynamic_reader`
/// for components that reflection cannot read as a display string and
/// `dynamic_json_reader` for value-filter comparisons.
pub fn execute_dynamic_query_with_reader(
    world: &World,
    query: &DynamicQuery,
    dynamic_reader: Option<DynamicValueReader<'_>>,
    dynamic_json_reader: Option<DynamicJsonReader<'_>>,
) -> Vec<DynamicQueryResult> {
    let components = world.components();
    let type_registry = world.get_resource::<AppTypeRegistry>();
    let registry_guard = type_registry.map(|r| r.read());

    let mut results = Vec::new();

    for archetype in world.archetypes().iter() {
        let arch_components: Vec<ComponentId> = archetype.components().to_vec();

        // Check with filter
        let has_all_with = query.with.iter().all(|id| arch_components.contains(id));
        if !has_all_with {
            continue;
        }

        // Check without filter
        let has_any_without = query.without.iter().any(|id| arch_components.contains(id));
        if has_any_without {
            continue;
        }

        // Determine which components to fetch
        let fetch_ids: Vec<ComponentId> = if query.fetch.is_empty() {
            // "* where X" or no query at all — fetch all components on matching entities
            arch_components.clone()
        } else {
            // Only fetch requested components that exist on this archetype
            let ids: Vec<ComponentId> = query
                .fetch
                .iter()
                .filter(|id| arch_components.contains(id))
                .copied()
                .collect();
            // Skip archetype if none of the requested fetch components exist
            if ids.is_empty() {
                continue;
            }
            ids
        };

        // Iterate entities in this archetype
        for arch_entity in archetype.entities() {
            let entity = arch_entity.id();

            // Apply value filters before computing display values so we
            // don't pay the serialization cost for entities that won't
            // appear in the output.
            let passes_value_filters = query.value_filters.iter().all(|vf| {
                let info = match components.get_info(vf.component_id) {
                    Some(i) => i,
                    None => return false,
                };
                let json =
                    try_read_value_as_json(world, entity, vf.component_id, info, &registry_guard)
                        .or_else(|| {
                            dynamic_json_reader.and_then(|r| r(world, entity, vf.component_id))
                        });
                let json = match json {
                    Some(v) => v,
                    None => return false,
                };
                let field_value = match follow_field_path(&json, &vf.field_path) {
                    Some(v) => v,
                    None => return false,
                };
                compare_values(field_value, vf.op, &vf.literal)
            });
            if !passes_value_filters {
                continue;
            }

            let mut values = Vec::new();
            for &comp_id in &fetch_ids {
                let info = match components.get_info(comp_id) {
                    Some(i) => i,
                    None => continue,
                };
                let name = info.name().to_string();

                // Try reflection first, then the dynamic reader fallback.
                let value = try_read_value(world, entity, comp_id, info, &registry_guard)
                    .or_else(|| dynamic_reader.and_then(|r| r(world, entity, comp_id)));
                values.push((name, value));
            }

            results.push(DynamicQueryResult { entity, values });
        }
    }

    results
}

/// Resolve a component name (possibly short) to a ComponentId.
fn resolve_component_id_from<'a>(
    registered: impl Iterator<Item = &'a ComponentInfo>,
    name: &str,
) -> Result<ComponentId, QueryError> {
    let name = name.trim();
    let candidates: Vec<&ComponentInfo> = registered
        .filter(|info| {
            let full_name = info.name().to_string();
            // Exact match
            full_name == name
                // Suffix match (e.g. "Health" matches "my_crate::Health")
                || full_name.ends_with(&format!("::{}", name))
                // Simple name match (last segment)
                || full_name.rsplit("::").next() == Some(name)
        })
        .collect();

    match candidates.len() {
        0 => Err(QueryError::UnknownComponent(name.to_string())),
        1 => Ok(candidates[0].id()),
        _ => {
            // Prefer exact match
            for c in &candidates {
                if c.name().to_string() == name {
                    return Ok(c.id());
                }
            }
            // Use first match
            Ok(candidates[0].id())
        }
    }
}

/// Resolve an exact or short component name against the live world.
///
/// Structured query frontends use this instead of manufacturing a textual
/// query and feeding it back through the DSL parser.
pub fn resolve_component_id(world: &World, name: &str) -> Result<ComponentId, QueryError> {
    resolve_component_id_from(world.components().iter_registered(), name)
}

fn try_read_value(
    world: &World,
    entity: Entity,
    component_id: ComponentId,
    info: &ComponentInfo,
    registry_guard: &Option<std::sync::RwLockReadGuard<'_, bevy_reflect::TypeRegistry>>,
) -> Option<String> {
    let type_id = info.type_id()?;
    let registry = registry_guard.as_ref()?;
    let reflect_from_ptr = registry.get_type_data::<ReflectFromPtr>(type_id)?;
    let ptr = world.get_by_id(entity, component_id)?;
    let reflected = unsafe { reflect_from_ptr.as_reflect(ptr) };
    Some(format!("{reflected:?}"))
}

/// Serialize a reflected component to `serde_json::Value`. Returns the
/// bare data object (no outer type-path wrapper) so value-filter field
/// paths can navigate directly into its fields.
fn try_read_value_as_json(
    world: &World,
    entity: Entity,
    component_id: ComponentId,
    info: &ComponentInfo,
    registry_guard: &Option<std::sync::RwLockReadGuard<'_, bevy_reflect::TypeRegistry>>,
) -> Option<serde_json::Value> {
    let type_id = info.type_id()?;
    let registry = registry_guard.as_ref()?;
    let reflect_from_ptr = registry.get_type_data::<ReflectFromPtr>(type_id)?;
    let ptr = world.get_by_id(entity, component_id)?;
    let reflected = unsafe { reflect_from_ptr.as_reflect(ptr) };
    let serializer =
        bevy_reflect::serde::TypedReflectSerializer::new(reflected.as_partial_reflect(), registry);
    serde_json::to_value(&serializer).ok()
}

/// Navigate a JSON value by path. For objects, each segment is a key
/// lookup; for arrays, each segment is parsed as a usize index.
fn follow_field_path<'a>(
    value: &'a serde_json::Value,
    path: &[String],
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path {
        current = match current {
            serde_json::Value::Object(map) => map.get(segment)?,
            serde_json::Value::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(current)
}

fn compare_values(lhs: &serde_json::Value, op: CompareOp, rhs: &serde_json::Value) -> bool {
    match op {
        CompareOp::Eq => lhs == rhs,
        CompareOp::Ne => lhs != rhs,
        CompareOp::Contains => match (lhs.as_str(), rhs.as_str()) {
            (Some(lhs), Some(rhs)) => lhs.contains(rhs),
            _ => false,
        },
        CompareOp::Gt | CompareOp::Lt | CompareOp::Ge | CompareOp::Le => {
            let l = lhs.as_f64();
            let r = rhs.as_f64();
            match (l, r) {
                (Some(l), Some(r)) => match op {
                    CompareOp::Gt => l > r,
                    CompareOp::Lt => l < r,
                    CompareOp::Ge => l >= r,
                    CompareOp::Le => l <= r,
                    _ => unreachable!(),
                },
                _ => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use bevy_reflect::Reflect;

    #[derive(Component, Reflect, Debug)]
    struct Health(f32);

    #[derive(Component, Reflect, Debug)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Component, Reflect, Debug)]
    struct Score {
        name: String,
        points: i32,
    }

    #[derive(Component, Debug)]
    struct Enemy;

    #[derive(Component, Debug)]
    struct Player;

    #[derive(Component, Debug)]
    struct Dead;

    fn setup_world() -> World {
        let mut world = World::new();
        world.register_component::<Health>();
        world.register_component::<Position>();
        world.register_component::<Enemy>();
        world.register_component::<Player>();
        world.register_component::<Dead>();

        let registry = AppTypeRegistry::default();
        {
            let mut reg = registry.write();
            reg.register::<Health>();
            reg.register_type_data::<Health, ReflectFromPtr>();
            reg.register::<Position>();
            reg.register_type_data::<Position, ReflectFromPtr>();
        }
        world.insert_resource(registry);

        // Player
        world.spawn((Player, Health(100.0), Position { x: 0.0, y: 0.0 }));
        // Living enemy
        world.spawn((Enemy, Health(50.0), Position { x: 5.0, y: 3.0 }));
        // Dead enemy
        world.spawn((Enemy, Health(0.0), Dead));
        // Another enemy
        world.spawn((Enemy, Health(75.0), Position { x: -2.0, y: 7.0 }));

        world
    }

    fn setup_score_world() -> World {
        let mut world = World::new();
        world.register_component::<Score>();
        let registry = AppTypeRegistry::default();
        {
            let mut reg = registry.write();
            reg.register::<Score>();
            reg.register_type_data::<Score, ReflectFromPtr>();
        }
        world.insert_resource(registry);

        world.spawn(Score {
            name: "alice".into(),
            points: 10,
        });
        world.spawn(Score {
            name: "bob".into(),
            points: 50,
        });
        world.spawn(Score {
            name: "carol".into(),
            points: 100,
        });
        world
    }

    #[test]
    fn keyword_presence_filters_match_short_forms() {
        let world = setup_world();
        for text in [
            "Health where WITH(Enemy) AND WITHOUT(Dead)",
            "Health where with(Enemy, Health), without(Dead)",
            "Health where Enemy and WITHOUT(Dead)",
        ] {
            let query = parse_query(&world, text).unwrap();
            assert_eq!(execute_dynamic_query(&world, &query).len(), 2, "{text}");
        }
        for text in [
            "* where WITH()",
            "* where WITH(Enemy,)",
            "* where WITH(Enemy",
            "* where WITHOUT((Dead))",
            "* where Enemy AND ",
        ] {
            assert!(
                matches!(parse_query(&world, text), Err(QueryError::ParseError(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn separators_inside_literals_are_not_filters() {
        let world = setup_score_world();
        let query = parse_query(
            &world,
            "Score where Score.name == 'a, b AND WITH(Score)' AND WITH(Score)",
        )
        .unwrap();
        assert_eq!(query.value_filters[0].literal, "a, b AND WITH(Score)");
        assert_eq!(query.value_filters.len(), 1);
        assert!(parse_query(&world, "Score where Score.name == 'unclosed").is_err());
    }

    #[test]
    fn test_simple_query() {
        let world = setup_world();
        let query = parse_query(&world, "Health").unwrap();
        let results = execute_dynamic_query(&world, &query);
        // Only 4 user-spawned entities have Health
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn test_query_with_filter() {
        let world = setup_world();
        let query = parse_query(&world, "Health where Enemy").unwrap();
        let results = execute_dynamic_query(&world, &query);
        // 3 enemies have Health
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_query_with_negative_filter() {
        let world = setup_world();
        let query = parse_query(&world, "Health where Enemy and !Dead").unwrap();
        let results = execute_dynamic_query(&world, &query);
        // 2 living enemies
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_wildcard_query() {
        let world = setup_world();
        let query = parse_query(&world, "* where Player").unwrap();
        let results = execute_dynamic_query(&world, &query);
        assert_eq!(results.len(), 1);
        // Player has Health, Position, Player
        assert!(results[0].values.len() >= 3);
    }

    #[test]
    fn test_multi_fetch() {
        let world = setup_world();
        let query = parse_query(&world, "Position, Health where Enemy and !Dead").unwrap();
        let results = execute_dynamic_query(&world, &query);
        assert_eq!(results.len(), 2);
        for r in &results {
            // Each result should have Position and Health values
            assert_eq!(r.values.len(), 2);
            assert!(r.values.iter().any(|(n, _)| n.contains("Position")));
            assert!(r.values.iter().any(|(n, _)| n.contains("Health")));
        }
    }

    #[test]
    fn test_reflected_values() {
        let world = setup_world();
        let query = parse_query(&world, "Health where Player").unwrap();
        let results = execute_dynamic_query(&world, &query);
        assert_eq!(results.len(), 1);
        let (name, value) = &results[0].values[0];
        assert!(name.contains("Health"));
        assert!(value.as_ref().unwrap().contains("100"));
    }

    #[test]
    fn test_unknown_component() {
        let world = setup_world();
        let result = parse_query(&world, "NonExistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_query_with_dynamic_reader() {
        use bevy_ecs::component::{ComponentCloneBehavior, ComponentDescriptor, StorageType};
        use bevy_ecs::ptr::OwningPtr;
        use std::alloc::Layout;

        unsafe fn drop_boxed_string(ptr: OwningPtr<'_>) {
            let _boxed: Box<String> = unsafe { ptr.read::<Box<String>>() };
        }

        let mut world = World::new();
        let descriptor = unsafe {
            ComponentDescriptor::new_with_layout(
                "DynTag",
                StorageType::Table,
                Layout::new::<Box<String>>(),
                Some(drop_boxed_string),
                true,
                ComponentCloneBehavior::Ignore,
                None,
            )
        };
        let id = world.register_component_with_descriptor(descriptor);

        let entity = world.spawn_empty().id();
        let val: Box<String> = Box::new("shipped".to_string());
        OwningPtr::make(val, |ptr| unsafe {
            world.entity_mut(entity).insert_by_id(id, ptr);
        });

        let query = parse_query(&world, "DynTag").unwrap();
        let dyn_id = id;
        let reader = move |w: &World, e: Entity, cid: ComponentId| -> Option<String> {
            if cid != dyn_id {
                return None;
            }
            let ptr = w.get_by_id(e, cid)?;
            let s: &Box<String> = unsafe { ptr.deref::<Box<String>>() };
            Some((**s).clone())
        };

        let results = execute_dynamic_query_with_reader(&world, &query, Some(&reader), None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].values.len(), 1);
        assert_eq!(results[0].values[0].1.as_deref(), Some("shipped"));
    }

    // ── Value filter tests ────────────────────────────────────────────

    #[test]
    fn parse_value_filter_equality() {
        let world = setup_score_world();
        let q = parse_query(&world, "Score where Score.name == 'alice'").unwrap();
        assert_eq!(q.value_filters.len(), 1);
        let vf = &q.value_filters[0];
        assert_eq!(vf.op, CompareOp::Eq);
        assert_eq!(vf.field_path, vec!["name".to_string()]);
        assert_eq!(vf.literal, serde_json::json!("alice"));
    }

    #[test]
    fn parse_value_filter_numeric_gt() {
        let world = setup_score_world();
        let q = parse_query(&world, "Score where Score.points > 5").unwrap();
        assert_eq!(q.value_filters.len(), 1);
        assert_eq!(q.value_filters[0].op, CompareOp::Gt);
        assert_eq!(q.value_filters[0].literal, serde_json::json!(5));
    }

    #[test]
    fn parse_value_filter_contains() {
        let world = setup_score_world();
        let q = parse_query(&world, "Score where Score.name contains 'ali'").unwrap();
        assert_eq!(q.value_filters.len(), 1);
        assert_eq!(q.value_filters[0].op, CompareOp::Contains);
        assert_eq!(q.value_filters[0].literal, serde_json::json!("ali"));
    }

    #[test]
    fn parse_value_filter_mixed_with_presence() {
        let world = setup_world();
        let q = parse_query(&world, "Position where Position.x > 0.0 and !Dead").unwrap();
        assert_eq!(q.value_filters.len(), 1);
        assert_eq!(q.without.len(), 1);
        // Position.x > 0.0 implicitly requires Position in `with`.
        assert!(
            q.with
                .iter()
                .any(|id| *id == q.value_filters[0].component_id)
        );
    }

    #[test]
    fn parse_value_filter_invalid_operator() {
        let world = setup_score_world();
        // Empty LHS/RHS around an operator should return an error, not a filter.
        let err = parse_query(&world, "Score where == 5");
        assert!(err.is_err());
    }

    #[test]
    fn parse_value_filter_invalid_literal() {
        let world = setup_score_world();
        let err = parse_query(&world, "Score where Score.points > notanumber");
        assert!(matches!(err, Err(QueryError::InvalidLiteral(_))));
    }

    #[test]
    fn execute_value_filter_filters_entities() {
        let world = setup_score_world();
        let q = parse_query(&world, "Score where Score.points > 20").unwrap();
        let results = execute_dynamic_query(&world, &q);
        // bob (50) + carol (100) pass; alice (10) does not.
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn execute_value_filter_string_comparison() {
        let world = setup_score_world();
        let q = parse_query(&world, "Score where Score.name == 'bob'").unwrap();
        let results = execute_dynamic_query(&world, &q);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn execute_value_filter_contains_string() {
        let world = setup_score_world();
        let q = parse_query(&world, "Score where Score.name contains 'o'").unwrap();
        let results = execute_dynamic_query(&world, &q);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn execute_value_filter_combined_with_absence() {
        let world = setup_world();
        // Enemies with x < 0 and not Dead → only the (-2, 7) enemy.
        let q = parse_query(&world, "Health where Enemy and Position.x < 0 and !Dead").unwrap();
        let results = execute_dynamic_query(&world, &q);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn execute_value_filter_on_dynamic_component() {
        use bevy_ecs::component::{ComponentCloneBehavior, ComponentDescriptor, StorageType};
        use bevy_ecs::ptr::OwningPtr;
        use std::alloc::Layout;

        unsafe fn drop_boxed_json(ptr: OwningPtr<'_>) {
            let _boxed: Box<serde_json::Value> = unsafe { ptr.read::<Box<serde_json::Value>>() };
        }

        let mut world = World::new();
        let descriptor = unsafe {
            ComponentDescriptor::new_with_layout(
                "DynDeadline",
                StorageType::Table,
                Layout::new::<Box<serde_json::Value>>(),
                Some(drop_boxed_json),
                true,
                ComponentCloneBehavior::Ignore,
                None,
            )
        };
        let id = world.register_component_with_descriptor(descriptor);

        let e1 = world.spawn_empty().id();
        let v1: Box<serde_json::Value> = Box::new(serde_json::json!({"urgent": true}));
        OwningPtr::make(v1, |ptr| unsafe {
            world.entity_mut(e1).insert_by_id(id, ptr);
        });
        let e2 = world.spawn_empty().id();
        let v2: Box<serde_json::Value> = Box::new(serde_json::json!({"urgent": false}));
        OwningPtr::make(v2, |ptr| unsafe {
            world.entity_mut(e2).insert_by_id(id, ptr);
        });

        let q = parse_query(&world, "DynDeadline where DynDeadline.urgent == true").unwrap();
        let dyn_id = id;
        let json_reader =
            move |w: &World, e: Entity, cid: ComponentId| -> Option<serde_json::Value> {
                if cid != dyn_id {
                    return None;
                }
                let ptr = w.get_by_id(e, cid)?;
                let v: &Box<serde_json::Value> = unsafe { ptr.deref::<Box<serde_json::Value>>() };
                Some((**v).clone())
            };
        let string_reader = move |w: &World, e: Entity, cid: ComponentId| -> Option<String> {
            if cid != dyn_id {
                return None;
            }
            let ptr = w.get_by_id(e, cid)?;
            let v: &Box<serde_json::Value> = unsafe { ptr.deref::<Box<serde_json::Value>>() };
            Some((**v).to_string())
        };

        let results =
            execute_dynamic_query_with_reader(&world, &q, Some(&string_reader), Some(&json_reader));
        assert_eq!(results.len(), 1);
    }
}
