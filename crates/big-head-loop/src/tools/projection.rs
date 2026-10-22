//! Store oversized context and record its provenance without interpreting it.

use super::RuntimeToolExecutor;
use crate::audit;
use big_head_ai::{ContextProjectionRequest, ContextProjectionResult};

impl RuntimeToolExecutor<'_> {
    pub(super) fn project_context(
        &mut self,
        request: ContextProjectionRequest,
    ) -> Result<ContextProjectionResult, String> {
        let stable_id = sanitize_projection_id(&request.stable_id);
        let source_kind = sanitize_projection_id(&request.source_kind);
        let preview: String = request.content.chars().take(1000).collect();
        let original_bytes = request.content.len();
        let (paradigm, entities) = crate::context_artifact::store(
            self.world,
            self.registry,
            &format!(
                "context:{}",
                if source_kind == "tool_result_budget" {
                    "tool_result"
                } else {
                    &source_kind
                }
            ),
            &request.source_label,
            &request.content,
        )?;
        let entity = entities[0];
        let total_shards = entities.len() as u32;

        if let Some(tx_id) = &self.tx_id {
            audit::record_action(
                self.world,
                self.registry,
                audit::ActionRecordDraft {
                    action_id: audit::new_action_id(),
                    tx_id: tx_id.clone(),
                    wave: self.current_wave as u32,
                    tool_call_id: format!("project_context:{source_kind}:{stable_id}"),
                    tool_name: "context_projection".into(),
                    kind: "ProjectContext".into(),
                    entity: Some(format!("{entity:?}")),
                    component: Some("ShardData".into()),
                    input: serde_json::json!({
                        "source_kind": source_kind.clone(),
                        "source_label": request.source_label.clone(),
                        "stable_id": stable_id.clone(),
                        "original_bytes": original_bytes
                    }),
                    output_summary: format!(
                        "Projected {} bytes from context {} into {}",
                        original_bytes, request.source_label, paradigm
                    ),
                    is_error: false,
                    before: None,
                    after: Some(serde_json::json!({
                        "paradigm": paradigm.clone(),
                        "component": "ShardData",
                        "total_shards": total_shards,
                        "original_bytes": original_bytes,
                        "preview": preview.clone()
                    })),
                    created_at: audit::now_secs(),
                },
            );
        }

        Ok(ContextProjectionResult {
            paradigm,
            component: "ShardData".into(),
            total_shards,
            original_bytes,
            preview,
        })
    }
}

fn sanitize_projection_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(96));
    for ch in raw.chars().take(96) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}
