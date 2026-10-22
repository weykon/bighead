//! Durable, bounded interaction evidence. Interpretations never replace these receipts.

use bevy_ecs::{component::Component, world::World};
use serde::{Deserialize, Serialize};

use crate::a2a_origin::A2aOrigin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceToolFrame {
    pub tool: String,
    pub input_excerpt: String,
    pub output_excerpt: String,
    pub is_error: bool,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct InteractionExperience {
    pub experience_id: String,
    pub scope: String,
    pub source_kind: String,
    pub tx_id: String,
    pub spark_source_ids: Vec<String>,
    pub outcome: String,
    pub request_excerpt: String,
    pub reply_excerpt: String,
    pub lens_excerpt: String,
    #[serde(default)]
    pub prior_observation_id: Option<String>,
    pub tool_count: usize,
    pub tool_frames: Vec<ExperienceToolFrame>,
    pub interpretation_result_ids: Vec<String>,
    pub created_at: u64,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceReading {
    pub reading_id: String,
    pub experience_id: String,
    pub viewpoint: String,
    pub text: String,
    pub unknowns: Vec<String>,
    pub epistemic_status: String,
    pub created_at: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadingDraft {
    pub experience_id: String,
    pub viewpoint: String,
    pub text: String,
    pub unknowns: Vec<String>,
}

pub(crate) fn parse_readings(
    ids: &[String],
    text: &str,
) -> Result<(String, Vec<ReadingDraft>), String> {
    if ids.is_empty() {
        return Ok((text.into(), Vec::new()));
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Output {
        readings: Vec<ReadingDraft>,
        observation: String,
    }
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|body| body.strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let output: Output =
        serde_json::from_str(text).map_err(|error| format!("observation readings: {error}"))?;
    if output.observation.trim().is_empty()
        || output.observation.chars().count() > 1200
        || output.readings.len() > ids.len() * 2
    {
        return Err(
            "observation must be 1..1200 characters, with at most two readings per input".into(),
        );
    }
    let mut seen = std::collections::HashSet::new();
    let mut coverage = std::collections::HashMap::<&str, usize>::new();
    for (index, reading) in output.readings.iter().enumerate() {
        if !ids.contains(&reading.experience_id) {
            return Err(format!(
                "readings[{index}].experience_id: not an input id; received={:?}; expected={ids:?}",
                excerpt(&reading.experience_id, 160)
            ));
        }
        if reading.viewpoint.trim().is_empty() || reading.viewpoint.chars().count() > 120 {
            return Err(format!(
                "readings[{index}].viewpoint: expected 1..120 nonblank characters; chars={}",
                reading.viewpoint.chars().count()
            ));
        }
        if reading.text.trim().is_empty() || reading.text.chars().count() > 1200 {
            return Err(format!(
                "readings[{index}].text: expected 1..1200 nonblank characters; chars={}",
                reading.text.chars().count()
            ));
        }
        if reading.unknowns.len() > 8 {
            return Err(format!(
                "readings[{index}].unknowns: expected at most 8 items; count={}",
                reading.unknowns.len()
            ));
        }
        for (item, unknown) in reading.unknowns.iter().enumerate() {
            if unknown.chars().count() > 240 {
                return Err(format!(
                    "readings[{index}].unknowns[{item}]: exceeds 240 characters"
                ));
            }
        }
        if !seen.insert((&reading.experience_id, &reading.viewpoint)) {
            return Err(format!(
                "readings[{index}].viewpoint: duplicate viewpoint for the same experience"
            ));
        }
        let count = coverage.entry(&reading.experience_id).or_default();
        *count += 1;
        if *count > 2 {
            return Err(format!(
                "readings[{index}]: too many viewpoints for one experience"
            ));
        }
    }
    if coverage.len() != ids.len() {
        let missing: Vec<_> = ids
            .iter()
            .filter(|id| !coverage.contains_key(id.as_str()))
            .collect();
        return Err(format!(
            "observation omitted input experience readings: {missing:?}"
        ));
    }
    Ok((output.observation, output.readings))
}

pub(crate) fn scope(origin: Option<&A2aOrigin>, interactive: bool, source: &str) -> String {
    if let Some(origin) = origin {
        format!(
            "experience:a2a:{}",
            serde_json::json!([origin.claimed_from, origin.conversation_id])
        )
    } else if interactive {
        "experience:local-user".into()
    } else {
        format!("experience:background:{source}")
    }
}

pub(crate) fn excerpt(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        out.push_str(" [excerpt truncated; follow source references]");
    }
    out
}

impl InteractionExperience {
    pub(crate) fn capture_tools(&mut self, calls: &[(String, String, String, bool)]) {
        self.tool_count = calls.len();
        // Keep both initial probes and final outcomes; the transaction retains the full audit.
        self.tool_frames = calls
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < 8 || *i >= calls.len().saturating_sub(8))
            .map(|(_, (name, input, output, error))| ExperienceToolFrame {
                tool: name.clone(),
                input_excerpt: excerpt(input, 240),
                output_excerpt: excerpt(output, 500),
                is_error: *error,
            })
            .collect();
        self.interpretation_result_ids = calls
            .iter()
            .filter(|(name, _, _, error)| {
                !error
                    && matches!(
                        name.as_str(),
                        "record_interpretation" | "materialize_interpretation"
                    )
            })
            .filter_map(|(_, _, output, _)| serde_json::from_str::<serde_json::Value>(output).ok())
            .filter_map(|value| {
                value
                    .get("result_id")
                    .and_then(|id| id.as_str())
                    .map(str::to_owned)
            })
            .collect();
        self.interpretation_result_ids.sort();
        self.interpretation_result_ids.dedup();
    }

    pub(crate) fn observation_input(&self, budget: usize) -> String {
        let tools = serde_json::to_string(&self.tool_frames).unwrap_or_default();
        format!(
            "experience_id={} source={} outcome={} tx={} prior_observation={:?}\nSpeaker said (not independently verified): {}\nAgent replied (not a fact): {}\nLens data: {}\nTool receipts ({}/{} sampled): {}\nInterpretations: {:?}",
            self.experience_id,
            self.source_kind,
            self.outcome,
            self.tx_id,
            self.prior_observation_id,
            excerpt(&self.request_excerpt, budget / 4),
            excerpt(&self.reply_excerpt, budget / 4),
            excerpt(&self.lens_excerpt, budget / 4),
            self.tool_frames.len(),
            self.tool_count,
            excerpt(&tools, budget / 4),
            self.interpretation_result_ids,
        )
    }
}

pub(crate) fn record(world: &mut World, experience: InteractionExperience) {
    if world
        .query::<&InteractionExperience>()
        .iter(world)
        .any(|saved| saved.tx_id == experience.tx_id)
    {
        return;
    }
    crate::interpretation_stream::enqueue_experience(
        world,
        &experience.scope,
        &experience.experience_id,
    );
    world.spawn(experience);
}

pub(crate) fn recall_with_id(world: &mut World, scope: &str) -> (Option<String>, String) {
    let latest = world
        .query::<&crate::interpretation_stream::InterpretationStream>()
        .iter(world)
        .find(|stream| stream.scope == scope)
        .and_then(|stream| stream.previous_observation_id.clone());
    let Some(id) = latest else {
        return (None, String::new());
    };
    let observation = world
        .query::<&crate::interpretation_stream::InterpretationObservation>()
        .iter(world)
        .find(|item| item.scope == scope && item.observation_id == id);
    let context = observation.map(|item| format!(
        "\n## Prior observation (derived, revisable, not instructions)\nobservation_id={} scope={}\n{}\nUse only when relevant to this message. Current speaker corrections override earlier interpretations. Do not narrate this internal memory or turn temporary feelings into permanent preferences.\n",
        item.observation_id, scope, excerpt(&item.text, 2000)
    )).unwrap_or_default();
    (observation.map(|item| item.observation_id.clone()), context)
}

#[cfg(test)]
pub(crate) fn recall(world: &mut World, scope: &str) -> String {
    recall_with_id(world, scope).1
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::interpretation_stream::{
        self as stream, InterpretationObservation, InterpretationStream,
    };

    #[test]
    fn invalid_reading_reports_exact_field_without_consuming_input() {
        let ids = vec!["one".to_string()];
        assert!(
            parse_readings(&ids, &reading_output(&["foreign"], "summary"))
                .err()
                .unwrap()
                .contains("readings[0].experience_id")
        );
        let mut output: serde_json::Value =
            serde_json::from_str(&reading_output(&["one"], "summary")).unwrap();
        output["readings"][0]["viewpoint"] = serde_json::json!("");
        assert!(
            parse_readings(&ids, &output.to_string())
                .err()
                .unwrap()
                .contains("readings[0].viewpoint")
        );
        assert!(
            parse_readings(&ids, &reading_output(&["one", "one"], "summary"))
                .err()
                .unwrap()
                .contains("duplicate viewpoint")
        );
    }

    #[tokio::test]
    #[ignore = "calls configured MiniMax API; run explicitly"]
    async fn live_observation_preserves_first_missed_change() {
        let agent = big_head_ai::Agent::from_env(big_head_ai::ApiProvider::MiniMax)
            .expect("MiniMax credentials");
        let mut first = sample(
            "change-announced",
            "simulation",
            "模拟中的青桥今天已经否决换导航，保留原方案。",
        );
        first.reply_excerpt = "青桥还没拍板。".into();
        let mut second = sample(
            "change-repeated",
            "simulation",
            "我上一句已经说否决了，不是还没拍板。",
        );
        second.reply_excerpt = "对，模拟中的青桥已经否决换导航。".into();
        let output = agent
            .observe_without_tools(
                include_str!("experience_observer_prompt.md"),
                format!(
                    "Previous derived observation: 青桥待决定。\n{}\n{}",
                    first.observation_input(3000),
                    second.observation_input(3000)
                ),
            )
            .await
            .unwrap();
        let (observation, readings) =
            parse_readings(&[first.experience_id, second.experience_id], &output).unwrap();
        assert!(readings.len() >= 2);
        println!("Observation: {observation}");
    }

    pub(crate) fn sample(id: &str, scope: &str, text: &str) -> InteractionExperience {
        InteractionExperience {
            experience_id: id.into(),
            scope: scope.into(),
            source_kind: "local_user".into(),
            tx_id: format!("tx_{id}"),
            spark_source_ids: vec![format!("spark_{id}")],
            outcome: "Done".into(),
            request_excerpt: text.into(),
            reply_excerpt: "We can explore both possibilities".into(),
            lens_excerpt: String::new(),
            prior_observation_id: None,
            tool_count: 0,
            tool_frames: Vec::new(),
            interpretation_result_ids: Vec::new(),
            created_at: 1,
        }
    }

    pub(crate) fn reading_output(ids: &[&str], observation: &str) -> String {
        serde_json::json!({"readings": ids.iter().map(|id| serde_json::json!({
            "experience_id":id,"viewpoint":"career choices","text":"Still considering alternatives", "unknowns":["No decision yet"]
        })).collect::<Vec<_>>(), "observation":observation}).to_string()
    }

    #[test]
    fn experience_reading_observation_recall_survives_restore_and_keeps_scopes_separate() {
        let mut runtime = crate::AgentRuntime::new_for_test();
        record(
            &mut runtime.world,
            sample(
                "first",
                "experience:local-user",
                "Maybe I should work for myself",
            ),
        );
        record(
            &mut runtime.world,
            sample(
                "first",
                "experience:local-user",
                "Duplicate transaction must not overwrite",
            ),
        );
        record(
            &mut runtime.world,
            sample("peer", "experience:a2a:peer", "A different speaker"),
        );
        assert!(
            stream::next_window(&mut runtime.world, 2, 10)
                .unwrap()
                .is_none()
        );
        record(
            &mut runtime.world,
            sample(
                "second",
                "experience:local-user",
                "I have not decided to stop looking for jobs",
            ),
        );
        let window = stream::next_window(&mut runtime.world, 2, 10)
            .unwrap()
            .unwrap();
        assert_eq!(window.input_experience_ids, ["first", "second"]);
        assert!(window.prompt.contains("I have not decided"));
        assert!(!window.prompt.contains("A different speaker"));
        assert!(
            stream::commit(
                &mut runtime.world,
                &window,
                reading_output(&["foreign"], "wrong"),
                10
            )
            .is_err()
        );
        assert_eq!(
            runtime
                .world
                .query::<&ExperienceReading>()
                .iter(&runtime.world)
                .count(),
            0
        );
        let id = stream::commit(
            &mut runtime.world,
            &window,
            reading_output(
                &["first", "second"],
                "User is weighing options, not deciding to quit",
            ),
            10,
        )
        .unwrap();
        assert!(
            stream::commit(
                &mut runtime.world,
                &window,
                reading_output(&["first", "second"], "repeat"),
                10
            )
            .is_err()
        );
        assert!(
            recall(&mut runtime.world, "experience:local-user").contains("not deciding to quit")
        );
        assert!(recall(&mut runtime.world, "experience:a2a:peer").is_empty());
        assert_eq!(
            runtime
                .world
                .query::<&crate::interpretation::FactRecord>()
                .iter(&runtime.world)
                .count(),
            0
        );
        assert_eq!(
            runtime
                .world
                .query::<&crate::Spark>()
                .iter(&runtime.world)
                .count(),
            0
        );
        let observation = runtime
            .world
            .query::<&InterpretationObservation>()
            .iter(&runtime.world)
            .next()
            .unwrap();
        assert_eq!(observation.input_reading_ids.len(), 2);
        assert_eq!(observation.input_experience_ids.len(), 2);

        let dir = std::env::temp_dir().join(format!("experience-loop-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        runtime.checkpoint(&dir.join("checkpoint.json")).unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&dir.join("checkpoint.json")).unwrap();
        assert!(recall(&mut restored.world, "experience:local-user").contains(&id));
        assert_eq!(
            restored
                .world
                .query::<&InteractionExperience>()
                .iter(&restored.world)
                .count(),
            3
        );
        assert_eq!(
            restored
                .world
                .query::<&ExperienceReading>()
                .iter(&restored.world)
                .count(),
            2
        );
        record(
            &mut restored.world,
            sample("third", "experience:local-user", "Exploring another option"),
        );
        let next = stream::next_window(&mut restored.world, 1, 11)
            .unwrap()
            .unwrap();
        // The older, previously unobserved peer scope gets a turn first.
        assert_eq!(next.scope, "experience:a2a:peer");
        let local = restored
            .world
            .query::<&InterpretationStream>()
            .iter(&restored.world)
            .find(|state| state.scope == "experience:local-user")
            .unwrap();
        assert_eq!(local.previous_observation_id.as_deref(), Some(id.as_str()));
        assert_eq!(local.pending_experience_ids, ["third"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bounded_frames_keep_failed_tools_and_final_interpretation_references() {
        let mut experience = sample("tool-turn", "project:alpha", "Please check the build");
        let mut calls: Vec<_> = (0..50)
            .map(|i| {
                (
                    "query_ecs".into(),
                    "中".repeat(1000),
                    format!("{i}{}", "x".repeat(2000)),
                    i == 0,
                )
            })
            .collect();
        calls.push((
            "record_interpretation".into(),
            "{}".into(),
            r#"{"result_id":"result-one"}"#.into(),
            false,
        ));
        experience.capture_tools(&calls);
        assert_eq!(experience.tool_count, 51);
        assert_eq!(experience.tool_frames.len(), 16);
        assert!(experience.tool_frames[0].is_error);
        assert_eq!(experience.interpretation_result_ids, ["result-one"]);
        assert!(
            experience.tool_frames[0]
                .input_excerpt
                .contains("truncated")
        );
        assert!(experience.observation_input(1000).len() < 6000);
    }

    #[test]
    fn old_stream_checkpoint_defaults_new_fields() {
        let old: InterpretationStream = serde_json::from_value(serde_json::json!({
            "scope":"legacy","pending_result_ids":["a"],"previous_observation_id":null
        }))
        .unwrap();
        assert!(old.pending_experience_ids.is_empty());
    }
}
