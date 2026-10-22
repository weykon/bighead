//! Exact transaction-scoped audit lookup; never joins actions by recency.

use bevy_ecs::{entity::Entity, world::World};
use big_head_ecs::{ActionRecord, TransactionRecord, execution::SparkDispatch};
use serde_json::{Value, json};

use crate::A2aOrigin;

pub(super) fn inspect(world: &mut World, input: &Value) -> Result<Value, String> {
    let tx_id = input
        .get("tx_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .ok_or("inspect_transaction requires tx_id")?;
    let canonical_id = if tx_id.starts_with("tx_") {
        tx_id.to_owned()
    } else {
        format!("tx_{tx_id}")
    };
    let tx = world
        .query::<&TransactionRecord>()
        .iter(world)
        .find(|record| record.tx_id == tx_id || record.tx_id == canonical_id)
        .cloned()
        .ok_or("unknown transaction id")?;
    let mut actions: Vec<_> = world
        .query::<&ActionRecord>()
        .iter(world)
        .filter(|action| action.tx_id == tx.tx_id)
        .cloned()
        .collect();
    actions.sort_by_key(|action| (action.wave, action.created_at, action.action_id.clone()));
    let action_count = actions.len();
    let actions: Vec<_> = actions
        .iter()
        .take(100)
        .map(|action| {
            json!({
                "action_id": action.action_id,
                "tx_id": action.tx_id,
                "wave": action.wave,
                "tool_name": action.tool_name,
                "is_error": action.is_error,
                "output_summary": action.output_summary.chars().take(1000).collect::<String>(),
            })
        })
        .collect();
    let spark_sources: Vec<_> = world
        .query::<(Entity, Option<&A2aOrigin>, Option<&SparkDispatch>)>()
        .iter(world)
        .filter(|(entity, _, _)| tx.spark_entities.contains(&format!("{entity:?}")))
        .map(|(entity, origin, dispatch)| {
            let kind = if origin.is_some() {
                "a2a_peer_claim"
            } else if dispatch.is_some_and(|dispatch| dispatch.interactive) {
                "interactive_user"
            } else {
                "background"
            };
            json!({
                "spark_entity": format!("{entity:?}"),
                "kind": kind,
                "claimed_from": origin.map(|origin| &origin.claimed_from),
                "conversation_id": origin.map(|origin| &origin.conversation_id),
                "message_id": origin.map(|origin| &origin.message_id),
            })
        })
        .collect();
    let mut executions: Vec<_> = world.query::<&crate::tool_tracking::ToolExecution>().iter(world)
        .filter(|trace| trace.tx_id == tx.tx_id).cloned().collect();
    executions.sort_by_key(|t| t.sequence);
    let execution_count = executions.len();
    executions.truncate(100);
    Ok(json!({
        "tool_executions": executions,
        "tool_execution_count": execution_count,
        "tool_executions_truncated": execution_count > 100,
        "transaction": {
            "tx_id": tx.tx_id,
            "source": tx.source,
            "status": tx.status,
            "wave_start": tx.wave_start,
            "wave_end": tx.wave_end,
            "spark_entities": tx.spark_entities,
            "started_at": tx.started_at,
            "committed_at": tx.committed_at,
            "failed_at": tx.failed_at,
            "summary_excerpt": tx.summary.chars().take(1000).collect::<String>(),
            "error": tx.error,
        },
        "spark_sources": spark_sources,
        "action_count": action_count,
        "actions_truncated": action_count > actions.len(),
        "actions": actions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_transaction_lookup_does_not_import_adjacent_actions() {
        let mut world = World::new();
        let spark = world.spawn_empty().id();
        world.entity_mut(spark).insert(A2aOrigin {
            claimed_from: "codex".into(),
            conversation_id: "one".into(),
            message_id: "m1".into(),
            can_observe: true,
            can_plan: true,
            can_mutate: false,
            can_execute: false,
            can_forward: false,
        });
        world.spawn(TransactionRecord {
            tx_id: "a2a-tx".into(),
            spark_entities: vec![format!("{spark:?}")],
            wave_end: Some(1),
            ..Default::default()
        });
        world.spawn(ActionRecord {
            tx_id: "other-tx".into(),
            tool_name: "execute_cli".into(),
            ..Default::default()
        });
        let result = inspect(&mut world, &json!({"tx_id":"a2a-tx"})).unwrap();
        assert_eq!(result["transaction"]["wave_end"], 1);
        assert_eq!(result["action_count"], 0);
        assert_eq!(result["spark_sources"][0]["kind"], "a2a_peer_claim");
        assert_eq!(result["spark_sources"][0]["claimed_from"], "codex");
    }

    #[test]
    fn accepts_missing_tx_prefix_but_returns_canonical_id() {
        let mut world = World::new();
        world.spawn(TransactionRecord {
            tx_id: "tx_example".into(),
            ..Default::default()
        });
        world.spawn(ActionRecord {
            tx_id: "tx_example".into(),
            tool_name: "inspect_runtime".into(),
            ..Default::default()
        });
        let result = inspect(&mut world, &json!({"tx_id":"example"})).unwrap();
        assert_eq!(result["transaction"]["tx_id"], "tx_example");
        assert_eq!(result["action_count"], 1);
    }
}
