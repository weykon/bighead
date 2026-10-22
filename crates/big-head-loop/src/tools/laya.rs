//! Local decision advice. This module never executes the suggested operation.

use big_head_ai::ToolDefinition;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};

pub(super) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "laya_decide".into(),
        description: "Ask the local Laya model to recommend one of 1..7 available ECS operations from compact observed state. Always advisory: no operation is executed, no parameters generated, confidence is not calibrated for ECS. A defer option is added. Discover operations and read relevant state first. Verify any subsequent write independently. Service must be running; see guide_laya. Long inputs fail rather than silently truncate.".into(),
        input_schema: json!({"type":"object","properties":{
            "goal":{"type":"string","minLength":1,"maxLength":512},
            "state":{"type":"object","description":"Relevant observed facts only, up to 8192 bytes; include missing evidence explicitly"},
            "candidates":{"type":"array","minItems":1,"maxItems":7,"items":{
                "type":"object","properties":{
                    "operation":{"type":"string","description":"Exact available tool name"},
                    "description":{"type":"string","minLength":1,"maxLength":256}
                },"required":["operation","description"],"additionalProperties":false}}
        },"required":["goal","state","candidates"],"additionalProperties":false}),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    operation: String,
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    goal: String,
    state: serde_json::Map<String, Value>,
    candidates: Vec<Candidate>,
}

fn validate(input: &Value, available: &[ToolDefinition]) -> Result<HashSet<String>, String> {
    let request: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if request.goal.trim().is_empty()
        || request.goal.chars().count() > 512
        || serde_json::to_vec(&request.state).unwrap().len() > 8192
        || !(1..=7).contains(&request.candidates.len())
    {
        return Err("Provide a short goal, compact state and 1..7 candidates".into());
    }
    let mut choices = HashSet::from(["defer".to_owned()]);
    for candidate in request.candidates {
        if candidate.operation == "laya_decide"
            || !available
                .iter()
                .any(|tool| tool.name == candidate.operation)
            || !choices.insert(candidate.operation)
            || candidate.description.trim().is_empty()
            || candidate.description.chars().count() > 256
        {
            return Err("Candidates must be unique available tools with short descriptions".into());
        }
    }
    if input.to_string().len() > 16 * 1024 {
        return Err("Request exceeds 16 KiB".into());
    }
    Ok(choices)
}

fn endpoint_at(base: &str, path: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|e| e.to_string())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("BIGHEAD_LAYA_URL must be http://127.0.0.1:<port>".into());
    }
    url.set_path(path);
    Ok(url)
}

fn endpoint(base: &str) -> Result<reqwest::Url, String> {
    endpoint_at(base, "/decide")
}

fn validate_response(value: &Value, choices: &HashSet<String>) -> Result<(), String> {
    let operation = value["operation"].as_str().ok_or("Missing operation")?;
    let probs = value["probabilities"]
        .as_object()
        .ok_or("Missing probabilities")?;
    let confidence = value["confidence"].as_f64().ok_or("Missing confidence")?;
    let valid_probability = |v: &Value| {
        v.as_f64()
            .is_some_and(|n| n.is_finite() && (0.0..=1.0).contains(&n))
    };
    if value["protocol_version"] != 1
        || value["mode"] != "advisory"
        || value["executed"] != false
        || value["behavior_verified"] != false
        || value["input_truncated"] != false
        || value["confidence_calibrated_for_ecs"] != false
        || !choices.contains(operation)
        || !(0.0..=1.0).contains(&confidence)
        || probs.len() != choices.len()
        || !choices
            .iter()
            .all(|key| probs.get(key).is_some_and(valid_probability))
        || (probs.values().filter_map(Value::as_f64).sum::<f64>() - 1.0).abs() > 0.01
        || !["decision_id", "model", "revision", "device"]
            .iter()
            .all(|key| value[key].as_str().is_some_and(|s| !s.is_empty()))
    {
        return Err("Local model returned an invalid advisory response".into());
    }
    Ok(())
}

pub(super) async fn decide(input: &Value, available: &[ToolDefinition]) -> Result<Value, String> {
    let base = std::env::var("BIGHEAD_LAYA_URL").unwrap_or_else(|_| "http://127.0.0.1:3031".into());
    decide_at(input, available, &base).await
}

async fn decide_at(
    input: &Value,
    available: &[ToolDefinition],
    base: &str,
) -> Result<Value, String> {
    let choices = validate(input, available)?;
    predict(input, &choices, base).await
}

pub(crate) async fn predict(
    input: &Value,
    choices: &HashSet<String>,
    base: &str,
) -> Result<Value, String> {
    let value = post_json(input, base, "/decide").await?;
    validate_response(&value, choices)?;
    Ok(value)
}

async fn post_json(input: &Value, base: &str, path: &str) -> Result<Value, String> {
    let url = endpoint_at(base, path)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client.post(url).json(input).send().await
        .map_err(|e| format!("Local Laya service unavailable (not an LLM provider error): {e}. Start integrations/laya/service.py; see guide_laya"))?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err("Local Laya response too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid local Laya JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Local Laya rejected request ({status}): {}",
            value["error"]
        ));
    }
    Ok(value)
}

fn probability(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
}

fn validate_evaluation_response(value: &Value, request: &Value) -> Result<(), String> {
    if value["protocol_version"] != 2
        || value["spec_id"] != request["spec_id"]
        || value["spec_hash"] != request["spec_hash"]
        || value["mode"] != "advisory"
        || value["executed"] != false
        || value["behavior_verified"] != false
        || value["input_truncated"] != false
        || value["confidence_calibrated_for_ecs"] != false
        || !["decision_id", "model", "revision", "device"]
            .iter()
            .all(|key| value[key].as_str().is_some_and(|text| !text.is_empty()))
    {
        return Err("Local model returned invalid v2 decision metadata".into());
    }
    let questions = request["questions"]
        .as_object()
        .ok_or("Missing v2 questions")?;
    let answers = value["answers"].as_object().ok_or("Missing v2 answers")?;
    if answers.len() != questions.len() || questions.keys().any(|id| !answers.contains_key(id)) {
        return Err("Local model answer ids do not match the decision spec".into());
    }
    for (id, question) in questions {
        let answer = &answers[id];
        let kind = question["type"].as_str().ok_or("Missing question type")?;
        if answer["type"].as_str() != Some(kind) || probability(&answer["confidence"]).is_none() {
            return Err(format!("Invalid answer metadata for question '{id}'"));
        }
        match kind {
            "noul" if probability(&answer["noul"]).is_some() => {}
            "choice" => {
                let criteria = question["criteria"]
                    .as_object()
                    .ok_or("Invalid choice criteria")?;
                let probabilities = answer["probabilities"]
                    .as_object()
                    .ok_or("Invalid choice probabilities")?;
                let choice = answer["choice"].as_str().ok_or("Missing choice")?;
                if !criteria.contains_key(choice)
                    || criteria.len() != probabilities.len()
                    || criteria
                        .keys()
                        .any(|key| probability(&probabilities[key]).is_none())
                    || (probabilities
                        .values()
                        .filter_map(Value::as_f64)
                        .sum::<f64>()
                        - 1.0)
                        .abs()
                        > 0.01
                {
                    return Err(format!("Invalid choice answer for question '{id}'"));
                }
            }
            "score" => {
                let levels = question["criteria"]
                    .as_array()
                    .ok_or("Invalid score criteria")?;
                let score = answer["score"].as_f64().ok_or("Missing score")?;
                let probabilities = answer["probabilities"]
                    .as_object()
                    .ok_or("Invalid score probabilities")?;
                if !score.is_finite()
                    || !(0.0..=(levels.len() - 1) as f64).contains(&score)
                    || probabilities.len() != levels.len()
                    || (0..levels.len())
                        .any(|index| probability(&probabilities[&index.to_string()]).is_none())
                    || (probabilities
                        .values()
                        .filter_map(Value::as_f64)
                        .sum::<f64>()
                        - 1.0)
                        .abs()
                        > 0.01
                {
                    return Err(format!("Invalid score answer for question '{id}'"));
                }
            }
            _ => return Err(format!("Invalid answer for question '{id}'")),
        }
    }
    Ok(())
}

pub(crate) async fn evaluate_at(input: &Value, base: &str) -> Result<Value, String> {
    let value = post_json(input, base, "/evaluate").await?;
    validate_evaluation_response(&value, input)?;
    Ok(value)
}

pub(crate) async fn evaluate(input: &Value) -> Result<Value, String> {
    let base = std::env::var("BIGHEAD_LAYA_URL").unwrap_or_else(|_| "http://127.0.0.1:3031".into());
    evaluate_at(input, &base).await
}

pub(crate) async fn submit_outcome_at(input: &Value, base: &str) -> Result<Value, String> {
    let value = post_json(input, base, "/outcome").await?;
    if value["recorded"] != true || value["decision_id"] != input["decision_id"] {
        return Err("Local Laya service did not acknowledge the outcome".into());
    }
    Ok(value)
}

pub(crate) async fn submit_outcome(input: &Value) -> Result<Value, String> {
    let base = std::env::var("BIGHEAD_LAYA_URL").unwrap_or_else(|_| "http://127.0.0.1:3031".into());
    submit_outcome_at(input, &base).await
}

#[cfg(test)]
#[path = "laya_tests.rs"]
mod tests;
