//! Shared, lossless ECS storage for context payloads and Lens receipts.
use bevy_ecs::{entity::Entity, world::World};
use big_head_ecs::ComponentActionRegistry;
use big_head_shard::ShardData;

const CHUNK_BYTES: usize = 16 * 1024;

pub(crate) fn store(
    world: &mut World,
    registry: &ComponentActionRegistry,
    namespace: &str,
    label: &str,
    content: &str,
) -> Result<(String, Vec<Entity>), String> {
    let paradigm = format!(
        "{namespace}:sha256:{}",
        big_head_save::persistence::digest(content.as_bytes())
    );
    let mut existing: Vec<_> = world
        .query::<(Entity, &ShardData)>()
        .iter(world)
        .filter(|(_, s)| s.paradigm == paradigm)
        .map(|(e, s)| (e, s.shard_index, s.total_shards, s.content.clone()))
        .collect();
    existing.sort_by_key(|(_, index, _, _)| *index);
    if !existing.is_empty() {
        let complete = existing
            .iter()
            .enumerate()
            .all(|(i, (_, index, total, _))| *index == i as u32 && *total == existing.len() as u32);
        if !complete
            || existing
                .iter()
                .map(|(_, _, _, s)| s.as_str())
                .collect::<String>()
                != content
        {
            return Err("inconsistent existing context artifact".into());
        }
        crate::shard_retention::touch(world, &paradigm, big_head_ecs::execution::now_secs());
        return Ok((
            paradigm,
            existing.into_iter().map(|(e, _, _, _)| e).collect(),
        ));
    }
    let mut chunks = Vec::new();
    let mut remaining = content;
    while !remaining.is_empty() {
        let mut end = remaining.len().min(CHUNK_BYTES);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(&remaining[..end]);
        remaining = &remaining[end..];
    }
    if chunks.is_empty() {
        chunks.push("");
    }
    let total = u32::try_from(chunks.len()).map_err(|_| "too many context shards")?;
    let mut entities = Vec::new();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let shard = ShardData {
            paradigm: paradigm.clone(),
            shard_index: index as u32,
            total_shards: total,
            content: chunk.into(),
            source_label: label.into(),
            created_at: big_head_ecs::execution::now_secs(),
        };
        let serialized = serde_json::to_string(&shard).map_err(|e| e.to_string())?;
        let entity = world.spawn_empty().id();
        entities.push(entity);
        if let Err(error) = registry.insert_component(world, entity, "ShardData", &serialized) {
            for entity in entities {
                world.despawn(entity);
            }
            return Err(error.to_string());
        }
    }
    crate::shard_retention::manage_new(world, &paradigm);
    Ok((paradigm, entities))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_roundtrip_reuses_identical_content() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let content = "\u{4e2d}\u{1f30d}".repeat(6000);
        let (first, entities) = store(
            &mut rt.world,
            &rt.component_registry,
            "test",
            "source",
            &content,
        )
        .unwrap();
        let mut restored = String::new();
        for (index, entity) in entities.iter().enumerate() {
            let shard = rt.world.get::<ShardData>(*entity).unwrap();
            assert!(shard.content.len() <= CHUNK_BYTES);
            assert_eq!(shard.shard_index, index as u32);
            assert_eq!(shard.total_shards, entities.len() as u32);
            restored.push_str(&shard.content);
        }
        assert_eq!(restored, content);
        let (second, _) = store(
            &mut rt.world,
            &rt.component_registry,
            "test",
            "source",
            &content,
        )
        .unwrap();
        assert_eq!(first, second);
    }
}
