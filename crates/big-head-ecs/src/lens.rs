//! Persistable Lens applications and their most recent execution receipt.
use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct LensApplication {
    pub id: String,
    pub definition_json: String,
    pub last_run_json: String,
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensTrigger {
    BeforeReasoning,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LensBinding {
    pub id: String,
    pub lens_id: String,
    pub lens_version: String,
    pub agent: String,
    pub trigger: LensTrigger,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    /// JSON pointer into the source fact, containing Unix seconds.
    #[serde(default)]
    pub source_updated_at_pointer: Option<String>,
    #[serde(default)]
    pub stale_threshold_ms: Option<u64>,
    /// Zero disables stage caching; does not control source refresh or fetching.
    #[serde(default)]
    pub refresh_interval_ms: u64,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensStatus {
    Ready,
    Paused,
    Succeeded,
    Failed,
    BudgetSkipped,
}

/// Runtime bookkeeping, not agent-authored business conclusions.
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct LensInstance {
    pub binding_id: String,
    pub binding_revision: u64,
    pub status: LensStatus,
    pub attempts: u64,
    pub last_cycle_id: String,
    pub observed_at: u64,
    pub last_receipt_json: String,
    pub last_error: String,
    #[serde(default)]
    pub cache_hits: u64,
    #[serde(default)]
    pub last_cache_hit: bool,
}

impl LensInstance {
    pub fn new(binding: &LensBinding) -> Self {
        Self {
            binding_id: binding.id.clone(),
            binding_revision: binding.revision,
            status: if binding.enabled {
                LensStatus::Ready
            } else {
                LensStatus::Paused
            },
            attempts: 0,
            last_cycle_id: String::new(),
            observed_at: 0,
            last_receipt_json: String::new(),
            last_error: String::new(),
            cache_hits: 0,
            last_cache_hit: false,
        }
    }
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensFeedbackKind {
    Useful,
    Gap,
    Correction,
}

/// Agent-authored assessment; source linkage does not make the assessment true.
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct LensFeedback {
    pub id: String,
    pub lens_id: String,
    pub lens_version: String,
    pub kind: LensFeedbackKind,
    pub note: String,
    pub source_id: String,
    pub correlation_id: String,
    pub created_at: u64,
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LensCandidateStatus {
    Proposed,
    Passed,
    Failed,
    Adopted,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct LensCandidate {
    pub id: String,
    pub definition_json: String,
    pub base_id: Option<String>,
    pub base_version: Option<String>,
    pub reason: String,
    pub feedback_ids: Vec<String>,
    pub revision: u64,
    pub status: LensCandidateStatus,
    pub trial_json: String,
    pub adoption_reason: String,
}
