//! The model sees only a detached window. ECS commits remain on the runtime thread.

use crate::{audit::TransactionContext, interpretation_stream::ObservationWindow};
use big_head_ai::Agent;

pub(crate) struct ObservationJob {
    pub window: ObservationWindow,
    pub tx: TransactionContext,
    pub handle: tokio::task::JoinHandle<Result<ObservationOutput, String>>,
}

#[derive(Debug)]
pub(crate) struct ObservationOutput {
    pub text: String,
    pub repair_diagnostic: Option<String>,
}

fn validate(ids: &[String], text: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.len() > 16384 {
        return Err("observation text must contain 1..16384 bytes".into());
    }
    crate::experience::parse_readings(ids, text).map(|_| ())
}

impl Drop for ObservationJob {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn observe_validated<F, Fut>(
    ids: &[String],
    input: String,
    mut call: F,
) -> Result<ObservationOutput, String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let text = call(input.clone()).await?;
    let Err(error) = validate(ids, &text) else {
        return Ok(ObservationOutput {
            text,
            repair_diagnostic: None,
        });
    };
    let diagnostic = format!(
        "{error}; rejected_output_excerpt={}",
        crate::experience::excerpt(&text, 2000)
    );
    let repair_input = serde_json::json!({
        "original_evidence": input,
        "invalid_output": crate::experience::excerpt(&text, 16384),
        "validation_error": error,
        "request": "Repair the output format once, using only original evidence. Include every required field and exact input id. Do not invent facts or treat invalid output as evidence."
    }).to_string();
    let text = call(repair_input)
        .await
        .map_err(|e| format!("{diagnostic}; repair failed: {e}"))?;
    validate(ids, &text).map_err(|e| {
        format!(
            "{diagnostic}; repair rejected: {e}; repair_excerpt={}",
            crate::experience::excerpt(&text, 2000)
        )
    })?;
    Ok(ObservationOutput {
        text,
        repair_diagnostic: Some(diagnostic),
    })
}

pub(crate) fn start(
    agent: &Agent,
    window: ObservationWindow,
    tx: TransactionContext,
    prompt: String,
) -> ObservationJob {
    let agent = agent.background_copy();
    let input = window.prompt.clone();
    let ids = window.input_experience_ids.clone();
    let handle = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(75),
            observe_validated(&ids, input, |input| {
                agent.observe_without_tools(&prompt, input)
            }),
        )
        .await
        .map_err(|_| "Background observation timed out".to_string())?
    });
    ObservationJob { window, tx, handle }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_repair_preserves_evidence_and_failure_diagnostics() {
        let ids = vec!["one".into()];
        for repaired in [true, false] {
            let mut inputs = Vec::new();
            let result = observe_validated(&ids, "original speaker evidence".into(), |input| {
                inputs.push(input);
                std::future::ready(Ok(if inputs.len() == 2 && repaired {
                    crate::experience::tests::reading_output(&["one"], "summary")
                } else {
                    "{}".into()
                }))
            })
            .await;
            assert_eq!(inputs.len(), 2);
            assert!(inputs[1].contains("original speaker evidence"));
            assert!(inputs[1].contains("validation_error"));
            if repaired {
                assert!(
                    result
                        .unwrap()
                        .repair_diagnostic
                        .unwrap()
                        .contains("missing field")
                );
            } else {
                assert!(result.unwrap_err().contains("repair rejected"));
            }
        }
    }

    #[test]
    fn missing_text_and_foreign_sources_are_rejected_before_commit() {
        let ids = vec!["one".into()];
        assert!(validate(&ids, r#"{"readings":[{"experience_id":"one","viewpoint":"scope","unknowns":[]}],"observation":"summary"}"#).unwrap_err().contains("text"));
        assert!(
            validate(
                &ids,
                &crate::experience::tests::reading_output(&["foreign"], "summary")
            )
            .is_err()
        );
        assert!(
            validate(
                &ids,
                &crate::experience::tests::reading_output(&["one"], "summary")
            )
            .is_ok()
        );
    }
}
