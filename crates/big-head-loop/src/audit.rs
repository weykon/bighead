//! Action log / transaction support.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use big_head_ecs::{ActionRecord, TransactionRecord, dynamic_component, resolve_entity};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::registry::ComponentActionRegistry;

pub(crate) const ACTION_LOG_PATH: &str = "outputs/action_log.jsonl";
const DEFAULT_ACTION_RETENTION_DAYS: u64 = 7;
const DEFAULT_TRANSACTION_RETENTION_DAYS: u64 = 90;

#[derive(Debug, Clone)]
pub(crate) struct TransactionContext {
    pub tx_id: String,
    pub entity: Entity,
}

#[derive(Debug, Clone)]
pub(crate) struct ActionRecordDraft {
    pub action_id: String,
    pub tx_id: String,
    pub wave: u32,
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: String,
    pub entity: Option<String>,
    pub component: Option<String>,
    pub input: Value,
    pub output_summary: String,
    pub is_error: bool,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ActionLogLine {
    TxStarted { record: TransactionRecord },
    TxCommitted { record: TransactionRecord },
    TxFailed { record: TransactionRecord },
    TxSuspended { record: TransactionRecord },
    TxUpdated { record: TransactionRecord },
    ToolExecution { record: crate::ToolExecution },
    Action { record: ActionLogRecord },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLogRecord {
    pub action_id: String,
    pub tx_id: String,
    pub wave: u32,
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: String,
    pub entity: Option<String>,
    pub component: Option<String>,
    pub input: Value,
    pub output_summary: String,
    pub is_error: bool,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct AuditTrail {
    pub tool_executions: Vec<crate::ToolExecution>,
    pub transactions: Vec<TransactionRecord>,
    pub actions: Vec<ActionLogRecord>,
}

#[derive(Debug, Clone)]
pub struct TransactionAuditSummary {
    pub tx_id: String,
    pub status: String,
    pub source: String,
    pub session_id: Option<String>,
    pub action_count: usize,
    pub failed_actions: usize,
    pub tool_names: Vec<String>,
    pub modified_targets: Vec<String>,
    pub projected_targets: Vec<String>,
}

pub(crate) fn begin_transaction(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    source: &str,
    spark_entities: &[Entity],
    agent_entity: Option<Entity>,
    session_id: Option<String>,
) -> TransactionContext {
    let tx_id = format!("tx_{}", Uuid::new_v4());
    let record = TransactionRecord {
        tx_id: tx_id.clone(),
        source: source.into(),
        started_at: now_secs(),
        spark_entities: spark_entities.iter().map(entity_label).collect(),
        agent_entity: agent_entity.map(|e| entity_label(&e)),
        session_id,
        ..Default::default()
    };
    let entity = world.spawn(record.clone()).id();
    let _ = registry;
    append_jsonl(ActionLogLine::TxStarted { record });
    TransactionContext { tx_id, entity }
}

pub(crate) fn commit_transaction(
    world: &mut World,
    tx: &TransactionContext,
    wave_end: Option<u32>,
    summary: String,
) {
    let Some(record) = update_tx_record(world, tx.entity, |record| {
        record.status = "committed".into();
        record.committed_at = Some(now_secs());
        record.wave_end = wave_end;
        record.summary = summary;
    }) else {
        return;
    };
    append_jsonl(ActionLogLine::TxCommitted { record });
}

pub(crate) fn fail_transaction(
    world: &mut World,
    tx: &TransactionContext,
    wave_end: Option<u32>,
    error: String,
) {
    let Some(record) = update_tx_record(world, tx.entity, |record| {
        record.status = "failed".into();
        record.failed_at = Some(now_secs());
        record.wave_end = wave_end;
        record.error = Some(error.clone());
        record.summary = error;
    }) else {
        return;
    };
    append_jsonl(ActionLogLine::TxFailed { record });
}

pub(crate) fn suspend_transaction(world: &mut World, tx: &TransactionContext, wave: usize, summary: String) {
    if let Some(record) = update_tx_record(world, tx.entity, |record| {
        record.status = "awaiting_input".into();
        record.wave_end = Some(wave as u32);
        record.summary = summary;
    }) { append_jsonl(ActionLogLine::TxSuspended { record }); }
}

pub(crate) fn record_tool_execution(trace: &crate::ToolExecution) {
    append_jsonl(ActionLogLine::ToolExecution { record: trace.clone() });
}

pub(crate) fn resolve_waiting_transaction(world: &mut World, tx_id: &str, answered: bool) {
    let record = world.query::<&mut TransactionRecord>().iter_mut(world)
        .find(|record| record.tx_id == tx_id && record.status == "awaiting_input")
        .map(|mut record| {
            record.status = if answered { "input_received" } else { "cancelled" }.into();
            record.clone()
        });
    if let Some(record) = record { append_jsonl(ActionLogLine::TxUpdated { record }); }
}

pub(crate) fn record_action(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    draft: ActionRecordDraft,
) {
    let ecs_record = ActionRecord {
        action_id: draft.action_id.clone(),
        tx_id: draft.tx_id.clone(),
        wave: draft.wave,
        tool_call_id: draft.tool_call_id.clone(),
        tool_name: draft.tool_name.clone(),
        kind: draft.kind.clone(),
        entity: draft.entity.clone(),
        component: draft.component.clone(),
        input_json: compact_json(&draft.input),
        output_summary: draft.output_summary.clone(),
        is_error: draft.is_error,
        before_json: draft.before.as_ref().map(compact_json),
        after_json: draft.after.as_ref().map(compact_json),
        created_at: draft.created_at,
    };
    world.spawn(ecs_record);

    let log_record = ActionLogRecord {
        action_id: draft.action_id,
        tx_id: draft.tx_id,
        wave: draft.wave,
        tool_call_id: draft.tool_call_id,
        tool_name: draft.tool_name,
        kind: draft.kind,
        entity: draft.entity,
        component: draft.component,
        input: draft.input,
        output_summary: draft.output_summary,
        is_error: draft.is_error,
        before: draft.before,
        after: draft.after,
        created_at: draft.created_at,
    };
    let _ = registry;
    append_jsonl(ActionLogLine::Action { record: log_record });
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AuditRetentionReport {
    pub actions_removed: usize,
    pub transactions_removed: usize,
}

fn retention_secs(variable: &str, default_days: u64) -> Option<u64> {
    match std::env::var(variable) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .and_then(|days| (days > 0).then_some(days.saturating_mul(86_400))),
        Err(_) => Some(default_days.saturating_mul(86_400)),
    }
}

/// Keep the append-only JSONL as the full audit source while bounding the
/// recoverable ECS working set. TransactionRecord already is the durable
/// per-transaction summary, so it is retained much longer than individual
/// ActionRecord detail.
pub(crate) fn compact_ecs_audit(world: &mut World, now: u64) -> AuditRetentionReport {
    compact_ecs_audit_with(
        world,
        now,
        retention_secs(
            "BIGHEAD_ACTION_RETENTION_DAYS",
            DEFAULT_ACTION_RETENTION_DAYS,
        ),
        retention_secs(
            "BIGHEAD_TRANSACTION_RETENTION_DAYS",
            DEFAULT_TRANSACTION_RETENTION_DAYS,
        ),
    )
}

fn compact_ecs_audit_with(
    world: &mut World,
    now: u64,
    action_retention_secs: Option<u64>,
    transaction_retention_secs: Option<u64>,
) -> AuditRetentionReport {
    let action_cutoff = action_retention_secs.map(|seconds| now.saturating_sub(seconds));
    let transaction_cutoff = transaction_retention_secs.map(|seconds| now.saturating_sub(seconds));

    let actions = {
        let mut query = world.query::<(Entity, &ActionRecord)>();
        query
            .iter(world)
            .filter_map(|(entity, record)| {
                action_cutoff
                    .is_some_and(|cutoff| record.created_at > 0 && record.created_at < cutoff)
                    .then_some(entity)
            })
            .collect::<Vec<_>>()
    };
    let transactions = {
        let mut query = world.query::<(Entity, &TransactionRecord)>();
        query
            .iter(world)
            .filter_map(|(entity, record)| {
                let finished_at = record.committed_at.or(record.failed_at)?;
                transaction_cutoff
                    .is_some_and(|cutoff| finished_at > 0 && finished_at < cutoff)
                    .then_some(entity)
            })
            .collect::<Vec<_>>()
    };
    let report = AuditRetentionReport {
        actions_removed: actions.len(),
        transactions_removed: transactions.len(),
    };
    for entity in actions.into_iter().chain(transactions) {
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
    }
    report
}

/// Transactions cannot continue across process sessions. Mark restored open
/// records explicitly instead of leaving them looking active forever.
pub(crate) fn abandon_restored_transactions(world: &mut World, current_session_id: &str) -> usize {
    let recovered = mark_restored_transactions_abandoned(world, current_session_id, now_secs());
    for record in &recovered {
        append_jsonl(ActionLogLine::TxFailed {
            record: record.clone(),
        });
    }
    recovered.len()
}

fn mark_restored_transactions_abandoned(
    world: &mut World,
    current_session_id: &str,
    now: u64,
) -> Vec<TransactionRecord> {
    let mut query = world.query::<&mut TransactionRecord>();
    query
        .iter_mut(world)
        .filter_map(|mut record| {
            if record.status != "open" || record.session_id.as_deref() == Some(current_session_id) {
                return None;
            }
            record.status = "abandoned_after_restart".into();
            record.failed_at = Some(now);
            record.error = Some("process ended before transaction completion".into());
            record.summary =
                "Abandoned after runtime restart; consult action_log.jsonl for partial effects"
                    .into();
            Some(record.clone())
        })
        .collect()
}

pub fn read_audit_trail(path: &Path) -> Result<AuditTrail, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {:?}: {e}", path))?;
    let reader = std::io::BufReader::new(file);
    let mut trail = AuditTrail::default();

    for (idx, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read {:?}: {e}", path))?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: ActionLogLine = serde_json::from_str(&line)
            .map_err(|e| format!("parse {:?} line {}: {e}", path, idx + 1))?;
        match parsed {
            ActionLogLine::TxStarted { record }
            | ActionLogLine::TxCommitted { record }
            | ActionLogLine::TxFailed { record }
            | ActionLogLine::TxUpdated { record }
            | ActionLogLine::TxSuspended { record } => trail.transactions.push(record),
            ActionLogLine::Action { record } => trail.actions.push(record),
            ActionLogLine::ToolExecution { record } => trail.tool_executions.push(record),
        }
    }

    Ok(trail)
}

impl AuditTrail {
    pub fn latest_transaction(&self) -> Option<&TransactionRecord> {
        self.transactions.iter().max_by_key(|record| {
            record
                .committed_at
                .or(record.failed_at)
                .unwrap_or(record.started_at)
        })
    }

    pub fn actions_for_tx(&self, tx_id: &str) -> Vec<&ActionLogRecord> {
        self.actions
            .iter()
            .filter(|record| record.tx_id == tx_id)
            .collect()
    }

    pub fn summarize_tx(&self, tx_id: &str) -> Option<TransactionAuditSummary> {
        let tx = self
            .transactions
            .iter()
            .rev()
            .find(|record| record.tx_id == tx_id)?;
        let actions = self.actions_for_tx(tx_id);
        let mut tool_names = Vec::new();
        let mut modified_targets = Vec::new();
        let mut projected_targets = Vec::new();

        for action in &actions {
            if !tool_names.contains(&action.tool_name) {
                tool_names.push(action.tool_name.clone());
            }
            if let Some(target) = action_target(action) {
                if action.kind == "ProjectContext" {
                    projected_targets.push(target);
                } else {
                    modified_targets.push(target);
                }
            }
        }

        Some(TransactionAuditSummary {
            tx_id: tx.tx_id.clone(),
            status: tx.status.clone(),
            source: tx.source.clone(),
            session_id: tx.session_id.clone(),
            action_count: actions.len(),
            failed_actions: actions.iter().filter(|record| record.is_error).count(),
            tool_names,
            modified_targets,
            projected_targets,
        })
    }
}

pub(crate) fn tool_action_kind(tool_name: &str) -> &'static str {
    match tool_name {
        "spawn_entity" => "SpawnEntity",
        "modify_component" => "ModifyComponent",
        "update_tick_action" => "UpdateTickAction",
        "remove_component" => "RemoveComponent",
        "despawn_entity" => "DespawnEntity",
        "define_component" => "DefineComponent",
        "undefine_component" => "UndefineComponent",
        "define_system" => "DefineSystem",
        "update_system" => "UpdateSystem",
        "remove_system" => "RemoveSystem",
        "define_tool" => "DefineTool",
        "run_system" => "RunSystem",
        "push_spark" => "PushSpark",
        "search" => "Search",
        "fetch_url" => "FetchUrl",
        "inspect_component_view" => "InspectComponentView",
        "render_view" | "render_mind_view" => "RenderView",
        "render_status" => "RenderStatus",
        "shard_content" | "get_shards" | "get_shard_content" => "ShardContent",
        _ => "ToolCall",
    }
}

pub(crate) fn write_before_after_target(
    world: &World,
    registry: &ComponentActionRegistry,
    tool_name: &str,
    input: &Value,
) -> Option<(Option<String>, Option<String>, Option<Value>)> {
    match tool_name {
        "modify_component" | "remove_component" => {
            let entity_id = input.get("entity_id")?.as_u64()?;
            let component = input.get("component")?.as_str()?.to_string();
            let entity = resolve_entity(world, entity_id).ok()?;
            let before = component_value_json(world, registry, entity, &component);
            Some((Some(entity_label(&entity)), Some(component), before))
        }
        "update_tick_action" => {
            let entity_id = input.get("entity_id")?.as_u64()?;
            let entity = resolve_entity(world, entity_id).ok()?;
            let component = "TickAction".to_string();
            let before = component_value_json(world, registry, entity, &component);
            Some((Some(entity_label(&entity)), Some(component), before))
        }
        "despawn_entity" => {
            let entity_id = input.get("entity_id")?.as_u64()?;
            let entity = resolve_entity(world, entity_id).ok()?;
            let before = entity_snapshot_json(world, registry, entity);
            Some((Some(entity_label(&entity)), None, before))
        }
        "define_component" => {
            let component = input.get("name")?.as_str()?.to_string();
            let before = dynamic_component_schema_json(registry, &component);
            Some((None, Some(component), before))
        }
        "undefine_component" => {
            let component = input.get("name")?.as_str()?.to_string();
            let before = dynamic_component_schema_json(registry, &component);
            Some((None, Some(component), before))
        }
        "render_view" | "render_mind_view" => {
            let component = "ViewState".to_string();
            let entity = first_entity_with_component(world, registry, &component)?;
            let before = component_value_json(world, registry, entity, &component);
            Some((Some(entity_label(&entity)), Some(component), before))
        }
        _ => None,
    }
}

pub(crate) fn write_after_value(
    world: &World,
    registry: &ComponentActionRegistry,
    tool_name: &str,
    entity: Option<&str>,
    component: Option<&str>,
) -> Option<Value> {
    match tool_name {
        "spawn_entity" => {
            let entity = parse_entity_label(world, entity?)?;
            entity_snapshot_json(world, registry, entity)
        }
        "modify_component" | "update_tick_action" => {
            let entity = parse_entity_label(world, entity?)?;
            component_value_json(world, registry, entity, component?)
        }
        "push_spark" => {
            let entity = parse_entity_label(world, entity?)?;
            entity_snapshot_json(world, registry, entity)
        }
        "render_view" | "render_mind_view" => {
            let entity = parse_entity_label(world, entity?)?;
            component_value_json(world, registry, entity, component?)
        }
        "define_component" => dynamic_component_schema_json(registry, component?),
        "remove_component" | "despawn_entity" | "undefine_component" => None,
        _ => None,
    }
}

pub(crate) fn first_entity_with_component(
    world: &World,
    registry: &ComponentActionRegistry,
    component: &str,
) -> Option<Entity> {
    let id = registry.lookup_component_id(world, component)?;
    world
        .archetypes()
        .iter()
        .find(|archetype| archetype.components().contains(&id))
        .and_then(|archetype| archetype.entities().first().map(|entity| entity.id()))
}

pub(crate) fn entity_from_tool_output(tool_name: &str, output: &str) -> Option<String> {
    match tool_name {
        "spawn_entity" => output
            .strip_prefix("Spawned ")?
            .split_whitespace()
            .next()
            .map(str::to_string),
        _ => None,
    }
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn short_output(output: &str) -> String {
    output.chars().take(500).collect()
}

pub(crate) fn new_action_id() -> String {
    format!("act_{}", Uuid::new_v4())
}

fn update_tx_record(
    world: &mut World,
    entity: Entity,
    update: impl FnOnce(&mut TransactionRecord),
) -> Option<TransactionRecord> {
    let mut entity_mut = world.get_entity_mut(entity).ok()?;
    let mut record = entity_mut.get_mut::<TransactionRecord>()?;
    update(&mut record);
    Some(record.clone())
}

fn component_value_json(
    world: &World,
    registry: &ComponentActionRegistry,
    entity: Entity,
    component: &str,
) -> Option<Value> {
    if let Some(value) = registry.serialize_rust_component(world, entity, component) {
        return Some(value);
    }
    let id = registry.lookup_component_id(world, component)?;
    if registry.dynamic_store().is_dynamic(id) {
        return dynamic_component::read(world, entity, id);
    }
    None
}

fn entity_snapshot_json(
    world: &World,
    registry: &ComponentActionRegistry,
    entity: Entity,
) -> Option<Value> {
    let archetype = world
        .archetypes()
        .iter()
        .find(|archetype| archetype.entities().iter().any(|e| e.id() == entity))?;
    let components = archetype
        .components()
        .iter()
        .filter_map(|component_id| {
            let info = world.components().get_info(*component_id)?;
            let name = info.name().to_string();
            let value = component_value_json(world, registry, entity, &name)?;
            Some(json!({ "name": name, "value": value }))
        })
        .collect::<Vec<_>>();
    Some(json!({
        "entity": entity_label(&entity),
        "components": components
    }))
}

fn dynamic_component_schema_json(
    registry: &ComponentActionRegistry,
    component: &str,
) -> Option<Value> {
    let resolved = registry.dynamic_store().resolve_name(component)?;
    let schema = registry
        .dynamic_store()
        .iter()
        .find_map(|(name, schema)| (name == resolved).then_some(schema))?;
    serde_json::to_value(schema).ok()
}

fn parse_entity_label(world: &World, label: &str) -> Option<Entity> {
    let index = label
        .split_once('v')
        .map(|(idx, _)| idx)
        .unwrap_or(label)
        .parse::<u64>()
        .ok()?;
    resolve_entity(world, index).ok()
}

fn entity_label(entity: &Entity) -> String {
    format!("{entity:?}")
}

fn action_target(action: &ActionLogRecord) -> Option<String> {
    match (&action.entity, &action.component) {
        (Some(entity), Some(component)) => Some(format!("{entity}:{component}")),
        (Some(entity), None) => Some(entity.clone()),
        (None, Some(component)) => Some(component.clone()),
        (None, None) => None,
    }
}

fn append_jsonl(line: ActionLogLine) {
    if let Err(e) = append_jsonl_result(Path::new(ACTION_LOG_PATH), &line) {
        eprintln!("[audit] failed to append action log: {e}");
    }
}

fn append_jsonl_result(path: &Path, line: &ActionLogLine) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {:?}: {e}", parent))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {:?}: {e}", path))?;
    let raw = serde_json::to_string(line).map_err(|e| format!("serialize jsonl: {e}"))?;
    writeln!(file, "{raw}").map_err(|e| format!("write {:?}: {e}", path))?;
    Ok(())
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    #[derive(Component, Debug, Clone, Serialize, Deserialize)]
    struct AuditTestComponent {
        value: i32,
    }

    #[test]
    fn transaction_lifecycle_records_ecs_components() {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        registry.register_rust_component::<TransactionRecord>("TransactionRecord");
        registry.register_rust_component::<ActionRecord>("ActionRecord");

        let tx = begin_transaction(
            &mut world,
            &mut registry,
            "test",
            &[],
            None,
            Some("session_test".into()),
        );
        record_action(
            &mut world,
            &mut registry,
            ActionRecordDraft {
                action_id: "act_test".into(),
                tx_id: tx.tx_id.clone(),
                wave: 1,
                tool_call_id: "toolu_1".into(),
                tool_name: "get_world_overview".into(),
                kind: "ToolCall".into(),
                entity: None,
                component: None,
                input: json!({}),
                output_summary: "ok".into(),
                is_error: false,
                before: None,
                after: None,
                created_at: now_secs(),
            },
        );
        commit_transaction(&mut world, &tx, Some(1), "done".into());

        let record = world.get::<TransactionRecord>(tx.entity).unwrap();
        assert_eq!(record.status, "committed");
        assert_eq!(record.summary, "done");
        assert_eq!(record.session_id.as_deref(), Some("session_test"));

        let mut query = world.query::<&ActionRecord>();
        let actions = query.iter(&world).collect::<Vec<_>>();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tx_id, tx.tx_id);
    }

    #[test]
    fn component_value_json_reads_before_after() {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        registry.register_rust_component::<AuditTestComponent>("AuditTestComponent");
        let entity = world.spawn(AuditTestComponent { value: 1 }).id();

        let before = component_value_json(&world, &registry, entity, "AuditTestComponent").unwrap();
        assert_eq!(before["value"], 1);

        world
            .entity_mut(entity)
            .insert(AuditTestComponent { value: 2 });
        let after = component_value_json(&world, &registry, entity, "AuditTestComponent").unwrap();
        assert_eq!(after["value"], 2);
    }

    #[test]
    fn append_jsonl_writes_machine_readable_line() {
        let path = PathBuf::from(format!(
            "/tmp/big_head_action_log_test_{}.jsonl",
            Uuid::new_v4()
        ));
        let record = TransactionRecord {
            tx_id: "tx_test".into(),
            source: "test".into(),
            started_at: 1,
            ..Default::default()
        };
        append_jsonl_result(&path, &ActionLogLine::TxStarted { record }).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let line: Value = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(line["type"], "tx_started");
        assert_eq!(line["record"]["tx_id"], "tx_test");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_audit_trail_summarizes_jsonl_without_ecs() {
        let path = PathBuf::from(format!(
            "/tmp/big_head_action_log_summary_test_{}.jsonl",
            Uuid::new_v4()
        ));
        let tx = TransactionRecord {
            tx_id: "tx_summary".into(),
            source: "spark_cycle".into(),
            status: "open".into(),
            started_at: 10,
            spark_entities: vec!["1v0".into()],
            session_id: Some("session_summary".into()),
            ..Default::default()
        };
        append_jsonl_result(&path, &ActionLogLine::TxStarted { record: tx }).unwrap();
        append_jsonl_result(
            &path,
            &ActionLogLine::Action {
                record: ActionLogRecord {
                    action_id: "act_modify".into(),
                    tx_id: "tx_summary".into(),
                    wave: 1,
                    tool_call_id: "toolu_modify".into(),
                    tool_name: "modify_component".into(),
                    kind: "ModifyComponent".into(),
                    entity: Some("2v0".into()),
                    component: Some("Task".into()),
                    input: json!({"entity_id": 2}),
                    output_summary: "modified".into(),
                    is_error: false,
                    before: Some(json!({"status": "todo"})),
                    after: Some(json!({"status": "done"})),
                    created_at: 11,
                },
            },
        )
        .unwrap();
        append_jsonl_result(
            &path,
            &ActionLogLine::Action {
                record: ActionLogRecord {
                    action_id: "act_project".into(),
                    tx_id: "tx_summary".into(),
                    wave: 1,
                    tool_call_id: "project_context:message:abc".into(),
                    tool_name: "context_projection".into(),
                    kind: "ProjectContext".into(),
                    entity: Some("3v0".into()),
                    component: Some("ShardData".into()),
                    input: json!({"source_kind": "message_text"}),
                    output_summary: "projected".into(),
                    is_error: true,
                    before: None,
                    after: Some(json!({"paradigm": "context:message:abc"})),
                    created_at: 12,
                },
            },
        )
        .unwrap();
        let committed = TransactionRecord {
            tx_id: "tx_summary".into(),
            source: "spark_cycle".into(),
            status: "committed".into(),
            started_at: 10,
            committed_at: Some(13),
            spark_entities: vec!["1v0".into()],
            session_id: Some("session_summary".into()),
            summary: "done".into(),
            ..Default::default()
        };
        append_jsonl_result(&path, &ActionLogLine::TxCommitted { record: committed }).unwrap();

        let trail = read_audit_trail(&path).unwrap();
        let latest = trail.latest_transaction().unwrap();
        assert_eq!(latest.tx_id, "tx_summary");
        assert_eq!(latest.status, "committed");

        let summary = trail.summarize_tx("tx_summary").unwrap();
        assert_eq!(summary.tx_id, "tx_summary");
        assert_eq!(summary.status, "committed");
        assert_eq!(summary.source, "spark_cycle");
        assert_eq!(summary.session_id.as_deref(), Some("session_summary"));
        assert_eq!(summary.action_count, 2);
        assert_eq!(summary.failed_actions, 1);
        assert_eq!(
            summary.tool_names,
            vec!["modify_component", "context_projection"]
        );
        assert_eq!(summary.modified_targets, vec!["2v0:Task"]);
        assert_eq!(summary.projected_targets, vec!["3v0:ShardData"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn tool_action_kind_covers_required_first_batch() {
        let cases = [
            ("unknown_tool", "ToolCall"),
            ("spawn_entity", "SpawnEntity"),
            ("modify_component", "ModifyComponent"),
            ("update_tick_action", "UpdateTickAction"),
            ("remove_component", "RemoveComponent"),
            ("despawn_entity", "DespawnEntity"),
            ("define_component", "DefineComponent"),
            ("undefine_component", "UndefineComponent"),
            ("define_system", "DefineSystem"),
            ("define_tool", "DefineTool"),
            ("run_system", "RunSystem"),
            ("push_spark", "PushSpark"),
            ("search", "Search"),
            ("fetch_url", "FetchUrl"),
            ("inspect_component_view", "InspectComponentView"),
            ("render_view", "RenderView"),
            ("render_mind_view", "RenderView"),
            ("render_status", "RenderStatus"),
        ];

        for (tool_name, expected_kind) in cases {
            assert_eq!(tool_action_kind(tool_name), expected_kind);
        }
    }

    #[test]
    fn checkpoint_retention_keeps_recent_actions_and_transaction_summaries_longer() {
        let mut world = World::new();
        world.spawn(ActionRecord {
            action_id: "old".into(),
            created_at: 10,
            ..Default::default()
        });
        world.spawn(ActionRecord {
            action_id: "recent".into(),
            created_at: 95,
            ..Default::default()
        });
        world.spawn(TransactionRecord {
            tx_id: "summary".into(),
            status: "committed".into(),
            committed_at: Some(10),
            ..Default::default()
        });

        let first = compact_ecs_audit_with(&mut world, 100, Some(20), Some(200));
        assert_eq!(
            first,
            AuditRetentionReport {
                actions_removed: 1,
                transactions_removed: 0,
            }
        );
        let actions = world
            .query::<&ActionRecord>()
            .iter(&world)
            .map(|record| record.action_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actions, vec!["recent"]);
        assert_eq!(world.query::<&TransactionRecord>().iter(&world).count(), 1);

        let second = compact_ecs_audit_with(&mut world, 300, Some(20), Some(200));
        assert_eq!(second.transactions_removed, 1);
    }

    #[test]
    fn restart_abandons_only_open_transactions_from_previous_sessions() {
        let mut world = World::new();
        world.spawn(TransactionRecord {
            tx_id: "interrupted".into(),
            status: "open".into(),
            session_id: Some("old-session".into()),
            ..Default::default()
        });
        world.spawn(TransactionRecord {
            tx_id: "current".into(),
            status: "open".into(),
            session_id: Some("new-session".into()),
            ..Default::default()
        });
        world.spawn(TransactionRecord {
            tx_id: "complete".into(),
            status: "committed".into(),
            session_id: Some("old-session".into()),
            ..Default::default()
        });

        let recovered = mark_restored_transactions_abandoned(&mut world, "new-session", 1234);
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].tx_id, "interrupted");
        assert_eq!(recovered[0].status, "abandoned_after_restart");
        assert_eq!(recovered[0].failed_at, Some(1234));

        let states = world
            .query::<&TransactionRecord>()
            .iter(&world)
            .map(|record| (record.tx_id.clone(), record.status.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(states["current"], "open");
        assert_eq!(states["complete"], "committed");
    }
}
