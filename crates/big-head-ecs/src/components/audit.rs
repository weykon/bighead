//! Audit components for transaction and action history.

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// One agent/runtime transaction. A transaction groups the tool calls and ECS
/// mutations produced while processing a spark/cycle.
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TransactionRecord {
    pub tx_id: String,
    pub parent_tx_id: Option<String>,
    pub source: String,
    pub status: String,
    pub started_at: u64,
    pub committed_at: Option<u64>,
    pub failed_at: Option<u64>,
    pub spark_entities: Vec<String>,
    pub agent_entity: Option<String>,
    pub session_id: Option<String>,
    pub wave_start: u32,
    pub wave_end: Option<u32>,
    pub summary: String,
    pub error: Option<String>,
}

impl Default for TransactionRecord {
    fn default() -> Self {
        Self {
            tx_id: String::new(),
            parent_tx_id: None,
            source: "unknown".into(),
            status: "open".into(),
            started_at: 0,
            committed_at: None,
            failed_at: None,
            spark_entities: Vec::new(),
            agent_entity: None,
            session_id: None,
            wave_start: 0,
            wave_end: None,
            summary: String::new(),
            error: None,
        }
    }
}

/// One action inside a transaction. `input_json`, `before_json`, and
/// `after_json` are JSON strings so the component remains easy to serialize
/// through the existing ECS registry path.
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionRecord {
    pub action_id: String,
    pub tx_id: String,
    pub wave: u32,
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: String,
    pub entity: Option<String>,
    pub component: Option<String>,
    pub input_json: String,
    pub output_summary: String,
    pub is_error: bool,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub created_at: u64,
}

impl Default for ActionRecord {
    fn default() -> Self {
        Self {
            action_id: String::new(),
            tx_id: String::new(),
            wave: 0,
            tool_call_id: String::new(),
            tool_name: String::new(),
            kind: "ToolCall".into(),
            entity: None,
            component: None,
            input_json: "{}".into(),
            output_summary: String::new(),
            is_error: false,
            before_json: None,
            after_json: None,
            created_at: 0,
        }
    }
}
