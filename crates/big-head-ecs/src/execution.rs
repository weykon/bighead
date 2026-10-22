//! Bounded delivery facts. Business meaning remains agent-authored.

use std::collections::VecDeque;

use bevy_ecs::{component::Component, entity::Entity, resource::Resource, world::World};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::{Spark, SparkStatus};

pub const EVENT_CAPACITY: usize = 1024;

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub id: String,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct SparkDispatch {
    pub sequence: u64,
    pub interactive: bool,
    pub source_id: String,
    pub correlation_id: String,
    pub attempts: u32,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct DispatchContext {
    pub source_id: String,
    pub correlation_id: String,
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeEventKind {
    TickStarted,
    TickFinished,
    SparkQueued,
    SparkStarted,
    SparkCompleted,
    SparkFailed,
    SparkInterrupted,
    SparkRecovered,
    ObserverLeased,
    ObserverAcknowledged,
    ObserverDeferred,
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub sequence: u64,
    pub kind: RuntimeEventKind,
    pub source_id: String,
    pub correlation_id: String,
    pub timestamp: u64,
    pub summary: String,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeJournal {
    pub next_sequence: u64,
    pub dropped: u64,
    pub events: VecDeque<RuntimeEvent>,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn identity(world: &mut World, entity: Entity) -> String {
    if let Some(identity) = world.get::<RuntimeIdentity>(entity) {
        return identity.id.clone();
    }
    let id = uuid::Uuid::new_v4().to_string();
    world
        .entity_mut(entity)
        .insert(RuntimeIdentity { id: id.clone() });
    id
}

pub fn journal_event(
    world: &mut World,
    kind: RuntimeEventKind,
    source_id: &str,
    correlation_id: &str,
    summary: &str,
) -> u64 {
    let entity = world
        .query::<(Entity, &RuntimeJournal)>()
        .iter(world)
        .next()
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| world.spawn(RuntimeJournal::default()).id());
    let mut journal = world
        .get_mut::<RuntimeJournal>(entity)
        .expect("journal exists");
    journal.next_sequence = journal.next_sequence.saturating_add(1);
    let sequence = journal.next_sequence;
    journal.events.push_back(RuntimeEvent {
        sequence,
        kind,
        source_id: source_id.into(),
        correlation_id: correlation_id.into(),
        timestamp: now_secs(),
        summary: summary.chars().take(800).collect(),
    });
    while journal.events.len() > EVENT_CAPACITY {
        journal.events.pop_front();
        journal.dropped = journal.dropped.saturating_add(1);
    }
    sequence
}

/// Used by both Rhai and native tools so there is only one enqueue contract.
pub fn enqueue(
    world: &mut World,
    queue: &mut VecDeque<Entity>,
    spark: Spark,
    interactive: bool,
) -> Entity {
    let entity = world.spawn(spark).id();
    attach_dispatch(world, entity, interactive);
    if interactive {
        let index = queue
            .iter()
            .position(|entity| {
                !world
                    .get::<SparkDispatch>(*entity)
                    .is_some_and(|d| d.interactive)
            })
            .unwrap_or(queue.len());
        queue.insert(index, entity);
    } else {
        queue.push_back(entity);
    }
    entity
}

pub fn attach_dispatch(world: &mut World, entity: Entity, interactive: bool) {
    if world.get::<SparkDispatch>(entity).is_some() {
        return;
    }
    let id = identity(world, entity);
    let context = world
        .get_resource::<DispatchContext>()
        .cloned()
        .unwrap_or_default();
    let correlation_id = if context.correlation_id.is_empty() {
        id.clone()
    } else {
        context.correlation_id
    };
    let summary = world
        .get::<Spark>(entity)
        .map(|s| s.text.clone())
        .unwrap_or_default();
    let sequence = journal_event(
        world,
        RuntimeEventKind::SparkQueued,
        &id,
        &correlation_id,
        &summary,
    );
    world.entity_mut(entity).insert(SparkDispatch {
        sequence,
        interactive,
        source_id: context.source_id,
        correlation_id,
        attempts: 0,
    });
}

pub fn spark_event(world: &mut World, entity: Entity, kind: RuntimeEventKind, summary: &str) {
    let id = identity(world, entity);
    let correlation = world
        .get::<SparkDispatch>(entity)
        .map(|d| d.correlation_id.clone())
        .unwrap_or_else(|| id.clone());
    journal_event(world, kind, &id, &correlation, summary);
}

/// Legacy checkpoints have no dispatch metadata; their timestamps supply a
/// deterministic migration order. Interrupted work has a bounded retry count.
pub fn restore_queue(world: &mut World) -> VecDeque<Entity> {
    let mut pending: Vec<_> = world
        .query::<(Entity, &Spark)>()
        .iter(world)
        .filter(|(_, s)| matches!(s.status, SparkStatus::Pending | SparkStatus::Processing))
        .map(|(e, s)| (s.timestamp, e))
        .collect();
    pending.sort_by_key(|(ts, e)| (*ts, e.index().index()));
    let mut queue = Vec::new();
    for (_, entity) in pending {
        attach_dispatch(world, entity, false);
        if world.get::<Spark>(entity).unwrap().status == SparkStatus::Processing {
            let attempts = world.get::<SparkDispatch>(entity).unwrap().attempts;
            let exhausted = attempts >= 3;
            let reason = if exhausted {
                "Recovery retry limit reached; inspect effects before retrying"
            } else {
                "Interrupted attempt recovered; inspect previous effects before continuing"
            };
            let mut spark = world.get_mut::<Spark>(entity).unwrap();
            spark.status = if exhausted {
                SparkStatus::Failed
            } else {
                SparkStatus::Pending
            };
            spark.result_summary = reason.into();
            spark_event(
                world,
                entity,
                if exhausted {
                    RuntimeEventKind::SparkFailed
                } else {
                    RuntimeEventKind::SparkRecovered
                },
                reason,
            );
            if exhausted {
                continue;
            }
        }
        let dispatch = world.get::<SparkDispatch>(entity).unwrap();
        queue.push((!dispatch.interactive, dispatch.sequence, entity));
    }
    queue.sort_by_key(|(background, sequence, entity)| {
        (*background, *sequence, entity.index().index())
    });
    queue.into_iter().map(|(_, _, entity)| entity).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_preserves_user_fifo_and_causality() {
        let mut world = World::new();
        let mut queue = VecDeque::new();
        world.insert_resource(DispatchContext {
            source_id: "tick-1".into(),
            correlation_id: "run-1".into(),
        });
        let background = enqueue(&mut world, &mut queue, Spark::default(), false);
        world.remove_resource::<DispatchContext>();
        let first = enqueue(&mut world, &mut queue, Spark::default(), true);
        let second = enqueue(&mut world, &mut queue, Spark::default(), true);
        assert_eq!(queue, VecDeque::from([first, second, background]));
        assert_eq!(restore_queue(&mut world), queue);
        let dispatch = world.get::<SparkDispatch>(background).unwrap();
        assert_eq!(dispatch.source_id, "tick-1");
        assert_eq!(dispatch.correlation_id, "run-1");
    }

    #[test]
    fn journal_retention_preserves_monotonic_sequence() {
        let mut world = World::new();
        for _ in 0..EVENT_CAPACITY + 5 {
            journal_event(
                &mut world,
                RuntimeEventKind::TickFinished,
                "tick",
                "run",
                "done",
            );
        }
        let journal = world.query::<&RuntimeJournal>().single(&world).unwrap();
        assert_eq!(journal.events.len(), EVENT_CAPACITY);
        assert_eq!(journal.events.front().unwrap().sequence, 6);
        assert_eq!(journal.dropped, 5);
        assert_eq!(journal.next_sequence, (EVENT_CAPACITY + 5) as u64);
    }

    #[test]
    fn processing_recovery_is_bounded_and_idempotent() {
        let mut world = World::new();
        let mut queue = VecDeque::new();
        let retry = enqueue(
            &mut world,
            &mut queue,
            Spark {
                status: SparkStatus::Processing,
                ..Default::default()
            },
            false,
        );
        let exhausted = enqueue(
            &mut world,
            &mut queue,
            Spark {
                status: SparkStatus::Processing,
                ..Default::default()
            },
            false,
        );
        world.get_mut::<SparkDispatch>(retry).unwrap().attempts = 1;
        world.get_mut::<SparkDispatch>(exhausted).unwrap().attempts = 3;
        assert_eq!(restore_queue(&mut world), VecDeque::from([retry]));
        assert_eq!(restore_queue(&mut world), VecDeque::from([retry]));
        assert_eq!(
            world.get::<Spark>(retry).unwrap().status,
            SparkStatus::Pending
        );
        assert_eq!(
            world.get::<Spark>(exhausted).unwrap().status,
            SparkStatus::Failed
        );
    }
}
