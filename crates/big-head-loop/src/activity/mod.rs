//! Durable business progression, separate from Spark/model termination.
mod runner;
mod tools;

use bevy_ecs::{component::Component, entity::Entity, world::World};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    registry::ComponentActionRegistry,
    system_def::{SystemKind, SystemRegistry},
};

pub(crate) use tools::{definitions, dispatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft,
    Ready,
    Waiting,
    Running,
    Paused,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub component: String,
    pub key_pointer: String,
    pub key: Value,
    pub value_pointer: String,
    pub equals: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub system_id: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub wait_for: Vec<Check>,
    pub verify: Vec<Check>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub scope: String,
    pub goal: String,
    pub steps: Vec<Step>,
    pub acceptance: Vec<Check>,
    #[serde(default)]
    pub deadline_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub check: Check,
    pub matches: usize,
    pub actual: Option<Value>,
    pub passed: bool,
    pub observed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub step_id: String,
    pub tx_id: String,
    pub effect_key: String,
    pub system_hash: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub outcome: String,
    pub output: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct ActivityRun {
    pub run_id: String,
    pub plan: Plan,
    #[serde(default)]
    pub plan_history: Vec<Plan>,
    pub system_hashes: Vec<String>,
    pub status: Status,
    pub revision: u64,
    pub cursor: usize,
    pub uncertain_step: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub next_poll_epoch: u64,
    pub reason: String,
    pub records: Vec<StepRecord>,
    pub evidence: Vec<Evidence>,
}

impl ActivityRun {
    fn change(&mut self, status: Status, now: u64, reason: impl Into<String>) {
        self.status = status;
        self.updated_at = now;
        self.revision += 1;
        self.reason = reason.into();
    }
}

fn system_hash(systems: &SystemRegistry, id: &str) -> Result<String, String> {
    let definition = systems
        .get(id)
        .ok_or_else(|| format!("system '{id}' not found"))?;
    let SystemKind::RhaiScript { source } = &definition.kind else {
        return Err("activity v1 requires a registered Rhai system".into());
    };
    Ok(big_head_save::persistence::digest(source.as_bytes()))
}

fn validate(plan: &Plan, systems: &SystemRegistry) -> Result<Vec<String>, String> {
    if plan.id.trim().is_empty()
        || plan.scope.trim().is_empty()
        || plan.goal.trim().is_empty()
        || plan.id.len() > 128
        || plan.scope.len() > 256
        || plan.goal.len() > 4000
        || plan.steps.is_empty()
        || plan.steps.len() > 32
        || plan.acceptance.is_empty()
        || plan.acceptance.len() > 16
        || serde_json::to_vec(plan).map_err(|e| e.to_string())?.len() > 64 * 1024
    {
        return Err("activity requires bounded id/scope/goal, 1..32 steps and 1..16 acceptance checks; max 64 KiB".into());
    }
    let mut ids = std::collections::HashSet::new();
    for step in &plan.steps {
        if step.id.trim().is_empty()
            || step.id.len() > 128
            || !ids.insert(&step.id)
            || step.verify.is_empty()
            || step.verify.len() > 16
            || step.wait_for.len() > 16
            || !(step.args.is_null() || step.args.is_object())
        {
            return Err("step ids must be unique; verify requires 1..16 checks; wait_for <=16; args must be an object".into());
        }
    }
    for check in plan.acceptance.iter().chain(
        plan.steps
            .iter()
            .flat_map(|s| s.wait_for.iter().chain(&s.verify)),
    ) {
        if check.component.is_empty()
            || check.key.is_null()
            || !valid_pointer(&check.key_pointer)
            || !valid_pointer(&check.value_pointer)
        {
            return Err(
                "checks require a component, non-null stable business key and JSON pointers".into(),
            );
        }
    }
    plan.steps
        .iter()
        .map(|step| system_hash(systems, &step.system_id))
        .collect()
}

fn valid_pointer(pointer: &str) -> bool {
    if !pointer.starts_with('/') || pointer.len() > 512 {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

fn read_checks(
    world: &World,
    registry: &ComponentActionRegistry,
    checks: &[Check],
    now: u64,
) -> Vec<Evidence> {
    checks
        .iter()
        .map(|check| {
            let dynamic_id = registry.dynamic_store().get_id(&check.component);
            let values: Vec<_> = world
                .iter_entities()
                .filter_map(|entity| {
                    let value = if let Some(id) = dynamic_id {
                        crate::dynamic_component::read(world, entity.id(), id)
                    } else {
                        registry.serialize_rust_component(world, entity.id(), &check.component)
                    }?;
                    (value.pointer(&check.key_pointer) == Some(&check.key)).then_some(value)
                })
                .collect();
            let actual = if values.len() == 1 {
                values[0].pointer(&check.value_pointer).cloned()
            } else {
                None
            };
            let passed = values.len() == 1 && actual.as_ref() == Some(&check.equals);
            Evidence {
                check: check.clone(),
                matches: values.len(),
                actual,
                passed,
                observed_at: now,
            }
        })
        .collect()
}

fn all_pass(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|check| check.passed)
}

fn find(world: &mut World, id: &str) -> Result<Entity, String> {
    world
        .query::<(Entity, &ActivityRun)>()
        .iter(world)
        .find(|(_, run)| run.run_id == id)
        .map(|(e, _)| e)
        .ok_or_else(|| format!("activity '{id}' not found"))
}

pub(crate) fn recover(world: &mut World, now: u64) -> usize {
    let mut count = 0;
    for mut run in world.query::<&mut ActivityRun>().iter_mut(world) {
        if run.status == Status::Running {
            run.uncertain_step = true;
            run.change(
                Status::Blocked,
                now,
                "interrupted execution; effects unknown; recheck evidence, do not replay",
            );
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests;
