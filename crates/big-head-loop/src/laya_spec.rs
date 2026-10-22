//! Versioned decision specs: observed state -> typed judgments -> policy -> bounded action.

use big_head_ai::{ToolCall, ToolDefinition};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};

const MAX_STATE_BYTES: usize = 8 * 1024;
const MAX_SPEC_BYTES: usize = 32 * 1024;
const MAX_QUESTIONS: usize = 16;
const MAX_ACTIONS: usize = 16;
const MAX_VERIFICATIONS: usize = 16;

pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "laya_run_spec".into(),
        description: "Evaluate a versioned DecisionSpec with the local Laya reflex model. The spec supplies compact observed state, parallel choice/score/noul questions, deterministic policy gates, bounded existing actions and read-back verification. mode=shadow records a decision without acting. mode=execute can run immediately when current tool capabilities, a confidence gate and verification checks permit it. It cannot define capabilities or run CLI/network tools. See guide_laya.".into(),
        input_schema: json!({
            "type":"object",
            "properties":{
                "mode":{"type":"string","enum":["shadow","execute"]},
                "spec":{"type":"object","properties":{
                    "version":{"type":"integer","enum":[2]},
                    "id":{"type":"string","minLength":1,"maxLength":128},
                    "goal":{"type":"string","minLength":1,"maxLength":512},
                    "state":{"type":"object"},
                    "questions":{"type":"object","minProperties":1,"maxProperties":16},
                    "policy":{"type":"object"},
                    "actions":{"type":"object","minProperties":1,"maxProperties":16}
                },"required":["version","id","goal","state","questions","policy","actions"],"additionalProperties":false}
            },
            "required":["mode","spec"],
            "additionalProperties":false
        }),
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionMode {
    Shadow,
    Execute,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    mode: ExecutionMode,
    spec: DecisionSpec,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DecisionSpec {
    version: u8,
    id: String,
    goal: String,
    state: Map<String, Value>,
    questions: BTreeMap<String, QuestionSpec>,
    policy: PolicySpec,
    actions: BTreeMap<String, ActionSpec>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct QuestionSpec {
    #[serde(rename = "type")]
    kind: String,
    instructions: String,
    #[serde(default)]
    criteria: Option<Value>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PolicySpec {
    action_question: String,
    defer_choices: Vec<String>,
    gates: Vec<PolicyGate>,
    on_gate_failure: String,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PolicyGate {
    question: String,
    metric: String,
    operator: String,
    value: f64,
    #[serde(default)]
    option: Option<String>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionSpec {
    pub(crate) tool: String,
    pub(crate) input: Value,
    description: String,
    pub(crate) verify: Vec<VerificationSpec>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerificationSpec {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) input: Value,
    expect: VerificationExpectation,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct VerificationExpectation {
    kind: String,
    #[serde(default)]
    pointer: Option<String>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    text: Option<String>,
}

pub(crate) struct PreparedDecision {
    pub(crate) mode: ExecutionMode,
    pub(crate) spec_id: String,
    pub(crate) spec_hash: String,
    pub(crate) decision_id: String,
    pub(crate) action_id: Option<String>,
    pub(crate) action: Option<ActionSpec>,
    pub(crate) verdict: String,
    pub(crate) policy: Value,
    pub(crate) model: Value,
}

fn valid_id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (b"_.:-".contains(&byte) && index > 0)
        })
        && value.as_bytes()[0].is_ascii_alphabetic()
}

fn validate_question(id: &str, question: &QuestionSpec) -> Result<(), String> {
    if !valid_id(id, 80)
        || question.instructions.trim().is_empty()
        || question.instructions.chars().count() > 512
    {
        return Err(format!("Invalid question '{id}'"));
    }
    match question.kind.as_str() {
        "choice" => {
            let criteria = question
                .criteria
                .as_ref()
                .and_then(Value::as_object)
                .ok_or_else(|| format!("Choice '{id}' needs an object criteria map"))?;
            if !(2..=32).contains(&criteria.len()) || criteria.keys().any(|key| !valid_id(key, 80))
            {
                return Err(format!("Choice '{id}' needs 2..32 valid labels"));
            }
        }
        "score" => {
            let levels = question
                .criteria
                .as_ref()
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Score '{id}' needs ordered criteria"))?;
            if !(2..=8).contains(&levels.len()) || levels.iter().any(|level| !level.is_string()) {
                return Err(format!("Score '{id}' needs 2..8 string levels"));
            }
        }
        "noul" => {
            if let Some(criteria) = &question.criteria {
                let object = criteria
                    .as_object()
                    .ok_or_else(|| format!("Noul '{id}' criteria must be an object"))?;
                if object.len() != 2
                    || !object.contains_key("false")
                    || !object.contains_key("true")
                {
                    return Err(format!("Noul '{id}' criteria needs false and true"));
                }
            }
        }
        _ => return Err(format!("Unknown question type for '{id}'")),
    }
    Ok(())
}

fn validate_gate(
    gate: &PolicyGate,
    questions: &BTreeMap<String, QuestionSpec>,
) -> Result<(), String> {
    let question = questions
        .get(&gate.question)
        .ok_or_else(|| format!("Gate references unknown question '{}'", gate.question))?;
    if !gate.value.is_finite() || !matches!(gate.operator.as_str(), "gte" | "lte") {
        return Err("Policy gate needs a finite value and gte/lte operator".into());
    }
    match gate.metric.as_str() {
        "confidence" if (0.0..=1.0).contains(&gate.value) && gate.option.is_none() => {}
        "noul"
            if question.kind == "noul"
                && (0.0..=1.0).contains(&gate.value)
                && gate.option.is_none() => {}
        "score"
            if question.kind == "score"
                && gate.option.is_none()
                && question
                    .criteria
                    .as_ref()
                    .and_then(Value::as_array)
                    .is_some_and(|levels| {
                        (0.0..=(levels.len() - 1) as f64).contains(&gate.value)
                    }) => {}
        "choice_probability" if question.kind == "choice" => {
            let option = gate
                .option
                .as_deref()
                .ok_or("choice_probability gate needs option")?;
            if !question
                .criteria
                .as_ref()
                .and_then(Value::as_object)
                .is_some_and(|values| values.contains_key(option))
                || !(0.0..=1.0).contains(&gate.value)
            {
                return Err("choice_probability gate has an invalid option or threshold".into());
            }
        }
        _ => {
            return Err(format!(
                "Metric '{}' does not match question '{}'",
                gate.metric, gate.question
            ));
        }
    }
    Ok(())
}

fn validate_verification(check: &VerificationSpec) -> Result<(), String> {
    if !valid_id(&check.id, 80)
        || !matches!(
            check.tool.as_str(),
            "get_entity"
                | "get_component"
                | "query_entities"
                | "query_ecs"
                | "inspect_scene"
                | "inspect_work"
                | "inspect_world_shape"
        )
        || !check.input.is_object()
    {
        return Err("Verification checks need an id, a read-only tool and object input".into());
    }
    match check.expect.kind.as_str() {
        "success"
            if check.expect.pointer.is_none()
                && check.expect.value.is_none()
                && check.expect.text.is_none() => {}
        "output_contains" | "output_not_contains"
            if check
                .expect
                .text
                .as_ref()
                .is_some_and(|text| !text.is_empty() && text.chars().count() <= 256)
                && check.expect.pointer.is_none()
                && check.expect.value.is_none() => {}
        "json_pointer_equals"
            if check
                .expect
                .pointer
                .as_ref()
                .is_some_and(|pointer| pointer.is_empty() || pointer.starts_with('/'))
                && check.expect.value.is_some()
                && check.expect.text.is_none() => {}
        "json_pointer_exists"
            if check
                .expect
                .pointer
                .as_ref()
                .is_some_and(|pointer| pointer.is_empty() || pointer.starts_with('/'))
                && check.expect.value.is_none()
                && check.expect.text.is_none() => {}
        _ => {
            return Err(format!(
                "Invalid expectation for verification '{}'",
                check.id
            ));
        }
    }
    Ok(())
}

pub(crate) fn action_is_bounded(tool: &str, registered_system_tools: &HashSet<String>) -> bool {
    matches!(
        tool,
        "get_entity"
            | "get_component"
            | "query_entities"
            | "query_ecs"
            | "inspect_scene"
            | "inspect_work"
            | "inspect_world_shape"
            | "spawn_entity"
            | "modify_component"
            | "remove_component"
            | "despawn_entity"
            | "run_system"
            | "record_work"
            | "arrange_work"
    ) || registered_system_tools.contains(tool)
}

fn validate(
    request: &Request,
    available: &HashSet<String>,
    registered_system_tools: &HashSet<String>,
) -> Result<(), String> {
    let spec = &request.spec;
    if spec.version != 2
        || !valid_id(&spec.id, 128)
        || spec.goal.trim().is_empty()
        || spec.goal.chars().count() > 512
    {
        return Err("DecisionSpec needs version=2, a valid id and a short goal".into());
    }
    if serde_json::to_vec(&spec.state)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_STATE_BYTES
        || serde_json::to_vec(spec)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_SPEC_BYTES
        || !(1..=MAX_QUESTIONS).contains(&spec.questions.len())
        || !(1..=MAX_ACTIONS).contains(&spec.actions.len())
    {
        return Err("DecisionSpec state/questions/actions exceed bounded limits".into());
    }
    for (id, question) in &spec.questions {
        validate_question(id, question)?;
    }
    if !matches!(spec.policy.on_gate_failure.as_str(), "defer" | "escalate")
        || spec.policy.defer_choices.is_empty()
        || spec
            .policy
            .defer_choices
            .iter()
            .any(|choice| !valid_id(choice, 80))
    {
        return Err("Policy needs defer choices and defer/escalate failure behavior".into());
    }
    let action_question = spec
        .questions
        .get(&spec.policy.action_question)
        .filter(|question| question.kind == "choice")
        .ok_or("policy.action_question must name a choice question")?;
    let labels: HashSet<_> = action_question
        .criteria
        .as_ref()
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let expected: HashSet<_> = spec
        .actions
        .keys()
        .cloned()
        .chain(spec.policy.defer_choices.iter().cloned())
        .collect();
    if labels != expected
        || spec
            .actions
            .keys()
            .any(|id| spec.policy.defer_choices.contains(id))
    {
        return Err("Action choice labels must exactly match actions plus defer_choices".into());
    }
    for gate in &spec.policy.gates {
        validate_gate(gate, &spec.questions)?;
    }
    let action_confidence_gate = spec.policy.gates.iter().any(|gate| {
        gate.question == spec.policy.action_question
            && gate.metric == "confidence"
            && gate.operator == "gte"
            && gate.value >= 0.5
    });
    for (id, action) in &spec.actions {
        let mut verification_ids = HashSet::new();
        if !valid_id(id, 80)
            || !available.contains(&action.tool)
            || !action.input.is_object()
            || action.description.trim().is_empty()
            || action.description.chars().count() > 256
            || action.tool.starts_with("laya_")
            || action.verify.len() > MAX_VERIFICATIONS
        {
            return Err(format!("Action '{id}' is unavailable or invalid"));
        }
        for check in &action.verify {
            if !verification_ids.insert(&check.id) {
                return Err(format!("Action '{id}' contains duplicate verification ids"));
            }
            validate_verification(check)?;
        }
        if request.mode == ExecutionMode::Execute
            && (!action_is_bounded(&action.tool, registered_system_tools)
                || action.verify.is_empty())
        {
            return Err(format!(
                "Executable action '{id}' is not bounded or has no read-back verification"
            ));
        }
    }
    if request.mode == ExecutionMode::Execute
        && (!action_confidence_gate || spec.policy.gates.is_empty())
    {
        return Err(
            "Execute mode requires an action confidence gate >= 0.5 and explicit policy gates"
                .into(),
        );
    }
    Ok(())
}

fn question_json(question: &QuestionSpec) -> Value {
    let mut value = json!({"type":question.kind,"instructions":question.instructions});
    if let Some(criteria) = &question.criteria {
        value["criteria"] = criteria.clone();
    }
    value
}

fn gate_metric(gate: &PolicyGate, answers: &Value) -> Option<f64> {
    let answer = &answers[&gate.question];
    match gate.metric.as_str() {
        "confidence" => answer["confidence"].as_f64(),
        "noul" => answer["noul"].as_f64(),
        "score" => answer["score"].as_f64(),
        "choice_probability" => gate
            .option
            .as_ref()
            .and_then(|option| answer["probabilities"][option].as_f64()),
        _ => None,
    }
}

pub(crate) async fn prepare(
    input: &Value,
    available: HashSet<String>,
    registered_system_tools: HashSet<String>,
) -> Result<PreparedDecision, String> {
    let request: Request =
        serde_json::from_value(input.clone()).map_err(|error| error.to_string())?;
    validate(&request, &available, &registered_system_tools)?;
    let spec_bytes = serde_json::to_vec(&request.spec).map_err(|error| error.to_string())?;
    let spec_hash = big_head_save::persistence::digest(&spec_bytes);
    let questions: Map<String, Value> = request
        .spec
        .questions
        .iter()
        .map(|(id, question)| (id.clone(), question_json(question)))
        .collect();
    let inference = json!({"spec_version":2,"spec_id":request.spec.id,"spec_hash":spec_hash,
        "state":request.spec.state,"questions":questions});
    let model = crate::tools::laya::evaluate(&inference).await?;
    let answers = &model["answers"];
    let selected = answers[&request.spec.policy.action_question]["choice"]
        .as_str()
        .ok_or("Action question did not return a choice")?
        .to_string();
    let gates: Vec<Value> = request
        .spec
        .policy
        .gates
        .iter()
        .map(|gate| {
            let observed = gate_metric(gate, answers);
            let passed = observed.is_some_and(|number| match gate.operator.as_str() {
                "gte" => number >= gate.value,
                "lte" => number <= gate.value,
                _ => false,
            });
            json!({"question":gate.question,"metric":gate.metric,"option":gate.option,
                "operator":gate.operator,"threshold":gate.value,"observed":observed,"passed":passed})
        })
        .collect();
    let gates_passed = gates.iter().all(|gate| gate["passed"] == true);
    let is_defer = request.spec.policy.defer_choices.contains(&selected);
    let (verdict, reason) = if is_defer {
        ("defer", "model selected an explicit defer choice")
    } else if !gates_passed {
        (
            request.spec.policy.on_gate_failure.as_str(),
            "one or more deterministic policy gates failed",
        )
    } else if request.mode == ExecutionMode::Shadow {
        ("shadow", "shadow mode never executes")
    } else {
        ("act", "selected action passed all policy and runtime gates")
    };
    let action = request.spec.actions.get(&selected).cloned();
    Ok(PreparedDecision {
        mode: request.mode,
        spec_id: request.spec.id,
        spec_hash,
        decision_id: model["decision_id"].as_str().unwrap().to_string(),
        action_id: action.as_ref().map(|_| selected.clone()),
        action,
        verdict: verdict.into(),
        policy: json!({"verdict":verdict,"reason":reason,"selected":selected,
            "gates_passed":gates_passed,"gates":gates}),
        model,
    })
}

pub(crate) fn action_call(decision: &PreparedDecision) -> Option<ToolCall> {
    let action = decision.action.as_ref()?;
    Some(ToolCall {
        id: format!("{}:action", decision.decision_id),
        name: action.tool.clone(),
        input: action.input.clone(),
    })
}

pub(crate) fn verification_call(decision: &PreparedDecision, check: &VerificationSpec) -> ToolCall {
    ToolCall {
        id: format!("{}:verify:{}", decision.decision_id, check.id),
        name: check.tool.clone(),
        input: check.input.clone(),
    }
}

fn structured_output(output: &str) -> Option<Value> {
    serde_json::from_str(output).ok().or_else(|| {
        output
            .split_once(": ")
            .and_then(|(_, value)| serde_json::from_str(value).ok())
    })
}

pub(crate) fn assess_verification(check: &VerificationSpec, output: &str, is_error: bool) -> Value {
    let parsed = structured_output(output);
    let passed = if is_error {
        false
    } else {
        match check.expect.kind.as_str() {
            "success" => true,
            "output_contains" => check
                .expect
                .text
                .as_ref()
                .is_some_and(|text| output.contains(text)),
            "output_not_contains" => check
                .expect
                .text
                .as_ref()
                .is_some_and(|text| !output.contains(text)),
            "json_pointer_equals" => {
                parsed
                    .as_ref()
                    .and_then(|value| value.pointer(check.expect.pointer.as_deref().unwrap()))
                    == check.expect.value.as_ref()
            }
            "json_pointer_exists" => parsed
                .as_ref()
                .and_then(|value| value.pointer(check.expect.pointer.as_deref().unwrap()))
                .is_some(),
            _ => false,
        }
    };
    json!({"id":check.id,"tool":check.tool,"passed":passed,"is_error":is_error,
        "expect":check.expect,"output_sha256":big_head_save::persistence::digest(output.as_bytes()),
        "output_preview":output.chars().take(240).collect::<String>()})
}

pub(crate) fn outcome(
    decision: &PreparedDecision,
    execution_attempted: bool,
    execution_succeeded: Option<bool>,
    execution_output: Option<&str>,
    verification: Vec<Value>,
) -> Value {
    let behavior_verified = execution_succeeded == Some(true)
        && !verification.is_empty()
        && verification.iter().all(|check| check["passed"] == true);
    json!({
        "decision_id":decision.decision_id,"spec_id":decision.spec_id,"spec_hash":decision.spec_hash,
        "action_id":decision.action_id,"execution_attempted":execution_attempted,
        "execution_succeeded":execution_succeeded,"behavior_verified":behavior_verified,
        "goal_achieved":if behavior_verified { Value::Bool(true) } else { Value::Null },
        "verification":verification,
        "evidence":{
            "policy_verdict":decision.verdict,
            "execution_output_sha256":execution_output.map(|output|big_head_save::persistence::digest(output.as_bytes())),
            "execution_output_preview":execution_output.map(|output|output.chars().take(240).collect::<String>())
        }
    })
}

pub(crate) fn result(
    decision: PreparedDecision,
    execution: Value,
    verification: Vec<Value>,
    outcome: Value,
    outcome_record: Result<Value, String>,
) -> Value {
    json!({"spec_version":2,"spec_id":decision.spec_id,"spec_hash":decision.spec_hash,
        "decision_id":decision.decision_id,
        "mode":match decision.mode {ExecutionMode::Shadow=>"shadow",ExecutionMode::Execute=>"execute"},
        "model_decision":decision.model,"policy":decision.policy,"execution":execution,
        "verification":verification,"outcome":outcome,
        "outcome_record":match outcome_record {Ok(value)=>value,Err(error)=>json!({"recorded":false,"error":error})}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_reads_json_component_values() {
        let check: VerificationSpec = serde_json::from_value(json!({
            "id":"count","tool":"get_component",
            "input":{"entity_id":1,"component":"Counter"},
            "expect":{"kind":"json_pointer_equals","pointer":"/count","value":2}
        }))
        .unwrap();
        assert_eq!(
            assess_verification(&check, "Counter: {\"count\":2}", false)["passed"],
            true
        );
        assert_eq!(
            assess_verification(&check, "Counter: {\"count\":1}", false)["passed"],
            false
        );
        assert_eq!(assess_verification(&check, "failed", true)["passed"], false);

        let root_check: VerificationSpec = serde_json::from_value(json!({
            "id":"health","tool":"get_component","input":{},
            "expect":{"kind":"json_pointer_equals","pointer":"","value":2.0}
        }))
        .unwrap();
        validate_verification(&root_check).unwrap();
        assert_eq!(
            assess_verification(&root_check, "TestHealth: 2.0", false)["passed"],
            true
        );
    }

    #[test]
    fn executable_specs_require_confidence_and_readback() {
        let request: Request = serde_json::from_value(json!({"mode":"execute","spec":{
            "version":2,"id":"counter.v1","goal":"Increment counter","state":{"count":0},
            "questions":{"action":{"type":"choice","instructions":"Choose action","criteria":{"increment":"increment","defer":"wait"}},
                "ready":{"type":"noul","instructions":"Is evidence sufficient?"}},
            "policy":{"action_question":"action","defer_choices":["defer"],"gates":[
                {"question":"ready","metric":"noul","operator":"gte","value":0.8},
                {"question":"action","metric":"confidence","operator":"gte","value":0.7}],"on_gate_failure":"defer"},
            "actions":{"increment":{"tool":"modify_component","input":{},"description":"increment",
                "verify":[{"id":"count","tool":"get_component","input":{},"expect":{"kind":"success"}}]}}
        }}))
        .unwrap();
        let available = HashSet::from(["modify_component".into()]);
        assert!(validate(&request, &available, &HashSet::new()).is_ok());
        let mut no_verification: Value = serde_json::to_value(&request.spec).unwrap();
        no_verification["actions"]["increment"]["verify"] = json!([]);
        let invalid: Request =
            serde_json::from_value(json!({"mode":"execute","spec":no_verification})).unwrap();
        assert!(
            validate(&invalid, &available, &HashSet::new())
                .unwrap_err()
                .contains("read-back")
        );
    }
}
