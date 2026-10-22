//! Policies see only revealed observations, in both online discovery and replay.
use std::collections::BTreeSet;

use rhai::{Array, Dynamic, Engine, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_NODES: usize = 32;
pub const MAX_SCRIPT_BYTES: usize = 8192;
pub const BASELINE: &str = r#"
if obs.nodes.len() == 0 || obs.nodes.len() % 3 == 0 {
    [0]
} else {
    [obs.nodes[obs.nodes.len() - 1].id]
}
"#;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub input: Value,
    pub expected: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: usize,
    pub parent: usize,
    pub script: String,
    pub passed: usize,
    pub score: f64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub batches: usize,
    pub policy_version: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayScore {
    pub best: f64,
    pub attempts: usize,
    pub batches: usize,
    pub utility: f64,
    pub visited: Vec<usize>,
    pub history_boundary: bool,
}

// A separate engine deliberately has no ECS, CLI, network or filesystem functions.
pub fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(20_000)
        .set_max_call_levels(16)
        .set_max_expr_depths(32, 16)
        .set_max_string_size(16_384)
        .set_max_array_size(256)
        .set_max_map_size(128)
        .set_module_resolver(rhai::module_resolvers::StaticModuleResolver::new());
    engine.disable_symbol("eval");
    engine.disable_symbol("import");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine
}

pub fn check_script(script: &str) -> Result<(), String> {
    if script.trim().is_empty() || script.len() > MAX_SCRIPT_BYTES {
        return Err(format!("script must contain 1..{MAX_SCRIPT_BYTES} bytes"));
    }
    Ok(())
}

pub fn evaluate(script: &str, cases: &[Case], id: usize, parent: usize) -> Node {
    let engine = engine();
    let ast = engine.compile(script);
    let mut errors = Vec::new();
    let mut passed = 0;
    for (index, case) in cases.iter().enumerate() {
        let result = (|| -> Result<Value, String> {
            let ast = ast.as_ref().map_err(ToString::to_string)?;
            let input = rhai::serde::to_dynamic(&case.input).map_err(|e| e.to_string())?;
            let mut scope = Scope::new();
            scope.push_dynamic("input", input);
            let output = engine
                .eval_ast_with_scope::<Dynamic>(&mut scope, ast)
                .map_err(|e| e.to_string())?;
            rhai::serde::from_dynamic(&output).map_err(|e| e.to_string())
        })();
        match result {
            Ok(value) if value == case.expected => passed += 1,
            Ok(value) => errors.push(format!(
                "case {index}: output differs: actual={}, expected={}",
                value.to_string().chars().take(128).collect::<String>(),
                case.expected
                    .to_string()
                    .chars()
                    .take(128)
                    .collect::<String>()
            )),
            Err(error) => errors.push(format!(
                "case {index}: {}",
                error.chars().take(256).collect::<String>()
            )),
        }
    }
    Node {
        id,
        parent,
        script: script.into(),
        passed,
        score: passed as f64 / cases.len() as f64,
        errors,
    }
}

pub fn choose(
    script: &str,
    nodes: &[Node],
    exhausted: &BTreeSet<usize>,
    batches: usize,
    budget: usize,
    workers: usize,
) -> Result<Vec<usize>, String> {
    check_script(script)?;
    if nodes.len() >= budget {
        return Ok(Vec::new());
    }
    let parents: BTreeSet<_> = nodes.iter().map(|n| n.parent).collect();
    let available: Vec<_> = std::iter::once(0)
        .chain(
            nodes
                .iter()
                .filter(|n| !parents.contains(&n.id))
                .map(|n| n.id),
        )
        .filter(|id| !exhausted.contains(id))
        .collect();
    if available.is_empty() {
        return Ok(Vec::new());
    }
    let observations: Vec<_> = nodes
        .iter()
        .map(|n| json!({"id":n.id,"parent":n.parent,"score":n.score,"passed":n.passed}))
        .collect();
    let obs = json!({
        "nodes":observations,"available":available,"batches":batches,
        "remaining":budget - nodes.len(),"workers":workers,
        "best":nodes.iter().map(|n| n.score).fold(0.0, f64::max)
    });
    let mut scope = Scope::new();
    scope.push_dynamic(
        "obs",
        rhai::serde::to_dynamic(&obs).map_err(|e| e.to_string())?,
    );
    let batch = engine()
        .eval_with_scope::<Array>(&mut scope, script)
        .map_err(|e| format!("policy: {e}"))?;
    if batch.len() > workers.min(budget - nodes.len()) {
        return Err("policy batch exceeds workers or remaining attempt budget".into());
    }
    let mut selected = Vec::new();
    for id in batch {
        let id = id
            .as_int()
            .map_err(|_| "policy must return an array of integer parent IDs")?;
        let id = usize::try_from(id).map_err(|_| "negative parent ID")?;
        if !available.contains(&id) || selected.contains(&id) {
            return Err("policy selected an unavailable or duplicate parent".into());
        }
        selected.push(id);
    }
    Ok(selected)
}

fn score(nodes: &[Node], batches: usize) -> ReplayScore {
    let best = nodes.iter().map(|n| n.score).fold(0.0, f64::max);
    let attempts = nodes.len();
    let parallelism = if batches == 0 {
        0.0
    } else {
        attempts as f64 / batches as f64
    };
    ReplayScore {
        best,
        attempts,
        batches,
        utility: best - 0.01 * attempts as f64 + 0.001 * parallelism,
        visited: nodes.iter().map(|n| n.id).collect(),
        history_boundary: false,
    }
}

pub fn replay(
    script: &str,
    tree: &Tree,
    budget: usize,
    workers: usize,
) -> Result<ReplayScore, String> {
    let mut revealed: Vec<Node> = Vec::new();
    let exhausted = BTreeSet::new();
    let mut batches = 0;
    let mut boundary = false;
    // Empty recorded continuations still consume a decision round, preventing spinning.
    for _ in 0..MAX_NODES * 2 {
        if revealed.len() == tree.nodes.len() {
            break;
        }
        let selected = choose(script, &revealed, &exhausted, batches, budget, workers)?;
        if selected.is_empty() {
            break;
        }
        batches += 1;
        let before = revealed.len();
        for parent in selected {
            let next = tree
                .nodes
                .iter()
                .find(|n| n.parent == parent && !revealed.iter().any(|seen| seen.id == n.id));
            if let Some(node) = next {
                revealed.push(node.clone());
            }
        }
        // Missing history is not an invalid online decision or a measured failure.
        if revealed.len() == before {
            boundary = true;
            break;
        }
    }
    let mut result = score(&revealed, batches);
    result.history_boundary = boundary;
    Ok(result)
}

pub fn online_score(tree: &Tree) -> ReplayScore {
    score(&tree.nodes, tree.batches)
}
