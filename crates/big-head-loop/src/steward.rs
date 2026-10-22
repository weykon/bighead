//! A result contract for the observation stage, checked against actual tool writes.

use std::collections::HashSet;

use bevy_ecs::{resource::Resource, world::World};
use big_head_ecs::{ActionRecord, ComponentActionRegistry};
use serde::{Deserialize, Serialize};

use crate::observation::TickObservation;

#[derive(Resource)]
pub(crate) struct ObserverContract(pub Vec<TickObservation>);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObserverResult {
    pub dispositions: Vec<Disposition>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Disposition {
    pub event: u64,
    pub disposition: DispositionKind,
    pub reason: String,
    pub evidence_action_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DispositionKind {
    Ignore,
    Record,
    Repair,
    Escalate,
}

pub(crate) fn validate(
    world: &mut World,
    registry: &ComponentActionRegistry,
    tx_id: &str,
    text: &str,
) -> Result<ObserverResult, String> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|text| text.trim().strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let result: ObserverResult = serde_json::from_str(text)
        .map_err(|e| format!("Expected {{\"dispositions\":[{{\"event\":1,\"disposition\":\"ignore\",\"reason\":\"...\",\"evidence_action_id\":null}}]}}: {e}"))?;
    let contract = world
        .get_resource::<ObserverContract>()
        .ok_or("No active observation lease")?;
    if result.dispositions.len() != contract.0.len() {
        return Err("Exactly one disposition is required for each leased event".into());
    }
    let observations = contract.0.clone();
    let actions: Vec<_> = world
        .query::<&ActionRecord>()
        .iter(world)
        .filter(|a| a.tx_id == tx_id && !a.is_error)
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    for disposition in &result.dispositions {
        let observation = observations
            .iter()
            .find(|e| e.sequence == disposition.event)
            .ok_or("Disposition references an event outside this lease")?;
        if !seen.insert(disposition.event) {
            return Err(format!(
                "event={}: duplicate disposition in this receipt",
                disposition.event
            ));
        }
        if disposition.reason.trim().is_empty() {
            return Err(format!(
                "event={}: reason is empty; provide a nonempty explanation",
                disposition.event
            ));
        }
        if disposition.disposition == DispositionKind::Ignore {
            continue;
        }
        let action = actions.iter().find(|a| Some(&a.action_id) == disposition.evidence_action_id.as_ref())
            .ok_or("record/repair/escalate requires evidence_action_id from a successful write in this batch")?;
        if action.before_json == action.after_json || action.after_json.is_none() {
            return Err("Evidence must contain an actual persisted change".into());
        }
        match disposition.disposition {
            DispositionKind::Repair => {
                if action.tool_name != "update_tick_action"
                    || action.entity.as_deref()
                        != Some(format!("{:?}", observation.entity).as_str())
                {
                    return Err("repair requires update_tick_action on the observed entity".into());
                }
                let current = world
                    .get::<big_head_ecs::TickAction>(observation.entity)
                    .ok_or("Repaired TickAction no longer exists")?;
                let after: serde_json::Value =
                    serde_json::from_str(action.after_json.as_deref().unwrap())
                        .map_err(|e| e.to_string())?;
                if after.get("script").and_then(|s| s.as_str()) != Some(&current.script) {
                    return Err("Repaired script changed after the cited action".into());
                }
            }
            DispositionKind::Record | DispositionKind::Escalate => {
                if !matches!(
                    action.tool_name.as_str(),
                    "spawn_entity" | "modify_component"
                ) {
                    return Err("record/escalate requires a component value write, not just a schema definition".into());
                }
                let input: serde_json::Value =
                    serde_json::from_str(&action.input_json).map_err(|e| e.to_string())?;
                let components: Vec<&str> = if action.tool_name == "spawn_entity" {
                    input
                        .get("components")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.get("name")?.as_str())
                        .collect()
                } else {
                    input
                        .get("component")
                        .and_then(|v| v.as_str())
                        .into_iter()
                        .collect()
                };
                if !components
                    .iter()
                    .any(|name| registry.dynamic_store().resolve_name(name).is_some())
                {
                    return Err("record must reference an agent-authored dynamic component".into());
                }
                if disposition.disposition == DispositionKind::Escalate
                    && !components.contains(&"RuntimeWorkItem")
                {
                    return Err("escalate requires a RuntimeWorkItem value write".into());
                }
                let after: serde_json::Value =
                    serde_json::from_str(action.after_json.as_deref().unwrap())
                        .map_err(|e| e.to_string())?;
                if crate::audit::write_after_value(
                    world,
                    registry,
                    &action.tool_name,
                    action.entity.as_deref(),
                    action.component.as_deref(),
                )
                .as_ref()
                    != Some(&after)
                {
                    return Err("Recorded value no longer matches its write receipt; cite the latest successful write".into());
                }
            }
            DispositionKind::Ignore => {}
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentRuntime, TickAction};

    fn setup() -> AgentRuntime {
        let mut rt = AgentRuntime::new_for_test();
        rt.world.spawn(TickAction {
            enabled: true,
            script: "log(\"healthy\");".into(),
            ..Default::default()
        });
        let reports = rt.evaluate_tick_actions();
        let mut pipeline = crate::observation::TickObservationPipeline::<16>::default();
        pipeline.ingest(&reports);
        rt.world
            .insert_resource(ObserverContract(pipeline.lease(u64::MAX, 8).unwrap().items));
        rt
    }

    #[test]
    fn receipt_errors_identify_event_and_exact_violation() {
        let mut rt = setup();
        for reason in ["", " ", "\n\t"] {
            let input = serde_json::json!({"dispositions":[{"event":1,"disposition":"ignore","reason":reason,"evidence_action_id":null}]}).to_string();
            let error = validate(&mut rt.world, &rt.component_registry, "tx", &input).unwrap_err();
            assert!(error.contains("event=1"));
            assert!(error.contains("reason is empty"));
        }
        let disposition = serde_json::json!({"event":1,"disposition":"ignore","reason":"完整处理理由".repeat(1000),"evidence_action_id":null});
        assert!(
            validate(
                &mut rt.world,
                &rt.component_registry,
                "tx",
                &serde_json::json!({"dispositions":[disposition.clone()]}).to_string()
            )
            .is_ok()
        );
        let mut contract = rt.world.resource_mut::<ObserverContract>();
        let mut second = contract.0[0].clone();
        second.sequence = 2;
        contract.0.push(second);
        let error = validate(
            &mut rt.world,
            &rt.component_registry,
            "tx",
            &serde_json::json!({"dispositions":[disposition.clone(),disposition]}).to_string(),
        )
        .unwrap_err();
        assert_eq!(error, "event=1: duplicate disposition in this receipt");
    }

    #[test]
    fn ignore_is_valid_without_business_writes_but_must_cover_the_lease() {
        let mut rt = setup();
        let input = r#"{"dispositions":[{"event":1,"disposition":"ignore","reason":"Healthy heartbeat","evidence_action_id":null}]}"#;
        assert!(validate(&mut rt.world, &rt.component_registry, "tx", input).is_ok());
        assert!(
            validate(
                &mut rt.world,
                &rt.component_registry,
                "tx",
                "Everything is healthy"
            )
            .is_err()
        );
        assert!(
            validate(
                &mut rt.world,
                &rt.component_registry,
                "tx",
                r#"{"dispositions":[]}"#
            )
            .is_err()
        );
        assert!(
            validate(
                &mut rt.world,
                &rt.component_registry,
                "tx",
                &input.replace("\"event\":1", "\"event\":2")
            )
            .is_err()
        );
    }

    #[test]
    fn prose_and_multiple_documents_are_not_silently_acknowledged() {
        let mut rt = setup();
        let input = r#"{"dispositions":[{"event":1,"disposition":"ignore","reason":"Healthy","evidence_action_id":null}]}"#;
        for text in [
            format!("Done. {input}"),
            format!("{input}\n{input}"),
            "".into(),
        ] {
            assert!(validate(&mut rt.world, &rt.component_registry, "tx", &text).is_err());
        }
        assert!(
            validate(
                &mut rt.world,
                &rt.component_registry,
                "tx",
                &format!("```json\n{input}\n```")
            )
            .is_ok()
        );
    }

    #[test]
    fn claims_without_successful_write_receipts_are_rejected() {
        let mut rt = setup();
        for disposition in ["record", "repair", "escalate"] {
            let input = serde_json::json!({"dispositions":[{"event":1,"disposition":disposition,"reason":"Claimed work","evidence_action_id":"fabricated"}]}).to_string();
            assert!(
                validate(&mut rt.world, &rt.component_registry, "tx", &input)
                    .unwrap_err()
                    .contains("successful write")
            );
        }
    }
}
