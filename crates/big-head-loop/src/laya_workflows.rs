//! Fixed routing vocabularies, distinct from executable ECS tool names.

use big_head_ai::ToolDefinition;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "laya_assess".into(),
        description: "Shadow advice for four narrow workflows: anomaly triage, observation selection, existing behavior selection, and completion review. Fixed route labels are NOT executable tool names. Supply compact observed facts and explicit unknowns; see guide_laya. No actions, scheduling, completion claims, or success labels are created. These models are not ECS-trained.".into(),
        input_schema: json!({"type":"object","properties":{
            "scenario":{"type":"string","enum":["anomaly_triage","observation_selection","behavior_selection","completion_review"]},
            "state":{"type":"object","description":"Compact scenario facts, unknown values null; <=4096 bytes. See guide_laya for required evidence."}
        },"required":["scenario","state"],"additionalProperties":false}),
    }, ToolDefinition {
        name: "inspect_laya_shadow".into(),
        description: "Read local shadow observer counters, queue drops, output directory and pending incidents. No inference or ECS writes. Enabled does not mean inference succeeded; inspect counters and evidence.".into(),
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
    }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    scenario: String,
    state: serde_json::Map<String, Value>,
}

pub(crate) fn prepare(input: &Value) -> Result<(Value, HashSet<String>), String> {
    let r: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_vec(&r.state).unwrap().len() > 4096 {
        return Err("Scenario state exceeds 4096 bytes; use compact observations".into());
    }
    let (goal, options): (&str, &[(&str, &str)]) = match r.scenario.as_str() {
        "anomaly_triage" => (
            "Choose handling for this runtime observation. Missing evidence means defer.",
            &[
                ("observe", "Gather missing evidence before diagnosis"),
                (
                    "repair_agent",
                    "Ask the main agent to investigate and repair a new failure",
                ),
                (
                    "merge_pending",
                    "Same failure already has a pending investigation; avoid duplicate dispatch",
                ),
                (
                    "wait",
                    "No current failure or urgent condition; wait for another observation",
                ),
            ],
        ),
        "observation_selection" => (
            "Choose the next source of evidence for the goal.",
            &[
                (
                    "reuse_observation",
                    "Existing fresh observation contains the required evidence",
                ),
                (
                    "refresh_lens",
                    "Relevant Lens exists but its observation is stale",
                ),
                ("read_entity", "Known entity needs a current component read"),
                ("main_agent", "Need broader discovery or reasoning"),
            ],
        ),
        "behavior_selection" => (
            "Choose how to handle the event using existing verified behavior only.",
            &[
                (
                    "invoke_existing",
                    "A registered behavior matches the event and preconditions are verified",
                ),
                ("wait", "Behavior is cooling down or no response is needed"),
                (
                    "main_agent",
                    "Need new behavior, parameters, or reasoning; do not invent systems",
                ),
            ],
        ),
        "completion_review" => (
            "Review task obligations. Tool success is not goal completion.",
            &[
                (
                    "verify",
                    "Required outcome lacks an independent read-back or later execution",
                ),
                (
                    "cleanup",
                    "Checks passed but required temporary-object cleanup remains",
                ),
                (
                    "handoff",
                    "Required work failed or is blocked; report incomplete work",
                ),
                (
                    "finish",
                    "Every requested obligation and cleanup has supporting evidence",
                ),
            ],
        ),
        _ => return Err("Unknown Laya workflow scenario".into()),
    };
    let candidates: Vec<_> = options
        .iter()
        .map(|(operation, description)| json!({"operation":operation,"description":description}))
        .collect();
    let mut choices: HashSet<String> = options.iter().map(|(name, _)| name.to_string()).collect();
    choices.insert("defer".into());
    Ok((
        json!({"goal":goal,"state":{"scenario":r.scenario,"facts":r.state},"candidates":candidates}),
        choices,
    ))
}

pub(crate) async fn assess_at(input: &Value, base: &str) -> Result<Value, String> {
    let (request, choices) = prepare(input)?;
    let mut result = crate::tools::laya::predict(&request, &choices, base).await?;
    result["scenario"] = input["scenario"].clone();
    result["decision_kind"] = json!("route_not_executable_tool");
    result["completion_verified"] = json!(false);
    Ok(result)
}

pub(crate) async fn assess(input: &Value) -> Result<Value, String> {
    let base = std::env::var("BIGHEAD_LAYA_URL").unwrap_or_else(|_| "http://127.0.0.1:3031".into());
    assess_at(input, &base).await
}
