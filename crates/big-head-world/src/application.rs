//! Pure, bounded context composition. No stage can mutate the source world.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LensDefinition {
    pub id: String,
    pub version: String,
    pub purpose: String,
    #[serde(default)]
    pub usage: LensUsage,
    pub reads: Vec<String>,
    pub stage: Stage,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LensUsage {
    #[serde(default)]
    pub tasks: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stage {
    Identity,
    ArchetypeWith { component: String },
    ArchetypeWithout { component: String },
    Filter { pointer: String, equals: Value },
    Project { fields: BTreeMap<String, String> },
    Take { limit: usize },
    Sequence { stages: Vec<Stage> },
    Parallel { branches: BTreeMap<String, Stage> },
    Lens { id: String, version: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub source_id: String,
    pub source_version: String,
    pub value: Value,
    #[serde(default)]
    pub branches: Vec<String>,
    /// Captured source membership, independent of projected values. Missing means unknown.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub membership: BTreeMap<String, bool>,
}

pub type Catalog = BTreeMap<String, LensDefinition>;

#[derive(Debug, Clone, Default)]
pub struct InputScope {
    pub reads: BTreeSet<String>,
    pub presence: BTreeSet<String>,
}

const MAX_PIPELINE_BYTES: usize = 1024 * 1024;

// Count serialization bytes without allocating an equally large JSON string.
fn bounded_size(value: &impl Serialize) -> Result<usize, String> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_PIPELINE_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other(
                    "Lens intermediate data exceeds 1 MiB",
                ));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|e| e.to_string())?;
    Ok(counter.0)
}

pub fn validate(id: &str, catalog: &Catalog) -> Result<Vec<String>, String> {
    Ok(input_scope(id, catalog)?.reads.into_iter().collect())
}

pub fn input_scope(id: &str, catalog: &Catalog) -> Result<InputScope, String> {
    let mut scope = InputScope::default();
    let mut stack = Vec::new();
    let mut budget = 128;
    visit_lens(id, None, catalog, &mut stack, &mut scope, &mut budget)?;
    Ok(scope)
}

fn visit_lens(
    id: &str,
    version: Option<&str>,
    catalog: &Catalog,
    stack: &mut Vec<String>,
    scope: &mut InputScope,
    budget: &mut usize,
) -> Result<(), String> {
    if stack.iter().any(|s| s == id) || stack.len() >= 16 {
        return Err("cyclic or overly deep Lens composition".into());
    }
    let lens = catalog
        .get(id)
        .ok_or_else(|| format!("unknown Lens: {id}"))?;
    if lens.id != id
        || id.trim().is_empty()
        || lens.version.trim().is_empty()
        || lens.purpose.trim().is_empty()
    {
        return Err("Lens id, version and purpose must be nonempty".into());
    }
    if version.is_some_and(|v| v != lens.version) {
        return Err(format!("Lens version mismatch: {id}"));
    }
    if lens.reads.len() > 32 || lens.reads.iter().any(|s| s.trim().is_empty()) {
        return Err("invalid read scope".into());
    }
    for items in [&lens.usage.tasks, &lens.usage.tags, &lens.usage.limitations] {
        if items.len() > 8 || items.iter().any(|s| s.trim().is_empty() || s.len() > 256) {
            return Err("Lens usage lists allow up to 8 nonempty entries of 256 bytes".into());
        }
    }
    scope.reads.extend(lens.reads.iter().cloned());
    stack.push(id.to_owned());
    visit_stage(&lens.stage, catalog, stack, scope, budget)?;
    stack.pop();
    Ok(())
}

fn visit_stage(
    stage: &Stage,
    catalog: &Catalog,
    stack: &mut Vec<String>,
    scope: &mut InputScope,
    budget: &mut usize,
) -> Result<(), String> {
    *budget = budget.checked_sub(1).ok_or("Lens exceeds 128 stages")?;
    match stage {
        Stage::Lens { id, version } => {
            visit_lens(id, Some(version), catalog, stack, scope, budget)?
        }
        Stage::Sequence { stages } => {
            for s in stages {
                visit_stage(s, catalog, stack, scope, budget)?;
            }
        }
        Stage::Parallel { branches } => {
            for s in branches.values() {
                visit_stage(s, catalog, stack, scope, budget)?;
            }
        }
        Stage::ArchetypeWith { component } | Stage::ArchetypeWithout { component } => {
            if component.trim().is_empty() || component.trim() != component || component.len() > 256
            {
                return Err("archetype component must be a nonempty name of at most 256 bytes without surrounding whitespace".into());
            }
            scope.presence.insert(component.clone());
        }
        Stage::Filter { pointer, .. } => check_pointer(pointer)?,
        Stage::Project { fields } => {
            if fields.len() > 64 {
                return Err("projection exceeds 64 fields".into());
            }
            for p in fields.values() {
                check_pointer(p)?;
            }
        }
        Stage::Take { limit } if *limit > 256 => return Err("take limit exceeds 256".into()),
        _ => (),
    }
    Ok(())
}

fn check_pointer(pointer: &str) -> Result<(), String> {
    if pointer.is_empty() || pointer.starts_with('/') {
        Ok(())
    } else {
        Err("field paths must be JSON pointers".into())
    }
}

pub fn run(id: &str, catalog: &Catalog, facts: Vec<Fact>) -> Result<Vec<Fact>, String> {
    validate(id, catalog)?;
    if facts.len() > 256 {
        return Err("input exceeds 256 facts; narrow the Lens read scope".into());
    }
    bounded_size(&facts)?;
    evaluate(&catalog[id].stage, catalog, facts)
}

fn evaluate(stage: &Stage, catalog: &Catalog, facts: Vec<Fact>) -> Result<Vec<Fact>, String> {
    let result = match stage {
        Stage::Identity => facts,
        Stage::ArchetypeWith { component } | Stage::ArchetypeWithout { component } => {
            let expected = matches!(stage, Stage::ArchetypeWith { .. });
            let mut selected = Vec::new();
            for fact in facts {
                let present = fact.membership.get(component).ok_or_else(|| {
                    format!("source membership not captured for component: {component}")
                })?;
                if *present == expected {
                    selected.push(fact);
                }
            }
            selected
        }
        Stage::Filter { pointer, equals } => facts
            .into_iter()
            .filter(|f| f.value.pointer(pointer) == Some(equals))
            .collect(),
        Stage::Project { fields } => {
            let mut projected = Vec::new();
            let mut bytes = 0;
            for mut fact in facts {
                let refs: BTreeMap<_, _> = fields
                    .iter()
                    .map(|(key, pointer)| {
                        (key, fact.value.pointer(pointer).unwrap_or(&Value::Null))
                    })
                    .collect();
                bytes += bounded_size(&refs)?;
                if bytes > MAX_PIPELINE_BYTES {
                    return Err("Lens intermediate data exceeds 1 MiB".into());
                }
                fact.value = Value::Object(
                    refs.into_iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                );
                projected.push(fact);
            }
            projected
        }
        Stage::Take { limit } => facts.into_iter().take(*limit).collect(),
        Stage::Sequence { stages } => {
            let mut current = facts;
            for s in stages {
                current = evaluate(s, catalog, current)?;
            }
            current
        }
        Stage::Parallel { branches } => {
            let mut output = Vec::new();
            for (name, s) in branches {
                let mut branch = evaluate(s, catalog, facts.clone())?;
                for f in &mut branch {
                    f.branches.push(name.clone());
                }
                output.extend(branch);
                if output.len() > 256 {
                    return Err("parallel output exceeds 256 facts".into());
                }
                bounded_size(&output)?;
            }
            output
        }
        Stage::Lens { id, .. } => evaluate(&catalog[id].stage, catalog, facts)?,
    };
    bounded_size(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn member_fact(id: &str, review: bool, completed: bool) -> Fact {
        Fact {
            source_id: id.into(),
            source_version: "v1".into(),
            value: json!({"WorkItem":{"title":id}}),
            branches: vec![],
            membership: BTreeMap::from([
                ("NeedsReview".into(), review),
                ("Completed".into(), completed),
            ]),
        }
    }

    #[test]
    fn archetype_selection_survives_projection_and_composition() {
        let selector: Stage = serde_json::from_value(json!({
            "kind":"sequence", "stages":[
                {"kind":"archetype_with","component":"NeedsReview"},
                {"kind":"archetype_without","component":"Completed"}
            ]
        }))
        .unwrap();
        let app = definition(
            "app",
            Stage::Sequence {
                stages: vec![
                    Stage::Project {
                        fields: BTreeMap::from([("title".into(), "/WorkItem/title".into())]),
                    },
                    Stage::Lens {
                        id: "selector".into(),
                        version: "1".into(),
                    },
                ],
            },
        );
        let catalog = Catalog::from([
            ("app".into(), app),
            ("selector".into(), definition("selector", selector)),
        ]);
        let scope = input_scope("app", &catalog).unwrap();
        assert_eq!(scope.reads, BTreeSet::from(["WorkItem".into()]));
        assert_eq!(
            scope.presence,
            BTreeSet::from(["Completed".into(), "NeedsReview".into()])
        );
        let output = run(
            "app",
            &catalog,
            vec![
                member_fact("todo", true, false),
                member_fact("done", true, true),
                member_fact("other", false, false),
            ],
        )
        .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].value, json!({"title":"todo"}));
        assert_eq!(output[0].source_id, "todo");
        assert_eq!(output[0].membership["NeedsReview"], true);
    }

    #[test]
    fn archetype_branches_are_independent_and_missing_membership_is_not_absence() {
        let catalog = Catalog::from([(
            "app".into(),
            definition(
                "app",
                Stage::Parallel {
                    branches: BTreeMap::from([
                        (
                            "active".into(),
                            Stage::ArchetypeWithout {
                                component: "Completed".into(),
                            },
                        ),
                        (
                            "done".into(),
                            Stage::ArchetypeWith {
                                component: "Completed".into(),
                            },
                        ),
                    ]),
                },
            ),
        )]);
        let facts = vec![
            member_fact("todo", true, false),
            member_fact("done", true, true),
        ];
        let output = run("app", &catalog, facts).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].branches, ["active"]);
        assert_eq!(output[1].branches, ["done"]);
        let old: Fact =
            serde_json::from_value(json!({"source_id":"old","source_version":"1","value":{}}))
                .unwrap();
        assert!(
            run("app", &catalog, vec![old])
                .unwrap_err()
                .contains("membership not captured")
        );
    }

    #[test]
    fn archetype_validation_rejects_bad_names_and_counts_stage_budget() {
        for name in ["", " ", " Completed", &"x".repeat(257)] {
            let catalog = Catalog::from([(
                "app".into(),
                definition(
                    "app",
                    Stage::ArchetypeWith {
                        component: name.into(),
                    },
                ),
            )]);
            assert!(validate("app", &catalog).is_err());
        }
        let catalog = Catalog::from([(
            "app".into(),
            definition(
                "app",
                Stage::Sequence {
                    stages: vec![
                        Stage::ArchetypeWithout {
                            component: "Completed".into()
                        };
                        128
                    ],
                },
            ),
        )]);
        assert!(
            validate("app", &catalog)
                .unwrap_err()
                .contains("128 stages")
        );
    }

    #[test]
    fn projection_amplification_is_bounded_before_cloning() {
        let lens = definition(
            "large",
            Stage::Project {
                fields: (0..64)
                    .map(|i| (format!("copy{i}"), "/payload".into()))
                    .collect(),
            },
        );
        let catalog = Catalog::from([("large".into(), lens)]);
        let facts = vec![Fact {
            source_id: "source".into(),
            source_version: "1".into(),
            branches: vec![],
            membership: BTreeMap::new(),
            value: json!({"payload":"x".repeat(32768)}),
        }];
        assert!(
            run("large", &catalog, facts)
                .unwrap_err()
                .contains("exceeds 1 MiB")
        );
    }
    fn definition(id: &str, stage: Stage) -> LensDefinition {
        LensDefinition {
            id: id.into(),
            version: "1".into(),
            purpose: "test".into(),
            usage: LensUsage::default(),
            reads: vec!["WorkItem".into()],
            stage,
        }
    }
    #[test]
    fn composition_preserves_evidence_and_branch_identity() {
        let select = definition(
            "select",
            Stage::Filter {
                pointer: "/status".into(),
                equals: json!("blocked"),
            },
        );
        let app = definition(
            "app",
            Stage::Sequence {
                stages: vec![
                    Stage::Lens {
                        id: "select".into(),
                        version: "1".into(),
                    },
                    Stage::Parallel {
                        branches: BTreeMap::from([
                            ("detail".into(), Stage::Identity),
                            (
                                "title".into(),
                                Stage::Project {
                                    fields: BTreeMap::from([("title".into(), "/title".into())]),
                                },
                            ),
                        ]),
                    },
                ],
            },
        );
        let catalog = BTreeMap::from([("select".into(), select), ("app".into(), app)]);
        let output = run(
            "app",
            &catalog,
            vec![Fact {
                source_id: "entity-a".into(),
                source_version: "v1".into(),
                value: json!({"status":"blocked","title":"Weather"}),
                branches: vec![],
                membership: BTreeMap::new(),
            }],
        )
        .unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[1].value, json!({"title":"Weather"}));
        assert_eq!(output[1].source_version, "v1");
        assert_eq!(output[1].branches, vec!["title"]);
    }
    #[test]
    fn rejects_cycles_and_stale_dependencies() {
        let mut catalog = BTreeMap::from([(
            "a".into(),
            definition(
                "a",
                Stage::Lens {
                    id: "a".into(),
                    version: "1".into(),
                },
            ),
        )]);
        assert!(validate("a", &catalog).is_err());
        catalog.insert(
            "a".into(),
            definition(
                "a",
                Stage::Lens {
                    id: "b".into(),
                    version: "2".into(),
                },
            ),
        );
        catalog.insert("b".into(), definition("b", Stage::Identity));
        assert!(
            validate("a", &catalog)
                .unwrap_err()
                .contains("version mismatch")
        );
    }
}
