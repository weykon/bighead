//! Bounded, agent-directed Dream-RSI experiments on executable Rhai tasks.
mod replay;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use bevy_ecs::{component::Component, entity::Entity, world::World};
use bevy_reflect::Reflect;
use big_head_ai::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use replay::{BASELINE, Case, MAX_NODES, Tree};

const MAX_EXPERIMENTS: usize = 4;
const MAX_WORLDS: usize = 8;
const MAX_STATE_BYTES: usize = 1_048_576;

#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
pub struct DreamExperiment {
    pub id: String,
    pub state_json: String,
}

impl crate::AgentRuntime {
    /// Run one explicitly requested, bounded practice session with only dream tools.
    /// Callers own checkpoint timing, just as for other runtime entrypoints.
    pub async fn run_dream_practice(&mut self, request: String) -> Result<String, String> {
        let definition = crate::SystemDefinition {
            id: "dream-practice".into(),
            description: "Isolated Rhai discovery and replay-based policy improvement".into(),
            system_prompt: include_str!("../../../big-head-ai/prompts/guides/dream_guide.md")
                .into(),
            kind: crate::SystemKind::Llm,
            tool_names: vec!["dream_task".into(), "dream_policy".into()],
            max_iterations: 32,
            is_tool: false,
        };
        if let Some(existing) = self.system_registry.get("dream-practice") {
            if serde_json::to_value(existing).unwrap() != serde_json::to_value(&definition).unwrap()
            {
                return Err("dream-practice definition differs from the bounded built-in".into());
            }
        } else {
            self.system_registry.register(definition)?;
        }
        self.run_llm_system("dream-practice", request).await
    }

    pub fn inspect_dream_experiments(&mut self) -> Result<Value, String> {
        let result = dispatch(
            &mut self.world,
            &ToolCall {
                id: "runtime-dream-inspection".into(),
                name: "dream_task".into(),
                input: json!({"action":"inspect"}),
            },
        )?;
        serde_json::from_str(&result).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    description: String,
    cases: Vec<Case>,
    #[serde(default = "default_budget")]
    budget: usize,
    #[serde(default = "default_workers")]
    workers: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskInput {
    description: String,
    cases_json: String,
    #[serde(default = "default_budget")]
    budget: usize,
    #[serde(default = "default_workers")]
    workers: usize,
}

impl TaskInput {
    fn into_task(self) -> Result<Task, String> {
        let cases = serde_json::from_str(&self.cases_json)
            .map_err(|e| format!("cases_json must encode an array of {{input,expected}}: {e}"))?;
        Ok(Task {
            description: self.description,
            cases,
            budget: self.budget,
            workers: self.workers,
        })
    }
}

fn default_budget() -> usize {
    8
}
fn default_workers() -> usize {
    1
}

#[derive(Clone, Serialize, Deserialize)]
struct Pending {
    id: String,
    parents: Vec<usize>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Submission {
    batch_id: String,
    digest: String,
    result: Value,
}

#[derive(Clone, Serialize, Deserialize)]
struct State {
    format_version: u32,
    revision: u64,
    task: Task,
    policy: String,
    policy_version: usize,
    previous_policy: Option<String>,
    history: Vec<Tree>,
    active: Option<Tree>,
    pending: Option<Pending>,
    last_submission: Option<Submission>,
    last_dream: Option<Value>,
    dream_calls: usize,
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![
        tool(
            "dream_task",
            "Run a bounded Dream-RSI experiment. Read run_system('guide_dream') first. action=create freezes a task with JSON input/expected cases; next starts/resumes a round and returns the deployed policy's selected parent scripts plus batch_id; submit executes one Rhai expression per selected parent in a PURE isolated engine (variable input), records runtime scores, and never touches live ECS. finish seals a nonempty round. inspect lists experiments or returns details for id. rollback restores the preceding exploration policy between rounds. This is measured practice, not production skill deployment. No automatic background loop. At most 4 experiments, 8 rounds each, 32 attempts/round. Existing task definitions are immutable.",
            json!({
                "action":{"type":"string","enum":["create","inspect","next","submit","finish","rollback"]},
                "id":{"type":"string"},
                "task":{"type":"object","properties":{
                    "description":{"type":"string"},
                    "cases_json":{"type":"string","description":"JSON text encoding 1..16 {input,expected} cases. Preserve JSON types: numbers unquoted, strings quoted, empty array []. Example: [{\"input\":[-2,3,0,4],\"expected\":7},{\"input\":[],\"expected\":0}]"},
                    "budget":{"type":"integer","minimum":1,"maximum":32},
                    "workers":{"type":"integer","minimum":1,"maximum":4}
                },"required":["description","cases_json"],"additionalProperties":false},
                "batch_id":{"type":"string"},
                "scripts":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"string"}},
                "expected_revision":{"type":"integer","minimum":0},
                "round":{"type":"integer","minimum":1,"maximum":8}
            }),
            &["action"],
        ),
        tool(
            "dream_policy",
            "Evaluate 1..4 proposed Rhai exploration policies against ALL completed trees of a dream_task. Each expression receives obs={nodes:[{id,parent,score,passed}],available:[parent IDs],best,batches,remaining,workers} and returns a unique array of eligible parent IDs (0 opens a new branch; [] stops). Only revealed prefixes are visible; no tool/model/evaluator execution during replay. Fixed utility=best-0.01*attempts+0.001*attempts/batches. Include current policy automatically; adopt only strict improvement with no per-world best-score regression. Requires no active round and expected_revision. Adoption controls next practice round, not live ECS. Returns visited IDs, scores, policy hashes and scope. Historical improvement is not a future-performance guarantee. Maximum 4 dreaming calls per history revision.",
            json!({"id":{"type":"string"},"expected_revision":{"type":"integer","minimum":0},"policies":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"string"}}}),
            &["id", "expected_revision", "policies"],
        ),
    ]
}

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty {key}"))
}

fn catalog(world: &mut World) -> Result<Vec<(Entity, DreamExperiment)>, String> {
    let mut entries: Vec<_> = world
        .query::<(Entity, &DreamExperiment)>()
        .iter(world)
        .map(|(entity, experiment)| (entity, experiment.clone()))
        .collect();
    entries.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    if entries.len() > MAX_EXPERIMENTS || entries.windows(2).any(|p| p[0].1.id == p[1].1.id) {
        return Err("duplicate DreamExperiment IDs or experiment limit exceeded".into());
    }
    Ok(entries)
}

fn decode(record: &DreamExperiment) -> Result<State, String> {
    if record.state_json.len() > MAX_STATE_BYTES {
        return Err("dream state exceeds 1 MiB".into());
    }
    let state: State =
        serde_json::from_str(&record.state_json).map_err(|e| format!("dream state: {e}"))?;
    if state.format_version != 1 || state.history.len() > MAX_WORLDS {
        return Err("unsupported dream format or too many replay worlds".into());
    }
    validate_task(&state.task)?;
    Ok(state)
}

fn save(world: &mut World, entity: Entity, state: &State) -> Result<(), String> {
    let data = serde_json::to_string(state).map_err(|e| e.to_string())?;
    if data.len() > MAX_STATE_BYTES {
        return Err(
            "dream state would exceed 1 MiB; start a separate experiment after archiving".into(),
        );
    }
    world
        .get_mut::<DreamExperiment>(entity)
        .ok_or("experiment disappeared")?
        .state_json = data;
    Ok(())
}

fn validate_task(task: &Task) -> Result<(), String> {
    if task.description.trim().is_empty()
        || task.description.len() > 2000
        || task.cases.is_empty()
        || task.cases.len() > 16
        || !(1..=MAX_NODES).contains(&task.budget)
        || !(1..=4).contains(&task.workers)
        || serde_json::to_vec(&task.cases)
            .map_err(|e| e.to_string())?
            .len()
            > 16_384
    {
        return Err("task requires description (1..2000 bytes), 1..16 cases (<=16 KiB), budget 1..32, workers 1..4".into());
    }
    // Reject inputs the execution engine cannot represent before accepting the task.
    for case in &task.cases {
        let _ = rhai::serde::to_dynamic(&case.input).map_err(|e| format!("case input: {e}"))?;
    }
    Ok(())
}

fn overview(id: &str, state: &State) -> Value {
    json!({"id":id,"revision":state.revision,"task":state.task.description,
        "policy_version":state.policy_version,"policy":state.policy,
        "completed_rounds":state.history.len(),"round_limit":MAX_WORLDS,
        "active_attempts":state.active.as_ref().map(|t|t.nodes.len()),
        "pending_batch":state.pending,"dream_calls_this_history":state.dream_calls,
        "round_results":state.history.iter().map(|t|json!({"policy_version":t.policy_version,"measured":replay::online_score(t)})).collect::<Vec<_>>(),
        "last_dream":state.last_dream,
        "scope":"isolated Rhai practice; test-case correctness only; no production skill deployed"})
}

fn finish(state: &mut State) -> Result<Value, String> {
    let tree = state.active.as_ref().ok_or("no active round")?;
    if tree.nodes.is_empty() {
        return Err("cannot record an empty replay world; submit at least one attempt".into());
    }
    let result = json!({"round":state.history.len()+1,"measured":replay::online_score(tree),"policy_version":tree.policy_version});
    state.history.push(state.active.take().unwrap());
    state.pending = None;
    state.dream_calls = 0;
    Ok(result)
}

fn next(state: &mut State) -> Result<Value, String> {
    if state.active.is_none() {
        if state.history.len() == MAX_WORLDS {
            return Err("experiment has reached 8 recorded rounds; inspect or replay it, no more online attempts".into());
        }
        state.active = Some(Tree {
            policy_version: state.policy_version,
            ..Tree::default()
        });
    }
    let tree = state.active.as_ref().unwrap();
    if state.pending.is_none() {
        let parents = replay::choose(
            &state.policy,
            &tree.nodes,
            &BTreeSet::new(),
            tree.batches,
            state.task.budget,
            state.task.workers,
        )?;
        if parents.is_empty() {
            return Ok(json!({"stopped":true,"result":finish(state)?}));
        }
        state.pending = Some(Pending {
            id: uuid::Uuid::new_v4().to_string(),
            parents,
        });
    }
    let pending = state.pending.as_ref().unwrap();
    let parents: Vec<_> = pending
        .parents
        .iter()
        .map(|id| {
            if *id == 0 {
                json!({"id":0,"script":null,"score":null})
            } else {
                serde_json::to_value(tree.nodes.iter().find(|n| n.id == *id).unwrap()).unwrap()
            }
        })
        .collect();
    Ok(
        json!({"batch_id":pending.id,"parents":parents,"task":state.task,
        "round":state.history.len()+1,"policy_version":state.policy_version,
        "instruction":"Produce one candidate Rhai expression per parent, in this order. Refine the supplied parent script, or start fresh for parent 0. Variable input contains each JSON test input. Submit with batch_id. Results are measured by runtime, not supplied by the agent."}),
    )
}

fn submit(state: &mut State, input: &Value) -> Result<Value, String> {
    let batch_id = text(input, "batch_id")?;
    let scripts: Vec<String> =
        serde_json::from_value(input.get("scripts").cloned().ok_or("missing scripts")?)
            .map_err(|e| e.to_string())?;
    let digest =
        big_head_save::persistence::digest(serde_json::to_string(&scripts).unwrap().as_bytes());
    if let Some(previous) = &state.last_submission {
        if previous.batch_id == batch_id {
            return if previous.digest == digest {
                Ok(previous.result.clone())
            } else {
                Err("batch_id already submitted with different scripts".into())
            };
        }
    }
    let pending = state
        .pending
        .as_ref()
        .ok_or("call next before submitting candidates")?;
    if pending.id != batch_id || pending.parents.len() != scripts.len() {
        return Err("stale batch_id or wrong number of candidate scripts".into());
    }
    for script in &scripts {
        replay::check_script(script)?;
    }
    let tree = state.active.as_mut().ok_or("no active round")?;
    let mut results = Vec::new();
    for (script, parent) in scripts.iter().zip(&pending.parents) {
        let node = replay::evaluate(script, &state.task.cases, tree.nodes.len() + 1, *parent);
        results.push(json!({"id":node.id,"parent":node.parent,"passed":node.passed,"score":node.score,"errors":node.errors}));
        tree.nodes.push(node);
    }
    tree.batches += 1;
    state.pending = None;
    let completed = if tree.nodes.len() == state.task.budget {
        Some(finish(state)?)
    } else {
        None
    };
    let result = json!({"attempts":results,"completed":completed,"evaluator":"fixed JSON equality cases; fraction passed"});
    state.last_submission = Some(Submission {
        batch_id: batch_id.into(),
        digest,
        result: result.clone(),
    });
    Ok(result)
}

fn dream(state: &mut State, input: &Value) -> Result<Value, String> {
    if state.active.is_some() || state.history.is_empty() {
        return Err(
            "finish a nonempty online round before dreaming; no active round allowed".into(),
        );
    }
    if state.dream_calls >= 4 {
        return Err(
            "4 dreaming calls already used for this history; collect a new online round".into(),
        );
    }
    let policies: Vec<String> =
        serde_json::from_value(input.get("policies").cloned().ok_or("missing policies")?)
            .map_err(|e| e.to_string())?;
    if policies.is_empty() || policies.len() > 4 {
        return Err("provide 1..4 candidate policies".into());
    }
    for policy in &policies {
        replay::check_script(policy)?;
    }
    let evaluate = |policy: &str| -> Result<Vec<replay::ReplayScore>, String> {
        state
            .history
            .iter()
            .map(|t| replay::replay(policy, t, state.task.budget, state.task.workers))
            .collect()
    };
    let baseline = evaluate(&state.policy)?;
    let mean = |scores: &[replay::ReplayScore]| {
        scores.iter().map(|s| s.utility).sum::<f64>() / scores.len() as f64
    };
    let baseline_mean = mean(&baseline);
    let mut winner = state.policy.clone();
    let mut best = baseline_mean;
    let mut trials = Vec::new();
    for policy in &policies {
        let hash = big_head_save::persistence::digest(policy.as_bytes());
        let evaluated = replay::choose(
            policy,
            &[],
            &BTreeSet::new(),
            0,
            state.task.budget,
            state.task.workers,
        )
        .and_then(|batch| {
            if batch.is_empty() {
                Err("policy must start with at least one attempt".into())
            } else {
                evaluate(policy)
            }
        });
        match evaluated {
            Ok(scores) => {
                let quality_preserved = scores
                    .iter()
                    .zip(&baseline)
                    .all(|(new, old)| new.best >= old.best);
                let utility = mean(&scores);
                if quality_preserved && utility > best + 1e-9 {
                    winner = policy.clone();
                    best = utility;
                }
                trials.push(json!({"policy_sha256":hash,"worlds":scores,"mean_utility":utility,"quality_preserved":quality_preserved}));
            }
            Err(error) => trials.push(
                json!({"policy_sha256":hash,"error":error.chars().take(512).collect::<String>()}),
            ),
        }
    }
    let adopted = winner != state.policy;
    if adopted {
        state.previous_policy = Some(std::mem::replace(&mut state.policy, winner));
        state.policy_version += 1;
    }
    state.dream_calls += 1;
    let receipt = json!({"adopted":adopted,"policy_version":state.policy_version,
        "policy_sha256":big_head_save::persistence::digest(state.policy.as_bytes()),
        "baseline":baseline,"baseline_mean_utility":baseline_mean,"selected_mean_utility":best,
        "candidates":trials,"worlds":state.history.len(),
        "new_candidate_executions":0,"discovery_agent_calls":0,
        "scope":"replay selection only; applies to next isolated practice round; future performance unproven"});
    state.last_dream = Some(receipt.clone());
    Ok(receipt)
}

pub(crate) fn dispatch(world: &mut World, call: &ToolCall) -> Result<String, String> {
    if call.input.to_string().len() > 65_536 {
        return Err("dream input exceeds 64 KiB".into());
    }
    let input = &call.input;
    let action = if call.name == "dream_policy" {
        "dream"
    } else {
        text(input, "action")?
    };
    let entries = catalog(world)?;
    if action == "inspect" && input.get("id").is_none() {
        let mut results = Vec::new();
        for (_, record) in entries {
            results.push(overview(&record.id, &decode(&record)?));
        }
        return Ok(json!({"experiments":results}).to_string());
    }
    let id = text(input, "id")?;
    if id.len() > 80
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        return Err("id must use 1..80 ASCII letters, digits, dot, hyphen or underscore".into());
    }
    let found = entries.iter().find(|(_, record)| record.id == id);
    if action == "create" {
        let task: TaskInput =
            serde_json::from_value(input.get("task").cloned().ok_or("missing task")?)
                .map_err(|e| e.to_string())?;
        let task = task.into_task()?;
        validate_task(&task)?;
        if let Some((_, record)) = found {
            let state = decode(record)?;
            if serde_json::to_value(&state.task).unwrap() != serde_json::to_value(&task).unwrap() {
                return Err("experiment already exists with a different immutable task".into());
            }
            return Ok(overview(id, &state).to_string());
        }
        if entries.len() >= MAX_EXPERIMENTS {
            return Err(
                "4 experiments already exist; archive explicitly before creating another".into(),
            );
        }
        let state = State {
            format_version: 1,
            revision: 0,
            task,
            policy: BASELINE.into(),
            policy_version: 0,
            previous_policy: None,
            history: Vec::new(),
            active: None,
            pending: None,
            last_submission: None,
            last_dream: None,
            dream_calls: 0,
        };
        world.spawn(DreamExperiment {
            id: id.into(),
            state_json: serde_json::to_string(&state).unwrap(),
        });
        return Ok(overview(id, &state).to_string());
    }
    let (entity, record) = found.ok_or("unknown experiment; create first")?;
    let mut state = decode(record)?;
    if action == "inspect" {
        let mut result = overview(id, &state);
        result["task_spec"] = serde_json::to_value(&state.task).unwrap();
        if let Some(round) = input.get("round") {
            let index = round
                .as_u64()
                .filter(|n| *n > 0 && *n <= state.history.len() as u64)
                .ok_or("unknown completed round")? as usize
                - 1;
            result["tree"] = serde_json::to_value(&state.history[index]).unwrap();
        }
        return Ok(result.to_string());
    }
    if matches!(action, "dream" | "rollback" | "finish")
        && input.get("expected_revision").and_then(Value::as_u64) != Some(state.revision)
    {
        return Err(format!(
            "stale or missing expected_revision; current revision is {}",
            state.revision
        ));
    }
    let mut result = match action {
        "next" => next(&mut state)?,
        "submit" => submit(&mut state, input)?,
        "finish" => finish(&mut state)?,
        "dream" => dream(&mut state, input)?,
        "rollback" => {
            if state.active.is_some() {
                return Err("finish active round before rollback".into());
            }
            state.policy = state
                .previous_policy
                .take()
                .ok_or("no preceding policy to restore")?;
            state.policy_version += 1;
            json!({"rolled_back":true,"policy_version":state.policy_version})
        }
        _ => return Err("unknown dream action".into()),
    };
    // Inspection and retries must not cause perpetual checkpoint churn.
    if serde_json::to_string(&state).unwrap() != record.state_json {
        state.revision += 1;
        save(world, *entity, &state)?;
    }
    result["revision"] = json!(state.revision);
    Ok(result.to_string())
}
