//! Bounded, scope-local observation windows over durable interpretation results.

use bevy_ecs::{component::Component, entity::Entity, world::World};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::interpretation::{FactRecord, InterpretationResult, RelationAssertion};

const DEFAULT_WINDOW_SIZE: usize = 2;
const MAX_WINDOW_SIZE: usize = 60;
const RETRY_BACKOFF_SECS: u64 = 300;
const MAX_OBSERVATION_BYTES: usize = 16 * 1024;

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InterpretationStream {
    pub scope: String,
    pub pending_result_ids: Vec<String>,
    #[serde(default)]
    pub pending_experience_ids: Vec<String>,
    pub previous_observation_id: Option<String>,
    #[serde(default)]
    pub retry_after_epoch: u64,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InterpretationObservation {
    pub observation_id: String,
    pub scope: String,
    pub producer_system: String,
    pub epistemic_status: String,
    pub previous_observation_id: Option<String>,
    pub input_result_ids: Vec<String>,
    #[serde(default)]
    pub input_experience_ids: Vec<String>,
    #[serde(default)]
    pub input_reading_ids: Vec<String>,
    pub text: String,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct ObservationWindow {
    stream_entity: Entity,
    pub scope: String,
    pub previous_observation_id: Option<String>,
    pub input_result_ids: Vec<String>,
    pub input_experience_ids: Vec<String>,
    pub prompt: String,
}

pub fn window_size() -> usize {
    std::env::var("BIGHEAD_INTERPRETATION_WINDOW_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=MAX_WINDOW_SIZE).contains(value))
        .unwrap_or(DEFAULT_WINDOW_SIZE)
}

pub fn enqueue(world: &mut World, scope: &str, result_id: &str) {
    let existing = world
        .query::<(Entity, &InterpretationStream)>()
        .iter(world)
        .find(|(_, stream)| stream.scope == scope)
        .map(|(entity, _)| entity);
    if let Some(entity) = existing {
        world
            .get_mut::<InterpretationStream>(entity)
            .unwrap()
            .pending_result_ids
            .push(result_id.to_owned());
    } else {
        world.spawn(InterpretationStream {
            scope: scope.to_owned(),
            pending_result_ids: vec![result_id.to_owned()],
            pending_experience_ids: Vec::new(),
            previous_observation_id: None,
            retry_after_epoch: 0,
        });
    }
}

pub fn enqueue_experience(world: &mut World, scope: &str, id: &str) {
    let existing = world
        .query::<(Entity, &InterpretationStream)>()
        .iter(world)
        .find(|(_, stream)| stream.scope == scope)
        .map(|(entity, _)| entity);
    if let Some(entity) = existing {
        world
            .get_mut::<InterpretationStream>(entity)
            .unwrap()
            .pending_experience_ids
            .push(id.into());
    } else {
        world.spawn(InterpretationStream {
            scope: scope.into(),
            pending_result_ids: Vec::new(),
            pending_experience_ids: vec![id.into()],
            previous_observation_id: None,
            retry_after_epoch: 0,
        });
    }
}

pub fn next_window(
    world: &mut World,
    size: usize,
    now: u64,
) -> Result<Option<ObservationWindow>, String> {
    if !(1..=MAX_WINDOW_SIZE).contains(&size) {
        return Err("interpretation window size must be 1..60".into());
    }
    let candidate = world
        .query::<(Entity, &InterpretationStream)>()
        .iter(world)
        .filter(|(_, stream)| {
            stream.pending_result_ids.len() + stream.pending_experience_ids.len() >= size
                && stream.retry_after_epoch <= now
        })
        .min_by_key(|(_, stream)| stream.retry_after_epoch)
        .map(|(entity, stream)| (entity, stream.clone()));
    let Some((stream_entity, stream)) = candidate else {
        return Ok(None);
    };
    let input_result_ids: Vec<_> = stream
        .pending_result_ids
        .iter()
        .take(size)
        .cloned()
        .collect();
    let input_experience_ids: Vec<_> = stream
        .pending_experience_ids
        .iter()
        .take(size - input_result_ids.len())
        .cloned()
        .collect();
    let results: HashMap<_, _> = world
        .query::<&InterpretationResult>()
        .iter(world)
        .filter(|result| input_result_ids.contains(&result.result_id))
        .map(|result| (result.result_id.clone(), result.output.clone()))
        .collect();
    let fact_statuses: HashMap<_, _> = world
        .query::<&FactRecord>()
        .iter(world)
        .map(|fact| (fact.fact_id.clone(), fact.epistemic_status.clone()))
        .collect();
    let selected: HashSet<_> = input_result_ids.iter().cloned().collect();
    let mut provenance: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for relation in world.query::<&RelationAssertion>().iter(world) {
        if relation.relation_class != "provenance" || !selected.contains(&relation.result_id) {
            continue;
        }
        let Some(id) = relation.to.as_str() else {
            continue;
        };
        let source = match fact_statuses.get(id) {
            Some(status) => {
                serde_json::json!({"id":id,"kind":"fact_record","epistemic_status":status})
            }
            None => serde_json::json!({"id":id,"kind":"lens_source_entity"}),
        };
        provenance
            .entry(relation.result_id.clone())
            .or_default()
            .push(source);
    }
    let mut lines = Vec::with_capacity(size);
    for id in &input_result_ids {
        let Some(output) = results.get(id) else {
            if let Some(mut state) = world.get_mut::<InterpretationStream>(stream_entity) {
                state.retry_after_epoch = now.saturating_add(RETRY_BACKOFF_SECS);
            }
            return Err("observation window references missing interpretation result".into());
        };
        let rendered = output.to_string();
        let excerpt: String = rendered.chars().take(800).collect();
        let sources = provenance.get(id).cloned().unwrap_or_default();
        lines.push(format!(
            "result_id={id} provenance={} output={excerpt}",
            serde_json::Value::Array(sources)
        ));
    }
    let previous = stream.previous_observation_id.as_ref().and_then(|id| {
        world
            .query::<&InterpretationObservation>()
            .iter(world)
            .find(|observation| &observation.observation_id == id)
            .map(|observation| observation.text.clone())
    });
    for id in &input_experience_ids {
        let experience = world
            .query::<&crate::experience::InteractionExperience>()
            .iter(world)
            .find(|item| &item.experience_id == id);
        let Some(experience) = experience else {
            world
                .get_mut::<InterpretationStream>(stream_entity)
                .unwrap()
                .retry_after_epoch = now.saturating_add(RETRY_BACKOFF_SECS);
            return Err("observation window references missing interaction experience".into());
        };
        lines.push(experience.observation_input(12000 / size));
    }
    let prompt = format!(
        "Scope: {}\nPrevious observation id: {}\nPrevious observation (derived, not evidence): {}\n\nProvenance kind fact_record is an actual FactRecord; lens_source_entity is only an entity identity. A recorded artifact_path is not a live file-existence check. Interaction excerpts describe what speakers and tools returned, not independently verified external facts.\n\nNew inputs ({}):\n{}",
        stream.scope,
        stream.previous_observation_id.as_deref().unwrap_or("none"),
        crate::experience::excerpt(previous.as_deref().unwrap_or("none"), 2000),
        size,
        lines.join("\n")
    );
    Ok(Some(ObservationWindow {
        stream_entity,
        scope: stream.scope,
        previous_observation_id: stream.previous_observation_id,
        input_result_ids,
        input_experience_ids,
        prompt,
    }))
}

pub fn commit(
    world: &mut World,
    window: &ObservationWindow,
    text: String,
    now: u64,
) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > MAX_OBSERVATION_BYTES {
        return Err("observation text must contain 1..16384 bytes".into());
    }
    let (text, readings) = crate::experience::parse_readings(&window.input_experience_ids, &text)?;
    let stream = world
        .get::<InterpretationStream>(window.stream_entity)
        .ok_or("interpretation stream disappeared")?;
    if stream
        .pending_result_ids
        .get(..window.input_result_ids.len())
        != Some(window.input_result_ids.as_slice())
        || stream
            .pending_experience_ids
            .get(..window.input_experience_ids.len())
            != Some(window.input_experience_ids.as_slice())
        || stream.previous_observation_id != window.previous_observation_id
    {
        return Err("interpretation stream changed during observation".into());
    }
    let observation_id = uuid::Uuid::new_v4().to_string();
    let mut input_reading_ids = Vec::new();
    for reading in readings {
        let reading_id = uuid::Uuid::new_v4().to_string();
        input_reading_ids.push(reading_id.clone());
        world.spawn(crate::experience::ExperienceReading {
            reading_id,
            experience_id: reading.experience_id,
            viewpoint: reading.viewpoint,
            text: reading.text,
            unknowns: reading.unknowns,
            epistemic_status: "derived".into(),
            created_at: now,
        });
    }
    world.spawn(InterpretationObservation {
        observation_id: observation_id.clone(),
        scope: window.scope.clone(),
        producer_system: "interpretation-observer".into(),
        epistemic_status: "derived".into(),
        previous_observation_id: window.previous_observation_id.clone(),
        input_result_ids: window.input_result_ids.clone(),
        input_experience_ids: window.input_experience_ids.clone(),
        input_reading_ids,
        text,
        created_at: now,
    });
    let mut stream = world
        .get_mut::<InterpretationStream>(window.stream_entity)
        .unwrap();
    stream
        .pending_result_ids
        .drain(..window.input_result_ids.len());
    stream
        .pending_experience_ids
        .drain(..window.input_experience_ids.len());
    stream.previous_observation_id = Some(observation_id.clone());
    // Eligible immediately, but let older ready scopes take their turn first.
    stream.retry_after_epoch = now;
    Ok(observation_id)
}

pub fn defer(world: &mut World, window: &ObservationWindow, now: u64) {
    if let Some(mut stream) = world.get_mut::<InterpretationStream>(window.stream_entity) {
        stream.retry_after_epoch = now.saturating_add(RETRY_BACKOFF_SECS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(world: &mut World, scope: &str, id: &str) {
        world.spawn(InterpretationResult {
            result_id: id.into(),
            frame_id: "frame".into(),
            producer_run_id: "run".into(),
            lens_id: "lens".into(),
            lens_version: "1".into(),
            method_id: None,
            method_version: None,
            method_hash: None,
            output: serde_json::json!({"event":id}),
            epistemic_status: "derived".into(),
            created_at: 1,
        });
        enqueue(world, scope, id);
    }

    #[test]
    fn windows_continue_from_previous_observation_without_consuming_other_scopes() {
        let mut world = World::new();
        result(&mut world, "garden", "a");
        result(&mut world, "other", "x");
        assert!(next_window(&mut world, 2, 10).unwrap().is_none());
        result(&mut world, "garden", "b");
        let first = next_window(&mut world, 2, 10).unwrap().unwrap();
        assert_eq!(first.input_result_ids, ["a", "b"]);
        let first_id = commit(&mut world, &first, "Movement changed".into(), 10).unwrap();
        result(&mut world, "garden", "c");
        result(&mut world, "garden", "d");
        let second = next_window(&mut world, 2, 11).unwrap().unwrap();
        assert_eq!(
            second.previous_observation_id.as_deref(),
            Some(first_id.as_str())
        );
        assert!(second.prompt.contains("Movement changed"));
        assert_eq!(second.input_result_ids, ["c", "d"]);
        commit(&mut world, &second, "Now waiting".into(), 11).unwrap();
        let other = world
            .query::<&InterpretationStream>()
            .iter(&world)
            .find(|stream| stream.scope == "other")
            .unwrap();
        assert_eq!(other.pending_result_ids, ["x"]);
    }

    #[test]
    fn failure_keeps_window_for_retry() {
        let mut world = World::new();
        result(&mut world, "garden", "a");
        let window = next_window(&mut world, 1, 10).unwrap().unwrap();
        defer(&mut world, &window, 10);
        assert!(next_window(&mut world, 1, 11).unwrap().is_none());
        assert_eq!(
            next_window(&mut world, 1, 310)
                .unwrap()
                .unwrap()
                .input_result_ids,
            ["a"]
        );
    }

    #[test]
    fn observation_prompt_types_sources_and_does_not_verify_recorded_paths() {
        let mut world = World::new();
        result(&mut world, "garden", "reading-1");
        world.spawn(FactRecord {
            fact_id: "build-fact".into(),
            statement: "Build exited 0".into(),
            payload: json!({"exit_code":0}),
            source: "build command".into(),
            observed_at: 1,
            epistemic_status: "verified".into(),
            version: "v1".into(),
        });
        for source in ["project-entity", "build-fact"] {
            world.spawn(RelationAssertion {
                relation_id: format!("relation-{source}"),
                result_id: "reading-1".into(),
                frame_id: "frame".into(),
                from_ref: "reading-1".into(),
                predicate: "derived_from".into(),
                to: json!(source),
                relation_class: "provenance".into(),
                source_run_id: "run".into(),
                epistemic_status: "recorded".into(),
                valid_from: 1,
                supersedes: None,
            });
        }
        let prompt = next_window(&mut world, 1, 2).unwrap().unwrap().prompt;
        assert!(prompt.contains("\"kind\":\"lens_source_entity\""));
        assert!(prompt.contains("\"kind\":\"fact_record\""));
        assert!(prompt.contains("\"epistemic_status\":\"verified\""));
        assert!(prompt.contains("artifact_path is not a live file-existence check"));
    }

    #[test]
    fn previous_observation_survives_checkpoint_restore() {
        let mut runtime = crate::AgentRuntime::new_for_test();
        result(&mut runtime.world, "garden", "a");
        let first = next_window(&mut runtime.world, 1, 10).unwrap().unwrap();
        let first_id = commit(
            &mut runtime.world,
            &first,
            "A pattern may be forming".into(),
            10,
        )
        .unwrap();
        result(&mut runtime.world, "garden", "b");

        let directory =
            std::env::temp_dir().join(format!("interpretation-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let checkpoint = directory.join("checkpoint.json");
        runtime.checkpoint(&checkpoint).unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&checkpoint).unwrap();
        let next = next_window(&mut restored.world, 1, 11).unwrap().unwrap();
        assert_eq!(
            next.previous_observation_id.as_deref(),
            Some(first_id.as_str())
        );
        assert_eq!(next.input_result_ids, ["b"]);
        assert!(next.prompt.contains("A pattern may be forming"));
        let inspected = crate::interpretation::dispatch(
            &mut restored.world,
            &restored.component_registry,
            &big_head_ai::ToolCall {
                id: "inspect-observation".into(),
                name: "inspect_interpretations".into(),
                input: serde_json::json!({"observation_id":first_id}),
            },
        )
        .unwrap();
        assert_eq!(
            inspected["observation"]["input_result_ids"],
            serde_json::json!(["a"])
        );
        assert_eq!(inspected["summary"]["observations"], 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
