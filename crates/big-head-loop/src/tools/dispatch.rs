//! Route calls to concrete services; lifecycle checks live in executor.rs.

use super::{
    RuntimeToolExecutor, cli, define, ecs_capabilities, fetch_url, helpers::log_tool_error,
    inspect_project, inspect_runtime, inspect_transaction, inspect_view, lens, lens_app, read,
    run_system, shard, shard_catalog, system, tick_action, unified_search,
    view_state::persist_view_state, write,
};
use crate::components::{Spark, SparkStatus};
use crate::system_def::SystemKind;
use big_head_ai::{AgentAction, ToolCall, ToolExecutor};
use std::collections::HashSet;

impl RuntimeToolExecutor<'_> {
    pub(super) async fn dispatch_call(
        &mut self,
        call: &ToolCall,
        audit_target: &mut Option<(Option<String>, Option<String>)>,
    ) -> (String, bool) {
        match call.name.as_str() {
            "request_user_input" => match crate::user_question::create(self.world, self.tx_id.as_deref().unwrap_or_default(), &call.input) {
                Ok(value) => (value.to_string(), false),
                Err(error) => (error, true),
            },
            "create_activity"
            | "update_activity_plan"
            | "control_activity"
            | "inspect_activity" => {
                match crate::activity::dispatch(
                    self.world,
                    self.registry,
                    self.system_registry,
                    &call.name,
                    &call.input,
                ) {
                    Ok(value) => {
                        if call.name != "inspect_activity" {
                            self.actions_applied.push(AgentAction::ExecuteSystem {
                                id: call.name.clone(),
                                result_summary:
                                    "activity state updated; not proof of goal completion".into(),
                            });
                        }
                        (value.to_string(), false)
                    }
                    Err(error) => (error, true),
                }
            }
            "select_interpretation_path" => {
                match crate::interpretation::prepare_laya_selection(self.world, &call.input) {
                    Err(error) => (format!("select_interpretation_path: {error}"), true),
                    Ok((request, choices, selection)) => {
                        let base = std::env::var("BIGHEAD_LAYA_URL")
                            .unwrap_or_else(|_| "http://127.0.0.1:3031".into());
                        match super::laya::predict(&request, &choices, &base).await {
                            Err(error) => (format!("select_interpretation_path: {error}"), true),
                            Ok(response) => match crate::interpretation::finish_laya_selection(
                                response, selection,
                            ) {
                                Ok(value) => (value.to_string(), false),
                                Err(error) => {
                                    (format!("select_interpretation_path: {error}"), true)
                                }
                            },
                        }
                    }
                }
            }
            "record_fact"
            | "record_interpretation"
            | "materialize_interpretation"
            | "inspect_interpretations" => {
                match crate::interpretation::dispatch(self.world, self.registry, call) {
                    Ok(value) => {
                        if call.name != "inspect_interpretations" {
                            self.actions_applied.push(AgentAction::ExecuteSystem {
                                id: call.name.clone(),
                                result_summary: "interpretation world updated".into(),
                            });
                        }
                        (value.to_string(), false)
                    }
                    Err(error) => (error, true),
                }
            }
            "define_interpretation_method" | "inspect_interpretation_methods" => {
                let result = if call.name == "define_interpretation_method" {
                    crate::interpretation_method::define(self.world, &call.input)
                } else {
                    crate::interpretation_method::inspect(self.world, &call.input)
                };
                match result {
                    Ok(value) => {
                        if call.name == "define_interpretation_method" {
                            self.actions_applied.push(AgentAction::ExecuteSystem {
                                id: call.name.clone(),
                                result_summary: "interpretation method updated".into(),
                            });
                        }
                        (value.to_string(), false)
                    }
                    Err(error) => (error, true),
                }
            }
            "inspect_work" | "record_work" | "arrange_work" => {
                match crate::work_world::dispatch(self.world, &call.name, &call.input) {
                    Ok(value) => {
                        if call.name != "inspect_work" {
                            self.actions_applied.push(AgentAction::ExecuteSystem {
                                id: call.name.clone(),
                                result_summary: "work district updated".into(),
                            });
                        }
                        (value.to_string(), false)
                    }
                    Err(error) => (error, true),
                }
            }
            "laya_assess" => match crate::laya_workflows::assess(&call.input).await {
                Ok(value) => (value.to_string(), false),
                Err(error) => (format!("laya_assess: {error}"), true),
            },
            "inspect_laya_shadow" => (crate::laya_shadow::status(self.world).to_string(), false),
            "laya_run_spec" => {
                let mut available = big_head_ai::ecs_tool_definitions();
                available.extend(self.dynamic_tool_definitions());
                if let Some(allowed) = self.allowed_tools {
                    available.retain(|tool| allowed.contains(&tool.name));
                }
                let available_names: HashSet<_> =
                    available.into_iter().map(|tool| tool.name).collect();
                let registered_system_tools: HashSet<_> = self
                    .system_registry
                    .tool_definitions()
                    .into_iter()
                    .map(|tool| tool.name)
                    .collect();
                match crate::laya_spec::prepare(
                    &call.input,
                    available_names,
                    registered_system_tools,
                )
                .await
                {
                    Err(error) => (format!("laya_run_spec: {error}"), true),
                    Ok(decision) => {
                        let should_execute = decision.verdict == "act";
                        let checks = decision
                            .action
                            .as_ref()
                            .map(|action| action.verify.clone())
                            .unwrap_or_default();
                        let mut execution_attempted = false;
                        let mut execution_succeeded = None;
                        let mut execution_output: Option<String> = None;
                        let mut execution = serde_json::json!({
                            "attempted":false,"succeeded":null,"tool":null,"output":null
                        });
                        let mut verification = Vec::new();

                        if should_execute {
                            let nested = crate::laya_spec::action_call(&decision)
                                .expect("act verdict always has an action");
                            execution_attempted = true;
                            let policy_error = self.pipeline_policy_violation(&nested);
                            let before_target = crate::audit::write_before_after_target(
                                self.world,
                                self.registry,
                                &nested.name,
                                &nested.input,
                            );
                            let mut nested_target = None;
                            let (output, failed) = if let Some(error) = policy_error {
                                (error, true)
                            } else {
                                Box::pin(self.dispatch_call(&nested, &mut nested_target)).await
                            };
                            execution_succeeded = Some(!failed);
                            execution_output = Some(output.clone());
                            execution = serde_json::json!({"attempted":true,"succeeded":!failed,
                                "tool":nested.name,"output":output});

                            if let Some(tx_id) = &self.tx_id {
                                let (mut entity, component, before) =
                                    before_target.unwrap_or((None, None, None));
                                if !failed && entity.is_none() {
                                    entity = crate::audit::entity_from_tool_output(
                                        &nested.name,
                                        execution_output.as_deref().unwrap_or_default(),
                                    );
                                }
                                super::action_audit::record_tool_action(
                                    self.world,
                                    self.registry,
                                    tx_id,
                                    &nested,
                                    self.current_wave,
                                    execution_output.as_deref().unwrap_or_default(),
                                    failed,
                                    entity,
                                    component,
                                    before,
                                    None,
                                );
                            }

                            if !failed {
                                for check in &checks {
                                    let verify_call =
                                        crate::laya_spec::verification_call(&decision, check);
                                    let mut verify_target = None;
                                    let (output, is_error) = Box::pin(
                                        self.dispatch_call(&verify_call, &mut verify_target),
                                    )
                                    .await;
                                    let assessed = crate::laya_spec::assess_verification(
                                        check, &output, is_error,
                                    );
                                    if let Some(tx_id) = &self.tx_id {
                                        super::action_audit::record_tool_action(
                                            self.world,
                                            self.registry,
                                            tx_id,
                                            &verify_call,
                                            self.current_wave,
                                            &output,
                                            is_error,
                                            None,
                                            None,
                                            None,
                                            None,
                                        );
                                    }
                                    verification.push(assessed);
                                }
                            }
                        }

                        let outcome = crate::laya_spec::outcome(
                            &decision,
                            execution_attempted,
                            execution_succeeded,
                            execution_output.as_deref(),
                            verification.clone(),
                        );
                        let outcome_record = super::laya::submit_outcome(&outcome).await;
                        (
                            crate::laya_spec::result(
                                decision,
                                execution,
                                verification,
                                outcome,
                                outcome_record,
                            )
                            .to_string(),
                            false,
                        )
                    }
                }
            }
            "laya_decide" => {
                let mut available = big_head_ai::ecs_tool_definitions();
                available.extend(self.dynamic_tool_definitions());
                if let Some(allowed) = self.allowed_tools {
                    available.retain(|tool| allowed.contains(&tool.name));
                }
                match super::laya::decide(&call.input, &available).await {
                    Ok(value) => (value.to_string(), false),
                    Err(error) => (format!("laya_decide: {error}"), true),
                }
            }
            "inspect_scene" => (
                crate::scene_world::projection(self.world).to_string(),
                false,
            ),
            "inspect_ecs_capabilities" => {
                let mut available = big_head_ai::ecs_tool_definitions();
                available.extend(self.system_registry.tool_definitions());
                available.extend(self.runtime_tool_definitions());
                match ecs_capabilities::inspect_authorized(
                    self.world,
                    self.registry,
                    self.system_registry,
                    &available,
                    self.allowed_tools,
                    &call.input,
                ) {
                    Ok(value) => (value.to_string(), false),
                    Err(error) => (format!("inspect_ecs_capabilities: {error}"), true),
                }
            }
            "update_system" | "remove_system" => {
                let reserved: Vec<_> = big_head_ai::ecs_tool_definitions()
                    .into_iter()
                    .chain(self.runtime_tool_definitions())
                    .map(|tool| tool.name)
                    .collect();
                match ecs_capabilities::mutate(
                    self.system_registry,
                    self.rhai_engine,
                    call,
                    &reserved,
                ) {
                    Ok((value, action)) => {
                        if let Some(ref hook) = self.state_hook {
                            hook.emit(big_head_state::AgentActivity::ActionApplied {
                                action: action.summary(),
                            });
                        }
                        self.actions_applied.push(action);
                        (value.to_string(), false)
                    }
                    Err(error) => (format!("{}: {error}", call.name), true),
                }
            }
            "dream_task" | "dream_policy" => match crate::dream::dispatch(self.world, call) {
                Ok(output) => (output, false),
                Err(error) => (error, true),
            },
            "inspect_runtime" => (
                inspect_runtime::inspect(self.world, self.spark_queue, &call.input).to_string(),
                false,
            ),
            "inspect_transaction" => match inspect_transaction::inspect(self.world, &call.input) {
                Ok(output) => (output.to_string(), false),
                Err(error) => (error, true),
            },
            "inspect_project" => match std::env::current_dir()
                .map_err(|e| e.to_string())
                .and_then(|root| inspect_project::inspect(&root, &call.input))
            {
                Ok(output) => (output.to_string(), false),
                Err(error) => (format!("inspect_project: {error}"), true),
            },
            // ── 读取工具 ──
            "inspect_world_shape" => {
                match crate::world_shape::inspect(self.world, self.registry, &call.input) {
                    Ok(output) => (output.to_string(), false),
                    Err(error) => (error, true),
                }
            }
            "get_world_overview" => {
                read::dispatch_get_world_overview(self.world, self.registry, call)
            }
            "query_entities" => read::dispatch_query_entities(self.world, self.registry, call),
            "query_ecs" => super::structured_query::dispatch(self.world, self.registry, call),
            "get_entity" => read::dispatch_get_entity(self.world, self.registry, call),
            "get_component" => read::dispatch_get_component(self.world, self.registry, call),

            "inspect_component_view" => {
                if let Some(ref handler) = self.view_handler {
                    let result = inspect_view::dispatch(self.world, self.registry, handler, call);
                    if !result.is_error {
                        self.actions_applied.push(AgentAction::ExecuteSystem {
                            id: "inspect_component_view".into(),
                            result_summary: "ok".into(),
                        });
                        if let Some(input) = result.persisted_input {
                            match persist_view_state(self.world, self.registry, &input) {
                                Ok(entity) => {
                                    *audit_target = Some((
                                        Some(format!("{entity:?}")),
                                        Some("ViewState".into()),
                                    ));
                                }
                                Err(error) => log_tool_error(
                                    &self.log_emitter,
                                    "persist_view_state",
                                    &format!("Failed: {error}"),
                                ),
                            }
                        }
                    } else {
                        log_tool_error(&self.log_emitter, "inspect_component_view", &result.output);
                    }
                    (result.output, result.is_error)
                } else {
                    let message = "View server not running. Start big-head with --view-server to inspect ECS components.".to_string();
                    log_tool_error(&self.log_emitter, "inspect_component_view", &message);
                    (message, true)
                }
            }

            // ── 写入工具 ──
            "spawn_entity" | "modify_component" | "remove_component" | "despawn_entity" => {
                let (output, is_error, action) =
                    write::dispatch_write_action(self.world, self.registry, call);
                if let Some(ref a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a.clone());
                }
                (output, is_error)
            }

            "update_tick_action" => {
                let (output, is_error, action) =
                    tick_action::dispatch_update_tick_action(self.world, self.rhai_engine, call);
                if let Some(action) = action {
                    if let Some(ref hook) = self.state_hook {
                        hook.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", action),
                        });
                    }
                    self.actions_applied.push(action);
                }
                (output, is_error)
            }

            // ── 定义工具 ──
            "define_component" => {
                let (output, is_error, action) =
                    define::dispatch_define_component(self.world, self.registry, call);
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "undefine_component" => {
                let (output, is_error, action) =
                    define::dispatch_undefine_component(self.world, self.registry, call);
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "define_system" => {
                let (output, is_error, action) =
                    system::dispatch_define_system(self.system_registry, self.rhai_engine, call);
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "define_tool" => {
                let (output, is_error, action) =
                    system::dispatch_define_tool(self.system_registry, self.rhai_engine, call);
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "run_system" => {
                let (output, is_error, action) = run_system::dispatch_run_system(
                    self.system_registry,
                    self.rhai_engine,
                    self.world,
                    self.registry,
                    Some(&mut *self.spark_queue),
                    self.log_emitter.clone(),
                    call,
                );
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "execute_cli" => {
                let (output, is_error, action) = cli::dispatch_execute_cli(
                    self.world,
                    self.cli_security,
                    self.cli_executor,
                    call,
                );
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "inspect_processes" => {
                let mut processes: Vec<_> = self
                    .world
                    .query::<&big_head_ecs::ManagedProcess>()
                    .iter(self.world)
                    .cloned()
                    .collect();
                processes
                    .sort_by_key(|process| (std::cmp::Reverse(process.started_at), process.pid));
                (
                    serde_json::to_string(&processes).unwrap_or_else(|_| "[]".into()),
                    false,
                )
            }

            "shard_content" => {
                let (output, is_error, action) = shard::dispatch_shard_content(
                    self.world,
                    self.registry,
                    self.sharder,
                    call,
                    self.last_tool_result.as_deref(),
                );
                if let Some(a) = action {
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("{:?}", a),
                        });
                    }
                    self.actions_applied.push(a);
                }
                (output, is_error)
            }

            "get_shards" => {
                let expired = call
                    .input
                    .get("paradigm")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|p| crate::shard_retention::expired_notice(self.world, p));
                if let Some(notice) = expired {
                    (notice, true)
                } else {
                    if let Some(p) = call
                        .input
                        .get("paradigm")
                        .and_then(serde_json::Value::as_str)
                    {
                        crate::shard_retention::touch(
                            self.world,
                            p,
                            big_head_ecs::execution::now_secs(),
                        );
                    }
                    let (output, is_error, _action) =
                        shard::dispatch_get_shards(self.world, self.registry, call);
                    (output, is_error)
                }
            }

            "get_shard_content" => {
                let expired = call
                    .input
                    .get("paradigm")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|p| crate::shard_retention::expired_notice(self.world, p));
                if let Some(notice) = expired {
                    (notice, true)
                } else {
                    if let Some(p) = call
                        .input
                        .get("paradigm")
                        .and_then(serde_json::Value::as_str)
                    {
                        crate::shard_retention::touch(
                            self.world,
                            p,
                            big_head_ecs::execution::now_secs(),
                        );
                    }
                    let (output, is_error, _action) =
                        shard::dispatch_get_shard_content(self.world, self.registry, call);
                    (output, is_error)
                }
            }

            "inspect_shard_retention" | "set_shard_retention" => {
                let result = if call.name == "inspect_shard_retention" {
                    crate::shard_retention::inspect(
                        self.world,
                        self.registry,
                        self.system_registry,
                        big_head_ecs::execution::now_secs(),
                    )
                } else {
                    crate::shard_retention::configure(self.world, &call.input)
                };
                match result {
                    Ok(v) => (v.to_string(), false),
                    Err(e) => (e, true),
                }
            }
            "inspect_shards" | "link_shard_sources" => {
                let result = if call.name == "inspect_shards" {
                    shard_catalog::inspect(self.world, &call.input)
                } else {
                    shard_catalog::link(self.world, &call.input)
                };
                match result {
                    Ok(value) => (value.to_string(), false),
                    Err(error) => (error, true),
                }
            }
            "search" => {
                let result =
                    unified_search::dispatch_unified_search(self.state_hook.as_ref(), call).await;
                if let Some(ref h) = self.state_hook {
                    h.emit(big_head_state::AgentActivity::ActionApplied {
                        action: format!(
                            "UnifiedSearch(query={}, kind={})",
                            call.input
                                .get("query")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .chars()
                                .take(60)
                                .collect::<String>(),
                            call.input
                                .get("kind")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?"),
                        ),
                    });
                }
                self.actions_applied.push(AgentAction::ExecuteSystem {
                    id: "search".into(),
                    result_summary: if result.is_error {
                        "error".into()
                    } else {
                        "ok".into()
                    },
                });
                (result.output, result.is_error)
            }

            "fetch_url" => {
                let result = fetch_url::dispatch_fetch_url(call).await;
                if let Some(ref h) = self.state_hook {
                    h.emit(big_head_state::AgentActivity::ActionApplied {
                        action: format!(
                            "FetchUrl(url={})",
                            call.input
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .chars()
                                .take(100)
                                .collect::<String>(),
                        ),
                    });
                }
                self.actions_applied.push(AgentAction::ExecuteSystem {
                    id: "fetch_url".into(),
                    result_summary: if result.is_error {
                        "error".into()
                    } else {
                        "ok".into()
                    },
                });
                (result.output, result.is_error)
            }

            "discover_lenses"
            | "record_lens_feedback"
            | "propose_lens_candidate"
            | "trial_lens_candidate"
            | "adopt_lens_candidate"
            | "inspect_lens_learning" => {
                match crate::lens_learning::dispatch(self.world, self.registry, call) {
                    Ok(output) => (output, false),
                    Err(error) => (error, true),
                }
            }
            "define_lens_binding" | "inspect_lens_bindings" => {
                match crate::lens_bindings::dispatch(self.world, self.registry, call) {
                    Ok(output) => (output, false),
                    Err(error) => (error, true),
                }
            }
            "define_lens_app" | "run_lens_app" | "inspect_lens_apps" | "describe_lens_contract" => {
                match lens_app::dispatch(self.world, self.registry, call) {
                    Ok(output) => (output, false),
                    Err(error) => (error, true),
                }
            }
            "create_lens" | "list_lens_inbox" | "read_lens" | "accept_lens" | "reject_lens" => {
                if let Some(manifest) = self.world_manifest {
                    lens::dispatch(manifest, call).unwrap_or_else(|| {
                        (
                            format!("Lens tool dispatch missing for '{}'", call.name),
                            true,
                        )
                    })
                } else {
                    (
                        "Lens tools are unavailable because no WorldManifest is attached to this runtime.".into(),
                        true,
                    )
                }
            }

            "render_view" | "render_mind_view" | "render_document" => {
                // All view-specific dispatch lives in big-head-view::ViewToolHandler.
                // Loop crate stays view-agnostic — it only routes the tool call.
                if let Some(ref h) = self.view_handler {
                    let normalized_input = if call.name == "render_document" {
                        big_head_view::normalize_document(&call.input)
                    } else if call.name == "render_mind_view" {
                        big_head_view::normalize_mind_view_input(&call.input)
                    } else {
                        Ok(big_head_view::normalize_view_command_input(
                            call.input.clone(),
                        ))
                    };
                    match normalized_input {
                        Ok(normalized_input) => {
                            let normalized_call = ToolCall {
                                id: call.id.clone(),
                                name: "render_view".into(),
                                input: normalized_input.clone(),
                            };
                            let (raw_out, is_err) = h.dispatch(&normalized_call);
                            let out = if !is_err && call.name == "render_document" {
                                serde_json::json!({"published":true,"panel_id":normalized_input["id"],"route":format!("/#panel={}",normalized_input["id"].as_str().unwrap_or_default()),"title":normalized_input["title"],"user_read_verified":false}).to_string()
                            } else if !is_err && call.name == "render_mind_view" {
                                let markup_bytes = normalized_input["content"]["markup"]
                                    .as_str()
                                    .map(str::len)
                                    .unwrap_or_default();
                                format!(
                                    "Mind View published and persisted: panel=mind-view markup_bytes={markup_bytes}"
                                )
                            } else {
                                raw_out
                            };
                            if !is_err {
                                self.actions_applied.push(AgentAction::ExecuteSystem {
                                    id: call.name.clone(),
                                    result_summary: "ok".into(),
                                });
                                // 持久化 view 状态到 ECS
                                match persist_view_state(
                                    self.world,
                                    self.registry,
                                    &normalized_input,
                                ) {
                                    Ok(entity) => {
                                        *audit_target = Some((
                                            Some(format!("{entity:?}")),
                                            Some("ViewState".into()),
                                        ));
                                    }
                                    Err(e) => {
                                        log_tool_error(
                                            &self.log_emitter,
                                            "persist_view_state",
                                            &format!("Failed: {}", e),
                                        );
                                        if call.name == "render_document" {
                                            return (format!("Document accepted by View but persistence failed: {e}. Reuse the same document id when repairing."), true);
                                        }
                                    }
                                }
                            } else {
                                log_tool_error(&self.log_emitter, &call.name, &out);
                            }
                            (out, is_err)
                        }
                        Err(error) => {
                            log_tool_error(&self.log_emitter, &call.name, &error);
                            (error, true)
                        }
                    }
                } else {
                    let msg = "View server not running. Start big-head with --view-server before rendering a browser panel.".to_string();
                    log_tool_error(&self.log_emitter, &call.name, &msg);
                    (msg, true)
                }
            }

            "render_status" => {
                // Lightweight ambient UI status for clients such as the macOS
                // menu bar app. Protocol and validation live in big-head-view.
                if let Some(ref h) = self.view_handler {
                    let (out, is_err) = h.dispatch_status(call);
                    if !is_err {
                        self.actions_applied.push(AgentAction::ExecuteSystem {
                            id: "render_status".into(),
                            result_summary: "ok".into(),
                        });
                    } else {
                        log_tool_error(&self.log_emitter, "render_status", &out);
                    }
                    (out, is_err)
                } else {
                    let msg =
                        "View server not running. Start big-head with --view-server to publish status."
                            .to_string();
                    log_tool_error(&self.log_emitter, "render_status", &msg);
                    (msg, true)
                }
            }

            "push_spark" => match call.input.get("description").and_then(|v| v.as_str()) {
                Some(desc) => {
                    let spark = Spark {
                        text: desc.to_string(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        status: SparkStatus::Pending,
                        ..Default::default()
                    };
                    let entity = big_head_ecs::execution::enqueue(
                        self.world,
                        self.spark_queue,
                        spark,
                        false,
                    );
                    *audit_target = Some((Some(format!("{entity:?}")), Some("Spark".into())));
                    if let Some(ref h) = self.state_hook {
                        h.emit(big_head_state::AgentActivity::ActionApplied {
                            action: format!("PushSpark({})", desc),
                        });
                    }
                    (format!("Spark queued: {}", desc), false)
                }
                None => ("Missing 'description' field".to_string(), true),
            },

            "set_session_context" => {
                *audit_target = Some((None, Some(crate::ambient::SESSION_CONTEXT.into())));
                crate::ambient::dispatch_set_session_context(self.world, self.registry, call)
            }

            "push_ambient_signal" => {
                *audit_target = Some((None, Some(crate::ambient::AMBIENT_SIGNAL.into())));
                crate::ambient::dispatch_push_ambient_signal(
                    self.world,
                    self.registry,
                    self.spark_queue,
                    call,
                )
            }

            // ── 动态工具（Rhai 定义的工具）──
            _ => {
                if let Some(sys) = self
                    .system_registry
                    .get(&call.name)
                    .filter(|sys| sys.is_tool)
                {
                    if let SystemKind::RhaiScript { source } = &sys.kind {
                        let source = source.clone();
                        let (result, logs) = crate::scripting::eval_with_world_captured_args(
                            self.rhai_engine,
                            &source,
                            self.world,
                            self.registry,
                            self.system_registry,
                            Some(&mut *self.spark_queue),
                            self.log_emitter.clone(),
                            call.input.get("args"),
                        );
                        let is_error = result.is_err();
                        let output = match result {
                            Ok(val) => {
                                let val_str = format!("{:?}", val);
                                if logs.is_empty() {
                                    format!("Tool '{}' executed. Return: {}", call.name, val_str)
                                } else {
                                    format!(
                                        "Tool '{}' executed.\n{}\nReturn: {}",
                                        call.name,
                                        logs.join("\n"),
                                        val_str
                                    )
                                }
                            }
                            Err(e) => {
                                if logs.is_empty() {
                                    format!("Tool '{}' failed: {}", call.name, e)
                                } else {
                                    format!(
                                        "Tool '{}' failed: {}\n{}",
                                        call.name,
                                        e,
                                        logs.join("\n")
                                    )
                                }
                            }
                        };
                        let action = AgentAction::ExecuteSystem {
                            id: call.name.clone(),
                            result_summary: if is_error {
                                "error".into()
                            } else {
                                "ok".into()
                            },
                        };
                        if let Some(ref h) = self.state_hook {
                            h.emit(big_head_state::AgentActivity::ActionApplied {
                                action: format!("{:?}", action),
                            });
                        }
                        self.actions_applied.push(action);
                        (output, is_error)
                    } else {
                        (format!("Unknown tool: {}", call.name), true)
                    }
                } else {
                    (format!("Unknown tool: {}", call.name), true)
                }
            }
        }
    }
}
