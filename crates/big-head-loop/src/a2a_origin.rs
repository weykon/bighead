//! Runtime-owned provenance for an inbound A2A Spark.

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct A2aOrigin {
    /// The bridge is local but unauthenticated; this is a claim, not a verified identity.
    pub claimed_from: String,
    pub conversation_id: String,
    pub message_id: String,
    pub can_observe: bool,
    pub can_plan: bool,
    pub can_mutate: bool,
    pub can_execute: bool,
    pub can_forward: bool,
}

pub fn same_conversation(current: Option<&A2aOrigin>, previous: Option<&A2aOrigin>) -> bool {
    match (current, previous) {
        (Some(current), Some(previous)) => {
            current.conversation_id == previous.conversation_id
                && current.claimed_from == previous.claimed_from
        }
        (None, None) => true,
        _ => false,
    }
}

pub fn read_only_tools() -> Vec<String> {
    [
        "get_world_overview",
        "inspect_runtime",
        "inspect_activity",
        "inspect_transaction",
        "inspect_ecs_capabilities",
        "inspect_world_shape",
        "inspect_shard_retention",
        "inspect_shards",
        "get_shards",
        "get_shard_content",
        "inspect_lens_apps",
        "inspect_lens_bindings",
        "inspect_lens_learning",
        "inspect_interpretations",
        "inspect_interpretation_methods",
        "describe_lens_contract",
        "discover_lenses",
        "query_entities",
        "query_ecs",
        "get_entity",
        "get_component",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(from: &str, conversation: &str) -> A2aOrigin {
        A2aOrigin {
            claimed_from: from.into(),
            conversation_id: conversation.into(),
            message_id: "message".into(),
            can_observe: true,
            can_plan: true,
            can_mutate: false,
            can_execute: false,
            can_forward: false,
        }
    }

    #[test]
    fn conversation_history_never_inherits_local_user_or_other_peer() {
        let codex = origin("codex", "thread-1");
        assert!(same_conversation(
            Some(&codex),
            Some(&origin("codex", "thread-1"))
        ));
        assert!(!same_conversation(
            Some(&codex),
            Some(&origin("weykon", "thread-1"))
        ));
        assert!(!same_conversation(
            Some(&codex),
            Some(&origin("codex", "thread-2"))
        ));
        assert!(!same_conversation(Some(&codex), None));
        assert!(same_conversation(None, None));
    }

    #[test]
    fn a2a_observation_catalog_has_no_execution_or_write_tools() {
        let allowed = read_only_tools();
        assert!(allowed.contains(&"inspect_runtime".to_string()));
        for forbidden in [
            "execute_cli",
            "spawn_entity",
            "record_fact",
            "push_spark",
            "search",
        ] {
            assert!(!allowed.contains(&forbidden.to_string()));
        }
    }
}
