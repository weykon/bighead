//! Bounded runtime observations for agent-driven world stewardship.
//!
//! The runtime owns delivery and backpressure. Semantic persistence remains an
//! agent decision: observations are offered to an LLM system, which may use
//! normal ECS tools to define and update durable world components.

use std::collections::{HashMap, VecDeque};

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use big_head_ecs::execution::RuntimeIdentity;
use serde::{Deserialize, Serialize};

const ROUTINE_REPEAT_THRESHOLD: u32 = 12;
const RETRY_BACKOFF_SECS: u64 = 300;

/// One TickAction execution report produced by the deterministic runtime.
#[derive(Debug, Clone)]
pub struct TickActionReport {
    pub entity: Entity,
    pub source_id: String,
    pub correlation_id: String,
    pub spark_ids: Vec<String>,
    pub script: String,
    pub script_hash: String,
    pub started_at_epoch: u64,
    pub duration_ms: u64,
    pub queued_sparks: usize,
    pub logs: Vec<String>,
    pub error: Option<String>,
    pub state: Option<TickState>,
}

/// Script-authored semantic state, not proof of successful business work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickState {
    pub state_key: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub urgent: bool,
}

impl TickState {
    pub(crate) fn from_return(value: &rhai::Dynamic) -> Result<Option<Self>, String> {
        if !value.is_map() {
            return Ok(None);
        }
        let map = value.clone().cast::<rhai::Map>();
        let Some(raw) = map.get("tick_observation") else {
            return Ok(None);
        };
        let raw = crate::scripting::dynamic_to_json(raw.clone());
        if raw.to_string().len() > 8192 {
            return Err("tick_observation exceeds 8 KiB".into());
        }
        let state: Self =
            serde_json::from_value(raw).map_err(|e| format!("invalid tick_observation: {e}"))?;
        if state.state_key.trim().is_empty()
            || state.state_key.len() > 1024
            || state.summary.len() > 4096
        {
            return Err("tick_observation requires a nonempty state_key <=1024 bytes and summary <=4096 bytes".into());
        }
        Ok(Some(state))
    }
}

impl TickActionReport {
    fn semantic_fingerprint(&self) -> u64 {
        let mut material = String::with_capacity(256);
        material.push_str(&self.script_hash);
        material.push('|');
        if let Some(state) = &self.state {
            material.push_str("state:");
            material.push_str(&state.state_key);
            material.push_str(if state.urgent { ":urgent" } else { ":routine" });
        } else {
            for log in &self.logs {
                if self.error.is_some() {
                    material.push_str(log);
                } else {
                    material.push_str(&normalize_volatile_values(log));
                }
                material.push('\n');
            }
        }
        if let Some(error) = &self.error {
            material.push_str("error:");
            material.push_str(error);
        }
        stable_hash(material.as_bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObservationPriority {
    Routine,
    Notable,
    Urgent,
}

impl ObservationPriority {
    fn as_str(self) -> &'static str {
        match self {
            Self::Routine => "routine",
            Self::Notable => "notable",
            Self::Urgent => "urgent",
        }
    }
}

/// Compact, prompt-safe projection of a TickAction report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickObservation {
    pub sequence: u64,
    #[serde(skip, default = "missing_entity")]
    pub entity: Entity,
    pub source_id: String,
    pub correlation_id: String,
    pub spark_ids: Vec<String>,
    pub label: String,
    pub script_hash: String,
    pub started_at_epoch: u64,
    pub duration_ms: u64,
    pub queued_sparks: usize,
    pub priority: ObservationPriority,
    pub repeat_count: u32,
    pub logs: Vec<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub state: Option<TickState>,
}

impl TickObservation {
    fn from_report(
        sequence: u64,
        report: &TickActionReport,
        priority: ObservationPriority,
        repeat_count: u32,
    ) -> Self {
        Self {
            sequence,
            entity: report.entity,
            source_id: report.source_id.clone(),
            correlation_id: report.correlation_id.clone(),
            spark_ids: report.spark_ids.clone(),
            label: script_label(&report.script),
            script_hash: report.script_hash.clone(),
            started_at_epoch: report.started_at_epoch,
            duration_ms: report.duration_ms,
            queued_sparks: report.queued_sparks,
            priority,
            repeat_count,
            logs: report
                .logs
                .iter()
                .take(6)
                .map(|line| compact(line, 500))
                .collect(),
            error: report.error.as_deref().map(|error| compact(error, 800)),
            state: report.state.clone(),
        }
    }

    fn render(&self) -> String {
        let mut out = format!(
            "### event={} priority={} tick_action={:?} label={}\n\
             ran_at={} duration_ms={} script_hash={} queued_sparks={} repeats={}\n",
            self.sequence,
            self.priority.as_str(),
            self.entity,
            self.label,
            self.started_at_epoch,
            self.duration_ms,
            self.script_hash,
            self.queued_sparks,
            self.repeat_count,
        );
        out.push_str(&format!(
            "source_id={} correlation_id={} spark_ids={:?}\n",
            self.source_id, self.correlation_id, self.spark_ids
        ));
        if !self.logs.is_empty() {
            out.push_str("logs:\n");
            for log in &self.logs {
                out.push_str("- ");
                out.push_str(log);
                out.push('\n');
            }
        }
        if let Some(state) = &self.state {
            out.push_str(&format!(
                "script_declared_state={}\n",
                serde_json::to_string(state).unwrap()
            ));
        }
        if let Some(error) = &self.error {
            out.push_str("error: ");
            out.push_str(error);
            out.push('\n');
        }
        out
    }
}

trait Sequenced {
    fn sequence(&self) -> u64;
}

impl Sequenced for TickObservation {
    fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// A cloned lease keeps events in the inbox until the consumer acknowledges
/// successful handling. This gives the observer at-least-once delivery without
/// holding mutable runtime borrows across an async model request.
#[derive(Debug, Clone)]
pub struct ObservationLease<T> {
    through_sequence: u64,
    pub items: Vec<T>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BoundedInbox<T, const CAPACITY: usize> {
    queue: VecDeque<T>,
    dropped: u64,
    retry_after_epoch: u64,
}

impl<T, const CAPACITY: usize> Default for BoundedInbox<T, CAPACITY> {
    fn default() -> Self {
        assert!(CAPACITY > 0, "observation inbox capacity must be positive");
        Self {
            queue: VecDeque::with_capacity(CAPACITY),
            dropped: 0,
            retry_after_epoch: 0,
        }
    }
}

impl<T: Sequenced + Clone, const CAPACITY: usize> BoundedInbox<T, CAPACITY> {
    fn push(&mut self, item: T) {
        if self.queue.len() == CAPACITY {
            self.queue.pop_front();
            self.dropped += 1;
        }
        self.queue.push_back(item);
    }

    fn lease(&self, now: u64, max_items: usize) -> Option<ObservationLease<T>> {
        if now < self.retry_after_epoch {
            return None;
        }
        let items = self
            .queue
            .iter()
            .take(max_items)
            .cloned()
            .collect::<Vec<_>>();
        let through_sequence = items.last()?.sequence();
        Some(ObservationLease {
            through_sequence,
            items,
        })
    }

    fn acknowledge(&mut self, lease: &ObservationLease<T>) {
        while self
            .queue
            .front()
            .is_some_and(|item| item.sequence() <= lease.through_sequence)
        {
            self.queue.pop_front();
        }
        self.retry_after_epoch = 0;
    }

    fn defer(&mut self, now: u64) {
        self.retry_after_epoch = now.saturating_add(RETRY_BACKOFF_SECS);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SeenState {
    fingerprint: u64,
    suppressed_repeats: u32,
    emitted: bool,
}

/// Domain gate layered over a generic bounded inbox.
#[derive(Debug, Serialize, Deserialize)]
pub struct TickObservationPipeline<const CAPACITY: usize> {
    inbox: BoundedInbox<TickObservation, CAPACITY>,
    seen: HashMap<String, SeenState>,
    next_sequence: u64,
    #[serde(default)]
    retry_batch_limit: Option<usize>,
}

impl<const CAPACITY: usize> Default for TickObservationPipeline<CAPACITY> {
    fn default() -> Self {
        Self {
            inbox: BoundedInbox::default(),
            seen: HashMap::new(),
            next_sequence: 1,
            retry_batch_limit: None,
        }
    }
}

impl<const CAPACITY: usize> TickObservationPipeline<CAPACITY> {
    pub fn ingest(&mut self, reports: &[TickActionReport]) {
        for report in reports {
            let fingerprint = report.semantic_fingerprint();
            let state = self
                .seen
                .entry(report.source_id.clone())
                .or_insert(SeenState {
                    fingerprint,
                    suppressed_repeats: 0,
                    emitted: false,
                });
            let changed = state.fingerprint != fingerprint;
            if changed {
                state.fingerprint = fingerprint;
                state.suppressed_repeats = 0;
            } else {
                state.suppressed_repeats = state.suppressed_repeats.saturating_add(1);
            }

            let urgent = report.error.is_some() || report.state.as_ref().is_some_and(|s| s.urgent);
            let first_seen = !state.emitted;
            let repeated_threshold = state.suppressed_repeats >= ROUTINE_REPEAT_THRESHOLD;
            let creates_work = report.queued_sparks > 0;
            if !(first_seen || changed || repeated_threshold || creates_work || urgent) {
                continue;
            }

            let priority = if urgent {
                ObservationPriority::Urgent
            } else if changed || creates_work {
                ObservationPriority::Notable
            } else {
                ObservationPriority::Routine
            };
            let repeat_count = state.suppressed_repeats.max(1);
            state.suppressed_repeats = 0;
            state.emitted = true;

            let event =
                TickObservation::from_report(self.next_sequence, report, priority, repeat_count);
            self.next_sequence = self.next_sequence.saturating_add(1);
            self.inbox.push(event);
        }
    }

    pub fn lease(&self, now: u64, max_items: usize) -> Option<ObservationLease<TickObservation>> {
        self.inbox.lease(
            now,
            max_items.min(self.retry_batch_limit.unwrap_or(max_items).max(1)),
        )
    }

    pub fn acknowledge(&mut self, lease: &ObservationLease<TickObservation>) {
        self.inbox.acknowledge(lease);
        if self.inbox.queue.is_empty() {
            self.retry_batch_limit = None;
        }
    }

    pub fn defer(&mut self, now: u64) {
        self.inbox.defer(now);
    }

    pub fn defer_failed_batch(&mut self, now: u64, lease: &ObservationLease<TickObservation>) {
        // Keep every event pending, but reduce work per attempt after a failure.
        self.retry_batch_limit = Some((lease.items.len() / 2).max(1));
        self.inbox.defer(now);
    }

    pub fn rebind(&mut self, world: &mut World) {
        let ids: HashMap<_, _> = world
            .query::<(Entity, &RuntimeIdentity)>()
            .iter(world)
            .map(|(entity, id)| (id.id.clone(), entity))
            .collect();
        for item in &mut self.inbox.queue {
            item.entity = ids
                .get(&item.source_id)
                .copied()
                .unwrap_or(Entity::PLACEHOLDER);
        }
        self.seen.retain(|id, _| ids.contains_key(id));
        while self.inbox.queue.len() > CAPACITY {
            self.inbox.queue.pop_front();
            self.inbox.dropped += 1;
        }
    }

    pub fn render_context(&self, lease: &ObservationLease<TickObservation>) -> String {
        let events = lease
            .items
            .iter()
            .map(TickObservation::render)
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "## TickAction Observation Batch\n\n\
             These are transient runtime facts. They are not durable ECS understanding yet.\n\
             Inspect the live world before mutating it. Decide whether to ignore, record, repair, or escalate.\n\
             inbox_dropped={} batch_size={}\n\n{}",
            self.inbox.dropped,
            lease.items.len(),
            events,
        )
    }
}

fn missing_entity() -> Entity {
    Entity::PLACEHOLDER
}

pub(crate) fn stable_script_hash(script: &str) -> String {
    format!("{:016x}", stable_hash(script.as_bytes()))
}

pub(crate) fn stable_hash(bytes: &[u8]) -> u64 {
    // FNV-1a is sufficient here: this is an optimistic concurrency token, not
    // a cryptographic identity.
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn normalize_volatile_values(input: &str) -> String {
    input
        .split_whitespace()
        .map(|token| match token.split_once('=') {
            Some(("snapshot", value))
                if value.strip_prefix("archetype-").is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
                }) =>
            {
                "snapshot=archetype-#".to_owned()
            }
            Some((key, _))
                if matches!(
                    key,
                    "wave"
                        | "current_wave"
                        | "total"
                        | "total_actions"
                        | "ts"
                        | "timestamp"
                        | "now_ts"
                        | "quiet_secs"
                        | "updated"
                ) =>
            {
                format!("{key}=#")
            }
            _ => token.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn script_label(script: &str) -> String {
    script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('/').trim())
        .filter(|line| !line.is_empty())
        .map(|line| compact(line, 80))
        .unwrap_or_else(|| "unnamed-tick-action".into())
}

fn compact(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        let mut prefix = normalized.chars().take(max_chars).collect::<String>();
        prefix.push_str("...");
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_batches_shrink_durably_without_acknowledging_other_events() {
        let mut pipeline = TickObservationPipeline::<64>::default();
        for id in 0..8 {
            pipeline.ingest(&[report(Entity::from_raw_u32(id).unwrap(), "healthy")]);
        }
        let first = pipeline.lease(100, 8).unwrap();
        pipeline.defer_failed_batch(100, &first);
        assert!(pipeline.lease(399, 8).is_none());
        let encoded = serde_json::to_string(&pipeline).unwrap();
        let mut restored: TickObservationPipeline<64> = serde_json::from_str(&encoded).unwrap();
        let smaller = restored.lease(400, 8).unwrap();
        assert_eq!(smaller.items.len(), 4);
        assert_eq!(smaller.items[0].sequence, first.items[0].sequence);
        restored.defer_failed_batch(400, &smaller);
        let two = restored.lease(700, 8).unwrap();
        assert_eq!(two.items.len(), 2);
        restored.defer_failed_batch(700, &two);
        let one = restored.lease(1000, 8).unwrap();
        assert_eq!(one.items.len(), 1);
        restored.defer_failed_batch(1000, &one);
        let mut acknowledged = Vec::new();
        while let Some(lease) = restored.lease(1300, 8) {
            assert_eq!(lease.items.len(), 1);
            acknowledged.push(lease.items[0].sequence);
            restored.acknowledge(&lease);
        }
        assert_eq!(acknowledged, (1..=8).collect::<Vec<_>>());
        assert_eq!(restored.retry_batch_limit, None);
        let mut legacy = serde_json::to_value(&pipeline).unwrap();
        legacy.as_object_mut().unwrap().remove("retry_batch_limit");
        let legacy: TickObservationPipeline<64> = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.lease(400, 8).unwrap().items.len(), 8);
    }

    fn report(entity: Entity, log: &str) -> TickActionReport {
        TickActionReport {
            entity,
            source_id: format!("test-{entity:?}"),
            correlation_id: "test-run".into(),
            spark_ids: Vec::new(),
            script: "// ambient heartbeat\nlog(\"ok\");".into(),
            script_hash: stable_script_hash("// ambient heartbeat\nlog(\"ok\");"),
            started_at_epoch: 100,
            duration_ms: 2,
            queued_sparks: 0,
            logs: vec![log.into()],
            error: None,
            state: None,
        }
    }

    #[test]
    fn explicit_state_suppresses_log_churn_but_not_errors_work_or_state_changes() {
        let entity = Entity::from_raw_u32(7).unwrap();
        let mut pipeline = TickObservationPipeline::<64>::default();
        let mut r = report(entity, "snapshot=archetype-100 total=100");
        r.state = Some(TickState {
            state_key: "orphans=1".into(),
            summary: "healthy".into(),
            urgent: false,
        });
        pipeline.ingest(&[r.clone()]);
        let lease = pipeline.lease(100, 64).unwrap();
        pipeline.acknowledge(&lease);
        for n in 1..12 {
            r.logs = vec![format!("arbitrary log {n} total={n}")];
            pipeline.ingest(&[r.clone()]);
            assert!(pipeline.lease(100, 64).is_none());
        }
        pipeline.ingest(&[r.clone()]);
        let lease = pipeline.lease(100, 64).unwrap();
        assert_eq!(lease.items[0].repeat_count, 12);
        pipeline.acknowledge(&lease);
        r.state.as_mut().unwrap().state_key = "orphans=2".into();
        pipeline.ingest(&[r.clone()]);
        let lease = pipeline.lease(100, 64).unwrap();
        assert_eq!(lease.items[0].priority, ObservationPriority::Notable);
        pipeline.acknowledge(&lease);
        r.queued_sparks = 1;
        pipeline.ingest(&[r.clone(), r.clone()]);
        let lease = pipeline.lease(100, 64).unwrap();
        assert_eq!(lease.items.len(), 2);
        pipeline.acknowledge(&lease);
        r.queued_sparks = 0;
        r.error = Some("write failed".into());
        pipeline.ingest(&[r.clone(), r.clone(), r.clone()]);
        let lease = pipeline.lease(100, 64).unwrap();
        assert_eq!(lease.items.len(), 3);
        assert!(
            lease
                .items
                .iter()
                .all(|e| e.priority == ObservationPriority::Urgent)
        );
        pipeline.acknowledge(&lease);
        r.error = None;
        r.state.as_mut().unwrap().urgent = true;
        pipeline.ingest(&[r.clone(), r]);
        assert_eq!(pipeline.lease(100, 64).unwrap().items.len(), 2);
    }

    #[test]
    fn legacy_snapshot_timestamp_is_volatile_but_orphan_count_is_not() {
        let entity = Entity::from_raw_u32(7).unwrap();
        let a = report(
            entity,
            "snapshot=archetype-1789528662 total=13376 orphans=1",
        );
        let b = report(
            entity,
            "snapshot=archetype-1789529299 total=14856 orphans=1",
        );
        let c = report(
            entity,
            "snapshot=archetype-1789529299 total=14856 orphans=2",
        );
        assert_eq!(a.semantic_fingerprint(), b.semantic_fingerprint());
        assert_ne!(b.semantic_fingerprint(), c.semantic_fingerprint());
    }

    #[test]
    fn volatile_numbers_do_not_create_semantic_changes() {
        let entity = Entity::from_raw_u32(7).unwrap();
        let mut pipeline = TickObservationPipeline::<16>::default();
        pipeline.ingest(&[report(entity, "wave=8 total=6993")]);
        let first = pipeline.lease(100, 8).expect("baseline event");
        pipeline.acknowledge(&first);

        pipeline.ingest(&[report(entity, "wave=9 total=7004")]);
        assert!(pipeline.lease(100, 8).is_none());
    }

    #[test]
    fn semantic_transition_is_enqueued() {
        let entity = Entity::from_raw_u32(8).unwrap();
        let mut pipeline = TickObservationPipeline::<16>::default();
        pipeline.ingest(&[report(entity, "intent=observe-steady quiet_secs=20")]);
        let first = pipeline.lease(100, 8).unwrap();
        pipeline.acknowledge(&first);

        pipeline.ingest(&[report(entity, "intent=notice-slowing quiet_secs=300")]);
        let changed = pipeline.lease(100, 8).expect("semantic transition");
        assert_eq!(changed.items[0].priority, ObservationPriority::Notable);
    }

    #[test]
    fn operational_numbers_and_error_codes_are_not_suppressed() {
        let entity = Entity::from_raw_u32(8).unwrap();
        let mut pipeline = TickObservationPipeline::<16>::default();
        pipeline.ingest(&[report(entity, "pending=1 failures=0")]);
        let lease = pipeline.lease(100, 8).unwrap();
        pipeline.acknowledge(&lease);
        pipeline.ingest(&[report(entity, "pending=10 failures=3")]);
        assert_eq!(
            pipeline.lease(100, 8).unwrap().items[0].priority,
            ObservationPriority::Notable
        );
    }

    #[test]
    fn failures_retry_after_backoff_and_acknowledge_by_lease() {
        let entity = Entity::from_raw_u32(9).unwrap();
        let mut pipeline = TickObservationPipeline::<16>::default();
        let mut failed = report(entity, "run failed");
        failed.error = Some("Function not found: take_head".into());
        pipeline.ingest(&[failed]);

        let lease = pipeline.lease(100, 8).expect("urgent event");
        pipeline.defer(100);
        assert!(pipeline.lease(399, 8).is_none());
        assert!(pipeline.lease(400, 8).is_some());
        pipeline.acknowledge(&lease);
        assert!(pipeline.lease(400, 8).is_none());
    }

    #[test]
    fn bounded_inbox_drops_oldest_event() {
        let mut inbox = BoundedInbox::<TickObservation, 1>::default();
        let entity = Entity::from_raw_u32(10).unwrap();
        let report = report(entity, "first");
        inbox.push(TickObservation::from_report(
            1,
            &report,
            ObservationPriority::Routine,
            1,
        ));
        inbox.push(TickObservation::from_report(
            2,
            &report,
            ObservationPriority::Routine,
            1,
        ));
        let lease = inbox.lease(0, 8).unwrap();
        assert_eq!(lease.items[0].sequence, 2);
        assert_eq!(inbox.dropped, 1);
    }
}
