//! Bounded queries over runtime delivery facts and queue state.

use bevy_ecs::{entity::Entity, world::World};
use big_head_ecs::{
    Spark, SparkStatus,
    execution::{RuntimeIdentity, RuntimeJournal, SparkDispatch},
};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub(crate) fn inspect(world: &mut World, queue: &VecDeque<Entity>, input: &Value) -> Value {
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    let after = input
        .get("after_sequence")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let correlation = input.get("correlation_id").and_then(Value::as_str);
    let source = input.get("source_id").and_then(Value::as_str);
    let journal = world
        .query::<&RuntimeJournal>()
        .iter(world)
        .next()
        .cloned()
        .unwrap_or_default();
    let events: Vec<_> = journal
        .events
        .iter()
        .rev()
        .filter(|e| {
            e.sequence > after
                && correlation.is_none_or(|c| e.correlation_id == c)
                && source.is_none_or(|s| e.source_id == s)
        })
        .take(limit)
        .cloned()
        .collect();
    let mut events = events;
    events.reverse();
    let mut sparks: Vec<_> = world
        .query::<(
            Entity,
            &Spark,
            Option<&RuntimeIdentity>,
            Option<&SparkDispatch>,
        )>()
        .iter(world)
        .filter(|(_, _, identity, dispatch)| {
            correlation.is_none_or(|c| dispatch.is_some_and(|d| d.correlation_id == c))
                && source.is_none_or(|s| {
                    identity.is_some_and(|i| i.id == s)
                        || dispatch.is_some_and(|d| d.source_id == s)
                })
        })
        .collect();
    sparks.sort_by_key(|(e, s, _, dispatch)| {
        (
            !matches!(s.status, SparkStatus::Pending | SparkStatus::Processing),
            queue
                .iter()
                .position(|queued| queued == e)
                .unwrap_or(usize::MAX),
            std::cmp::Reverse(dispatch.map(|d| d.sequence).unwrap_or(s.timestamp)),
        )
    });
    let matching_sparks = sparks.len();
    let sparks: Vec<_> = sparks
        .into_iter()
        .take(limit)
        .map(|(entity, spark, identity, dispatch)| {
            json!({
                "entity_id": entity.index().index(), "stable_id": identity.map(|id| &id.id),
                "status": spark.status, "dispatch": dispatch,
                "queue_position": queue.iter().position(|e| *e == entity),
                "orphan_pending": spark.status == SparkStatus::Pending && !queue.contains(&entity),
                "text": spark.text.chars().take(240).collect::<String>(),
                "result": spark.result_summary.chars().take(400).collect::<String>(),
            })
        })
        .collect();
    json!({"events": events, "latest_sequence": journal.next_sequence, "dropped_events": journal.dropped,
        "retention_limit": big_head_ecs::execution::EVENT_CAPACITY, "queued_count": queue.len(),
        "matching_sparks": matching_sparks, "sparks": sparks})
}
