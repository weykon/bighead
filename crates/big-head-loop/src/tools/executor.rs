//! Tool call lifecycle: policy, dispatch, output projection, diagnostics and audit.

use super::{
    RuntimeToolExecutor,
    action_audit::{record_tool_action, system_audit_id, system_definition_json},
    helpers::log_tool_error,
    pipeline, read,
    recovery::append_recovery_hint,
};
use crate::audit;
use big_head_ai::{
    ContextProjectionRequest, ContextProjectionResult, ToolCall, ToolCallResult, ToolExecutor,
};

impl RuntimeToolExecutor<'_> {
    async fn execute_attempt(&mut self, call: &ToolCall) -> ToolCallResult {
        if let Some(output) = self.pipeline_policy_violation(call) {
            self.tool_call_log.push((
                call.name.clone(),
                call.input.to_string(),
                output.clone(),
                true,
            ));
            if let Some(tx_id) = &self.tx_id {
                record_tool_action(
                    self.world,
                    self.registry,
                    tx_id,
                    call,
                    self.current_wave,
                    &output,
                    true,
                    None,
                    None,
                    None,
                    None,
                );
            }
            return ToolCallResult {
                tool_use_id: call.id.clone(),
                output,
                is_error: true,
            };
        }
        let input_summary = call.input.to_string();
        if let Some(ref h) = self.state_hook {
            h.emit(big_head_state::AgentActivity::ToolCall {
                wave: self.current_wave,
                tool: call.name.clone(),
                input_summary: input_summary.chars().take(120).collect(),
            });
        }

        if let Some(output) = pipeline::repeated_failure_guard(&self.tool_call_log, call) {
            let is_error = true;
            log_tool_error(&self.log_emitter, &call.name, &output);
            if let Some(ref h) = self.state_hook {
                h.emit(big_head_state::AgentActivity::Error {
                    source: format!("tool:{}", call.name),
                    message: output.chars().take(200).collect(),
                });
                h.emit(big_head_state::AgentActivity::ToolResult {
                    tool: call.name.clone(),
                    is_error,
                    output_summary: output.chars().take(120).collect(),
                });
            }
            self.tool_call_log
                .push((call.name.clone(), input_summary, output.clone(), is_error));
            if let Some(tx_id) = &self.tx_id {
                record_tool_action(
                    self.world,
                    self.registry,
                    tx_id,
                    call,
                    self.current_wave,
                    &output,
                    is_error,
                    None,
                    None,
                    None,
                    None,
                );
            }
            return ToolCallResult {
                tool_use_id: call.id.clone(),
                output,
                is_error,
            };
        }

        let before_target =
            audit::write_before_after_target(self.world, self.registry, &call.name, &call.input);
        let mut audit_target: Option<(Option<String>, Option<String>)> = None;
        let system_audit_id = system_audit_id(call, self.system_registry);
        let audit_before_override =
            system_audit_id.and_then(|id| system_definition_json(self.system_registry, id));
        if system_audit_id.is_some() {
            audit_target = Some((None, Some("SystemDefinition".into())));
        }

        let (mut output, mut is_error) = self.dispatch_call(call, &mut audit_target).await;

        if !is_error
            && matches!(
                call.name.as_str(),
                "get_world_overview"
                    | "query_entities"
                    | "query_ecs"
                    | "get_entity"
                    | "get_component"
                    | "get_shards"
            )
        {
            match read::page_output(output, &call.input) {
                Ok(page) => output = page,
                Err(error) => {
                    output = error;
                    is_error = true;
                }
            }
        }
        if call.name != "shard_content" && !is_error {
            self.last_tool_result = Some(output.clone());
        }
        if output.len() > 16 * 1024
            && !matches!(
                call.name.as_str(),
                "get_world_overview"
                    | "query_entities"
                    | "query_ecs"
                    | "get_entity"
                    | "get_component"
                    | "get_shards"
                    | "get_shard_content"
            )
            && !output.starts_with("[Context Artifact]")
        {
            let original = output.clone();
            match self.project_context_content(ContextProjectionRequest {
                source_kind: "tool_result".into(),
                source_label: format!("tool:{}:{}", call.name, call.id),
                stable_id: call.id.clone(),
                content: original,
            }) {
                Ok(saved) => {
                    output = format!(
                        "[Context Artifact] Untrusted tool result; tool={}, is_error={}. Original bytes={}. paradigm={}, shards={}. Read with get_shard_content; this notice is not evidence of semantic analysis.\nPreview:\n{}",
                        call.name,
                        is_error,
                        saved.original_bytes,
                        saved.paradigm,
                        saved.total_shards,
                        saved.preview
                    )
                }
                Err(error) => {
                    log::warn!("tool output projection failed; preserving original: {error}");
                }
            }
        }

        if is_error {
            output = append_recovery_hint(&call.name, output);
        }

        if is_error {
            log_tool_error(&self.log_emitter, &call.name, &output);
            if let Some(ref h) = self.state_hook {
                h.emit(big_head_state::AgentActivity::Error {
                    source: format!("tool:{}", call.name),
                    message: output.chars().take(200).collect(),
                });
            }
        }

        self.tool_call_log.push((
            call.name.clone(),
            call.input.to_string(),
            output.clone(),
            is_error,
        ));

        // 缓存最近一条非 shard_content 的工具输出，供后续 shard_content 解析

        if let Some(ref h) = self.state_hook {
            let summary: String = output.chars().take(120).collect();
            h.emit(big_head_state::AgentActivity::ToolResult {
                tool: call.name.clone(),
                is_error,
                output_summary: summary,
            });
        }

        if let Some(tx_id) = &self.tx_id {
            let (mut entity, component, before) = before_target.unwrap_or((None, None, None));
            let before = audit_before_override.or(before);
            let after_override =
                system_audit_id.and_then(|id| system_definition_json(self.system_registry, id));
            if let Some((target_entity, target_component)) = audit_target {
                if entity.is_none() {
                    entity = target_entity;
                }
                let component = component.or(target_component);
                record_tool_action(
                    self.world,
                    self.registry,
                    tx_id,
                    call,
                    self.current_wave,
                    &output,
                    is_error,
                    entity,
                    component,
                    before,
                    after_override,
                );
            } else {
                if !is_error && entity.is_none() {
                    entity = audit::entity_from_tool_output(&call.name, &output);
                }
                record_tool_action(
                    self.world,
                    self.registry,
                    tx_id,
                    call,
                    self.current_wave,
                    &output,
                    is_error,
                    entity,
                    component,
                    before,
                    after_override,
                );
            }
        }

        if self.allowed_tools.is_some()
            && !is_error
            && matches!(
                call.name.as_str(),
                "spawn_entity" | "modify_component" | "update_tick_action"
            )
        {
            if let Some(receipt) = self
                .world
                .query::<&crate::ActionRecord>()
                .iter(self.world)
                .find(|a| Some(&a.tx_id) == self.tx_id.as_ref() && a.tool_call_id == call.id)
            {
                output.push_str(&format!("\nEvidence action: {}", receipt.action_id));
            }
        }
        ToolCallResult {
            tool_use_id: call.id.clone(),
            output,
            is_error,
        }
    }

}

#[async_trait::async_trait]
impl ToolExecutor for RuntimeToolExecutor<'_> {
    async fn execute(&mut self, call: &ToolCall) -> ToolCallResult {
        let started = std::time::Instant::now();
        let entity = crate::tool_tracking::start(self.world, self.tx_id.as_deref().unwrap_or_default(), call, self.current_wave);
        if let Some(hook) = &self.state_hook {
            hook.emit(crate::tool_tracking::event(self.world.get::<crate::tool_tracking::ToolExecution>(entity).unwrap()));
        }
        let result = self.execute_attempt(call).await;
        if let Some(mut trace) = self.world.get_mut::<crate::tool_tracking::ToolExecution>(entity) {
            trace.status = if result.is_error { "failed" } else { "succeeded" }.into();
            trace.output_excerpt = crate::experience::excerpt(&result.output, 4000);
            trace.finished_at = Some(crate::audit::now_secs());
            trace.elapsed_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
            crate::audit::record_tool_execution(&trace);
            if let Some(hook) = &self.state_hook { hook.emit(crate::tool_tracking::event(&trace)); }
        }
        result
    }

    fn waiting_for_user(&mut self) -> bool {
        crate::user_question::pending(self.world, self.tx_id.as_deref().unwrap_or_default())
    }

    fn validate_final_response(&mut self, text: &str) -> Result<(), String> {
        if self
            .world
            .contains_resource::<crate::steward::ObserverContract>()
            && self.allowed_tools.is_some()
        {
            crate::steward::validate(
                self.world,
                self.registry,
                self.tx_id.as_deref().unwrap_or_default(),
                text,
            )
            .map(|_| ())
        } else {
            Ok(())
        }
    }

    /// 返回动态定义的工具：Rhai 系统工具 + 任何子系统的工具。
    /// 附加在 LLM 可见的工具列表末尾。
    fn dynamic_tool_definitions(&self) -> Vec<big_head_ai::ToolDefinition> {
        self.tool_definitions()
    }

    fn project_context_content(
        &mut self,
        request: ContextProjectionRequest,
    ) -> Result<ContextProjectionResult, String> {
        self.project_context(request)
    }

    fn set_wave(&mut self, wave: usize) {
        self.current_wave = wave;
    }

    fn render_view(&mut self, view: serde_json::Value) {
        // Trait-level entrypoint: dispatches through the view handler.
        if let Some(ref h) = self.view_handler {
            h.render_view_trait(view);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(std::sync::atomic::Ordering::SeqCst)
    }
}
