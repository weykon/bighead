//! Dispatch for `shard_content` and `get_shards` tools — content projection and retrieval.

use bevy_ecs::world::World;
use big_head_ai::{AgentAction, ToolCall};
use big_head_shard::{ShardData, ShardStrategy, Sharder};

use crate::registry::ComponentActionRegistry;

/// Dispatch `shard_content` tool. Splits content by strategy and stores the
/// resulting blocks as ShardData entities. Semantic indexing remains an agent
/// decision made through normal ECS tools.
pub(crate) fn dispatch_shard_content(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    sharder: &Sharder,
    call: &ToolCall,
    last_tool_result: Option<&str>,
) -> (String, bool, Option<AgentAction>) {
    let strategy_str = match call.input.get("strategy").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            return (
                "shard_content: missing 'strategy' parameter".into(),
                true,
                None,
            );
        }
    };

    let paradigm = match call.input.get("paradigm").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                "shard_content: missing 'paradigm' parameter".into(),
                true,
                None,
            );
        }
    };

    // ── Build strategy ──────────────────────────────────────────
    if world
        .query::<&ShardData>()
        .iter(world)
        .any(|s| s.paradigm == paradigm)
        || crate::shard_retention::expired_notice(world, &paradigm).is_some()
    {
        return (
            "shard_content: destination paradigm already exists; choose a new paradigm".into(),
            true,
            None,
        );
    }
    for (name, min, max) in [
        ("overlap_size", 1, 500),
        ("head_lines", 0, usize::MAX as u64),
        ("tail_lines", 0, usize::MAX as u64),
    ] {
        if let Some(value) = call.input.get(name) {
            if !value.as_u64().is_some_and(|n| n >= min && n <= max) {
                return (format!("shard_content: invalid {name}"), true, None);
            }
        }
    }
    let strategy = match strategy_str {
        "overlap" => {
            let overlap =
                call.input
                    .get("overlap_size")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(sharder.config().default_overlap as u64) as usize;
            ShardStrategy::Overlap {
                overlap_size: overlap,
            }
        }
        "custom_ranges" => {
            if !call
                .input
                .get("ranges")
                .and_then(|v| v.as_array())
                .is_some_and(|ranges| {
                    !ranges.is_empty()
                        && ranges.iter().all(|r| {
                            r.as_array().is_some_and(|pair| {
                                pair.len() == 2
                                    && pair[0]
                                        .as_u64()
                                        .zip(pair[1].as_u64())
                                        .is_some_and(|(a, b)| a < b && b <= usize::MAX as u64)
                            })
                        })
                })
            {
                return ("shard_content: ranges must contain [start,end] integer pairs with 0 <= start < end".into(),true,None);
            }
            let ranges: Vec<(usize, usize)> = call
                .input
                .get("ranges")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            let pair = v.as_array()?;
                            Some((
                                pair.first()?.as_u64()? as usize,
                                pair.get(1)?.as_u64()? as usize,
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if ranges.is_empty() {
                return (
                    "shard_content: 'ranges' required for custom_ranges strategy".into(),
                    true,
                    None,
                );
            }
            ShardStrategy::CustomRanges { ranges }
        }
        "head_tail" => {
            let head = call
                .input
                .get("head_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;
            let tail = call
                .input
                .get("tail_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;
            if head.checked_add(tail).is_none() {
                return (
                    "shard_content: head_lines + tail_lines overflow".into(),
                    true,
                    None,
                );
            }
            ShardStrategy::HeadTail {
                head_lines: head,
                tail_lines: tail,
            }
        }
        "custom" => {
            let desc = call
                .input
                .get("custom_split_description")
                .and_then(|v| v.as_str())
                .unwrap_or("no description provided");
            ShardStrategy::Custom {
                description: desc.to_string(),
            }
        }
        _ => {
            return (
                format!(
                    "shard_content: unknown strategy '{}'. Expected: overlap, custom_ranges, head_tail, custom",
                    strategy_str
                ),
                true,
                None,
            );
        }
    };

    // ── Get content ─────────────────────────────────────────────
    // 优先用 agent 直传的 content（spark 文本、消息历史摘要等），
    // 否则按 content_ref 从 executor 缓存解析。
    let content = match call.input.get("content").and_then(|v| v.as_str()) {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => {
            let content_ref = call
                .input
                .get("content_ref")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match resolve_content(world, content_ref, last_tool_result) {
                Ok(content) => content,
                Err(error) => return (format!("shard_content: {error}"), true, None),
            }
        }
    };

    if content.is_empty() {
        return (
            "shard_content: no content found. Ensure content_ref references recent tool output."
                .into(),
            true,
            None,
        );
    }

    // ── Split + Store ───────────────────────────────────────────
    if let ShardStrategy::CustomRanges { ranges } = &strategy {
        let lines = content.lines().count();
        if ranges.iter().any(|(_, end)| *end > lines) {
            return (
                format!("shard_content: range exceeds source line count {lines}"),
                true,
                None,
            );
        }
    }
    let chunks = sharder.split(&content, &strategy);
    let total_shards = chunks.len() as u32;
    let now = unix_now();

    let source_label = format!("shard:{}", paradigm);
    let mut created = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let shard = ShardData {
            paradigm: paradigm.clone(),
            shard_index: i as u32,
            total_shards,
            content: chunk.clone(),
            source_label: source_label.clone(),
            created_at: now,
        };

        let entity = world.spawn_empty().id();
        created.push(entity);
        let json_str = serde_json::to_string(&shard)
            .unwrap_or_else(|_| r#"{"error":"serialize failed"}"#.into());
        if let Err(e) = registry.insert_component(world, entity, "ShardData", &json_str) {
            for entity in created {
                world.despawn(entity);
            }
            return (
                format!("shard_content: failed to create ShardData entity: {}", e),
                true,
                None,
            );
        }
    }

    if !call
        .input
        .get("content")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        if let Some(source) = call
            .input
            .get("content_ref")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("paradigm:"))
        {
            if let Some(entity) = created.first() {
                world
                    .entity_mut(*entity)
                    .insert(super::shard_catalog::ShardSources {
                        paradigms: vec![source.into()],
                    });
            }
        }
    }
    let action = AgentAction::ShardContent {
        paradigm: paradigm.clone(),
        strategy: strategy_str.to_string(),
        total_shards,
    };

    (
        format!(
            "Content projected into {} ShardData block(s). Paradigm: '{}'\n\
             Full content is stored in ECS, not semantically analyzed. \
             Query with: get_shards paradigm={} or query_entities component=ShardData, \
             then create any needed facts, indexes, or relations explicitly.",
            total_shards, paradigm, paradigm
        ),
        false,
        Some(action),
    )
}

/// Dispatch `get_shards` tool. Queries ECS for all ShardData with matching paradigm.
pub(crate) fn dispatch_get_shards(
    world: &World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let paradigm = match call
        .input
        .get("paradigm")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(s) => s,
        None => {
            return (
                "get_shards: missing 'paradigm' parameter".into(),
                true,
                None,
            );
        }
    };

    // Look up component id for ShardData
    let component_id = match registry.lookup_component_id(world, "ShardData") {
        Some(id) => id,
        None => {
            return (
                "No ShardData component registered in world".into(),
                true,
                None,
            );
        }
    };

    let mut shards: Vec<(u32, u32, String, usize)> = Vec::new();

    for archetype in world.archetypes().iter() {
        if !archetype.components().iter().any(|c| *c == component_id) {
            continue;
        }
        for arch_entity in archetype.entities() {
            let entity = arch_entity.id();
            if let Some(shard) = world.get::<ShardData>(entity) {
                if shard.paradigm == paradigm {
                    shards.push((
                        shard.shard_index,
                        entity.index().index(),
                        shard.content.chars().take(300).collect::<String>(),
                        shard.content.len(),
                    ));
                }
            }
        }
    }

    if shards.is_empty() {
        return (
            format!("No shards found for paradigm '{}'", paradigm),
            false,
            None,
        );
    }

    shards.sort_by_key(|(idx, id, _, _)| (*idx, *id));

    let mut output = format!(
        "{} shard(s) found for paradigm '{}':\n\n",
        shards.len(),
        paradigm
    );
    for (idx, id, content, bytes) in &shards {
        let preview: String = content.chars().take(300).collect();
        let suffix = if *bytes > content.len() { "..." } else { "" };
        output.push_str(&format!(
            "--- Shard {}/{} entity_id={} bytes={} ---\n{}{}\n\n",
            idx + 1,
            shards.len(),
            id,
            bytes,
            preview,
            suffix
        ));
    }

    (output, false, None)
}

/// Dispatch `get_shard_content`. Reads a bounded character window from one shard.
pub(crate) fn dispatch_get_shard_content(
    world: &World,
    registry: &ComponentActionRegistry,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    const DEFAULT_LIMIT: usize = 4_000;
    const MAX_LIMIT: usize = 12_000;

    let paradigm = match call
        .input
        .get("paradigm")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(s) => s,
        None => {
            return (
                "get_shard_content: missing 'paradigm' parameter".into(),
                true,
                None,
            );
        }
    };

    let shard_index = match call.input.get("shard_index").and_then(|v| v.as_u64()) {
        Some(idx) => idx as u32,
        None => {
            return (
                "get_shard_content: missing 'shard_index' parameter".into(),
                true,
                None,
            );
        }
    };

    let offset = call
        .input
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let limit = call
        .input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_LIMIT as u64) as usize;
    let limit = limit.clamp(1, MAX_LIMIT);

    let component_id = match registry.lookup_component_id(world, "ShardData") {
        Some(id) => id,
        None => {
            return (
                "No ShardData component registered in world".into(),
                true,
                None,
            );
        }
    };

    let mut found_for_paradigm = false;
    let requested_entity = match call.input.get("entity_id") {
        Some(v) => match v.as_u64() {
            Some(id) => Some(id),
            None => return ("invalid entity_id".into(), true, None),
        },
        None => None,
    };
    let matches: Vec<_> = world
        .archetypes()
        .iter()
        .flat_map(|a| a.entities())
        .filter_map(|e| {
            world
                .get::<ShardData>(e.id())
                .filter(|s| s.paradigm == paradigm && s.shard_index == shard_index)
                .map(|_| e.id().index().index() as u64)
        })
        .collect();
    if requested_entity.is_none() && matches.len() > 1 {
        return (
            format!(
                "Ambiguous shard index: {} matching entities. Use inspect_shards/get_shards and supply entity_id.",
                matches.len()
            ),
            true,
            None,
        );
    }
    if requested_entity.is_some_and(|id| !matches.contains(&id)) {
        return (
            "entity_id does not match the requested paradigm and shard_index".into(),
            true,
            None,
        );
    }
    for archetype in world.archetypes().iter() {
        if !archetype.components().iter().any(|c| *c == component_id) {
            continue;
        }
        for arch_entity in archetype.entities() {
            let entity = arch_entity.id();
            if requested_entity.is_some_and(|id| id != entity.index().index() as u64) {
                continue;
            }
            if let Some(shard) = world.get::<ShardData>(entity) {
                if shard.paradigm != paradigm {
                    continue;
                }
                found_for_paradigm = true;
                if shard.shard_index != shard_index {
                    continue;
                }

                let total_chars = shard.content.chars().count();
                if offset >= total_chars {
                    return (
                        format!(
                            "ShardData window is empty: paradigm='{}', shard_index={}, offset={} is past end (total_chars={})",
                            paradigm, shard_index, offset, total_chars
                        ),
                        false,
                        None,
                    );
                }

                let window: String = shard.content.chars().skip(offset).take(limit).collect();
                let returned_chars = window.chars().count();
                let next_offset = offset + returned_chars;
                let has_more = next_offset < total_chars;
                let output = format!(
                    "ShardData content window\n\
                         paradigm: {}\n\
                         shard_index: {}/{}\n\
                         offset: {}\n\
                         returned_chars: {}\n\
                         total_chars: {}\n\
                         has_more: {}\n\
                         next_offset: {}\n\n\
                         --- content ---\n{}",
                    paradigm,
                    shard.shard_index,
                    shard.total_shards,
                    offset,
                    returned_chars,
                    total_chars,
                    has_more,
                    next_offset,
                    window
                );
                return (output, false, None);
            }
        }
    }

    if found_for_paradigm {
        (
            format!(
                "No shard found for paradigm '{}' with shard_index {}",
                paradigm, shard_index
            ),
            false,
            None,
        )
    } else {
        (
            format!("No shards found for paradigm '{}'", paradigm),
            false,
            None,
        )
    }
}

/// Resolve content from a reference string.
///
/// `__last_tool_result` resolves to the most recent tool output cached by the executor.
/// `paradigm:<id>` concatenates a complete group's stored chunks verbatim.
/// Overlap is retained; arbitrary historical tool_call_id references are unsupported.
fn resolve_content(
    world: &mut World,
    content_ref: &str,
    last_tool_result: Option<&str>,
) -> Result<String, String> {
    if content_ref == "__last_tool_result" {
        last_tool_result
            .map(str::to_owned)
            .ok_or_else(|| "no cached tool result".into())
    } else if let Some(paradigm) = content_ref.strip_prefix("paradigm:") {
        if let Some(notice) = crate::shard_retention::expired_notice(world, paradigm) {
            return Err(notice);
        }
        let mut shards: Vec<_> = world
            .query::<&ShardData>()
            .iter(world)
            .filter(|s| s.paradigm == paradigm)
            .collect();
        shards.sort_by_key(|s| s.shard_index);
        if shards.is_empty() {
            return Err("source paradigm not found".into());
        }
        if !shards
            .iter()
            .enumerate()
            .all(|(i, s)| s.shard_index as usize == i && s.total_shards as usize == shards.len())
        {
            return Err("source group is incomplete or has duplicate indices".into());
        }
        let content = shards
            .iter()
            .map(|s| s.content.as_str())
            .collect::<String>();
        crate::shard_retention::touch(world, paradigm, unix_now());
        Ok(content)
    } else {
        Err("unsupported content_ref; use __last_tool_result or paradigm:<group>".into())
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn duplicate_legacy_indices_require_exact_entity_id() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let mut ids = vec![];
        for text in ["first archive", "second archive"] {
            ids.push(
                rt.world
                    .spawn(ShardData {
                        paradigm: "spark-archive".into(),
                        shard_index: 0,
                        total_shards: 1,
                        content: text.into(),
                        source_label: "legacy".into(),
                        created_at: 0,
                    })
                    .id(),
            );
        }
        let mut request = ToolCall {
            id: "read".into(),
            name: "get_shard_content".into(),
            input: json!({"paradigm":"spark-archive","shard_index":0}),
        };
        assert!(dispatch_get_shard_content(&rt.world, &rt.component_registry, &request).1);
        request.input["entity_id"] = json!(ids[1].index().index());
        let (text, error, _) =
            dispatch_get_shard_content(&rt.world, &rt.component_registry, &request);
        assert!(!error);
        assert!(text.contains("second archive"));
        assert!(!text.contains("first archive"));
        request.input["paradigm"] = json!("wrong");
        assert!(dispatch_get_shard_content(&rt.world, &rt.component_registry, &request).1);
    }

    fn call(
        rt: &mut crate::AgentRuntime,
        input: serde_json::Value,
    ) -> (String, bool, Option<AgentAction>) {
        dispatch_shard_content(
            &mut rt.world,
            &mut rt.component_registry,
            &rt.sharder,
            &ToolCall {
                id: "test".into(),
                name: "shard_content".into(),
                input,
            },
            None,
        )
    }

    #[test]
    fn reshard_existing_projection_and_reject_duplicate_destination() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let content = "第一行\n第二行\n第三行";
        let (source, _) = crate::context_artifact::store(
            &mut rt.world,
            &rt.component_registry,
            "context:test",
            "test",
            content,
        )
        .unwrap();
        let input = json!({"content_ref":format!("paradigm:{source}"),"strategy":"custom_ranges","ranges":[[1,3]],"paradigm":"derived"});
        let result = call(&mut rt, input.clone());
        assert!(!result.1, "{}", result.0);
        let text = resolve_content(&mut rt.world, "paradigm:derived", None).unwrap();
        assert_eq!(text, "第二行\n第三行");
        assert_eq!(
            resolve_content(&mut rt.world, &format!("paradigm:{source}"), None).unwrap(),
            content
        );
        assert!(call(&mut rt, input).1);
    }

    #[test]
    fn references_validate_complete_groups_and_preserve_chunk_bytes() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for (index, text) in [(1, "shared\nend"), (0, "start\nshared\n")] {
            rt.world.spawn(ShardData {
                paradigm: "source".into(),
                shard_index: index,
                total_shards: 2,
                content: text.into(),
                source_label: "test".into(),
                created_at: 0,
            });
        }
        assert_eq!(
            resolve_content(&mut rt.world, "paradigm:source", None).unwrap(),
            "start\nshared\nshared\nend"
        );
        let entity = rt
            .world
            .query::<(bevy_ecs::entity::Entity, &ShardData)>()
            .iter(&rt.world)
            .next()
            .unwrap()
            .0;
        rt.world.despawn(entity);
        let before = rt.world.entities().len();
        assert!(
            call(
                &mut rt,
                json!({"content_ref":"paradigm:source","strategy":"custom","paradigm":"target"})
            )
            .1
        );
        assert_eq!(rt.world.entities().len(), before);
        assert!(
            resolve_content(&mut rt.world, "tool_call:missing", None)
                .unwrap_err()
                .contains("unsupported")
        );
        assert!(resolve_content(&mut rt.world, "paradigm:missing", None).is_err());
        assert_eq!(
            resolve_content(&mut rt.world, "__last_tool_result", Some("cached")).unwrap(),
            "cached"
        );
    }

    #[test]
    fn invalid_ranges_and_numeric_parameters_do_not_write() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let before = rt.world.entities().len();
        for ranges in [
            json!([[0, 1], [2]]),
            json!([[1, 1]]),
            json!([[-1, 1]]),
            json!([[0, 4]]),
        ] {
            assert!(call(&mut rt,json!({"content":"a\nb\nc","strategy":"custom_ranges","ranges":ranges,"paradigm":"bad"})).1);
        }
        for input in [
            json!({"content":"a","strategy":"overlap","overlap_size":501,"paradigm":"bad"}),
            json!({"content":"a","strategy":"head_tail","head_lines":-1,"paradigm":"bad"}),
        ] {
            assert!(call(&mut rt, input).1);
        }
        assert_eq!(rt.world.entities().len(), before);
        assert!(!call(&mut rt,json!({"content":"a\nb\nc\nd","strategy":"head_tail","head_lines":1,"tail_lines":1,"paradigm":"parts"})).1);
        let mut parts: Vec<_> = rt
            .world
            .query::<&ShardData>()
            .iter(&rt.world)
            .filter(|s| s.paradigm == "parts")
            .map(|s| (s.shard_index, s.content.clone()))
            .collect();
        parts.sort();
        assert_eq!(
            parts,
            vec![(0, "a".into()), (1, "b\nc".into()), (2, "d".into())]
        );
    }
}
