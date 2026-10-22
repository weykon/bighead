//! Compare Lens alternatives over one captured world input, without publishing them.
use bevy_ecs::world::World;
use big_head_ecs::{ComponentActionRegistry, lens::LensCandidate};
use big_head_world::application::{self, Fact, InputScope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn one() -> usize {
    1
}
fn context_bytes() -> usize {
    2048
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checks {
    #[serde(default = "one")]
    pub min_facts: usize,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default = "context_bytes")]
    pub max_output_bytes: usize,
}

impl Default for Checks {
    fn default() -> Self {
        Self {
            min_facts: 1,
            required_fields: vec![],
            max_output_bytes: 2048,
        }
    }
}

impl Checks {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=256).contains(&self.min_facts)
            || !(1..=65536).contains(&self.max_output_bytes)
            || self.required_fields.len() > 16
            || self
                .required_fields
                .iter()
                .any(|p| p.len() > 256 || (!p.is_empty() && !p.starts_with('/')))
        {
            return Err("invalid trial checks: min_facts=1..256, max_output_bytes=1..65536, up to 16 JSON pointers".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Trial {
    pub id: String,
    pub candidate_revision: u64,
    pub observed_at: u64,
    pub passed: bool,
    pub error: Option<String>,
    pub evidence_key: String,
    pub checks: Checks,
    pub inputs: Value,
    pub candidate_metrics: Value,
    pub baseline_metrics: Value,
    pub baseline_error: Option<String>,
    pub output: Vec<Fact>,
}

impl Trial {
    pub fn failed(revision: u64, checks: Checks, error: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            candidate_revision: revision,
            observed_at: big_head_ecs::execution::now_secs(),
            passed: false,
            error: Some(error.chars().take(512).collect()),
            evidence_key: String::new(),
            checks,
            inputs: json!([]),
            candidate_metrics: Value::Null,
            baseline_metrics: Value::Null,
            baseline_error: None,
            output: vec![],
        }
    }
    pub fn summary(&self) -> Value {
        json!({"trial_id":self.id,"candidate_revision":self.candidate_revision,"passed":self.passed,"error":self.error,
            "candidate_metrics":self.candidate_metrics,"baseline_metrics":self.baseline_metrics,"baseline_error":self.baseline_error,
            "observed_at":self.observed_at,"meaning":"Structural trial only, not proof of semantic quality or source freshness"})
    }
}

pub(super) fn fingerprint(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    Ok(format!("{:016x}", crate::observation::stable_hash(&bytes)))
}

fn scoped(facts: &[Fact], scope: &InputScope) -> Vec<Fact> {
    facts
        .iter()
        .filter_map(|f| {
            let values: serde_json::Map<_, _> = scope
                .reads
                .iter()
                .filter_map(|name| f.value.get(name).map(|v| (name.clone(), v.clone())))
                .collect();
            if values.is_empty() {
                return None;
            }
            let value = Value::Object(values);
            let membership = f
                .membership
                .iter()
                .filter(|(name, _)| scope.presence.contains(*name))
                .map(|(name, present)| (name.clone(), *present))
                .collect::<std::collections::BTreeMap<_, _>>();
            Some(Fact {
                source_id: f.source_id.clone(),
                source_version: fingerprint(&(&value, &membership)).unwrap(),
                value,
                branches: vec![],
                membership,
            })
        })
        .collect()
}

fn metrics(output: &[Fact]) -> Value {
    fn nulls(v: &Value) -> usize {
        match v {
            Value::Null => 1,
            Value::Array(a) => a.iter().map(nulls).sum(),
            Value::Object(m) => m.values().map(nulls).sum(),
            _ => 0,
        }
    }
    json!({"facts":output.len(),"output_bytes":serde_json::to_vec(output).unwrap().len(),"null_values":output.iter().map(|f|nulls(&f.value)).sum::<usize>()})
}

pub(super) fn run(
    world: &mut World,
    registry: &ComponentActionRegistry,
    candidate: &LensCandidate,
    checks: Checks,
) -> Result<Trial, String> {
    checks.validate()?;
    let mut catalog = crate::lens_runtime::definition_catalog(world)?;
    let original_catalog_key = fingerprint(&catalog)?;
    if catalog.contains_key(&candidate.id) {
        return Err("candidate id already exists in published Lens catalog".into());
    }
    let definition: application::LensDefinition =
        serde_json::from_str(&candidate.definition_json).map_err(|e| e.to_string())?;
    if definition.id != candidate.id {
        return Err("candidate identity mismatch".into());
    }
    let base_scope = match (&candidate.base_id, &candidate.base_version) {
        (Some(id), Some(version)) => {
            if catalog.get(id).is_none_or(|d| &d.version != version) {
                return Err("baseline version changed or disappeared".into());
            }
            application::input_scope(id, &catalog)?
        }
        (None, None) => InputScope::default(),
        _ => return Err("invalid baseline reference".into()),
    };
    catalog.insert(candidate.id.clone(), definition);
    let scope = application::input_scope(&candidate.id, &catalog)?;
    let union = InputScope {
        reads: scope.reads.union(&base_scope.reads).cloned().collect(),
        presence: scope
            .presence
            .union(&base_scope.presence)
            .cloned()
            .collect(),
    };
    let facts = crate::lens_runtime::collect_facts(world, registry, &union)?;
    let evidence_key = fingerprint(&(original_catalog_key, &candidate.definition_json, &facts))?;
    let inputs = json!(
        facts
            .iter()
            .map(|f| json!({"source_id":f.source_id,"version":f.source_version}))
            .collect::<Vec<_>>()
    );
    let (baseline_metrics, baseline_error) = if let Some(id) = &candidate.base_id {
        match application::run(id, &catalog, scoped(&facts, &base_scope)) {
            Ok(output) => (metrics(&output), None),
            Err(e) => (Value::Null, Some(e)),
        }
    } else {
        (Value::Null, None)
    };
    let output = application::run(&candidate.id, &catalog, scoped(&facts, &scope))?;
    let candidate_metrics = metrics(&output);
    let error = if output.len() < checks.min_facts {
        Some("too few output facts".into())
    } else if candidate_metrics["output_bytes"].as_u64().unwrap() > checks.max_output_bytes as u64 {
        Some("trial output byte budget exceeded".into())
    } else if output.iter().any(|f| {
        checks
            .required_fields
            .iter()
            .any(|p| f.value.pointer(p).is_none_or(Value::is_null))
    }) {
        Some("required output field missing or null".into())
    } else {
        None
    };
    Ok(Trial {
        id: uuid::Uuid::new_v4().to_string(),
        candidate_revision: candidate.revision,
        observed_at: big_head_ecs::execution::now_secs(),
        passed: error.is_none(),
        error,
        evidence_key,
        checks,
        inputs,
        candidate_metrics,
        baseline_metrics,
        baseline_error,
        output,
    })
}
