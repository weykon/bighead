//! `run_system` 工具分发 —— 执行已注册的 Rhai 脚本系统，
//! 并返回捕获的 log 输出。

use std::collections::VecDeque;

use bevy_ecs::entity::Entity;
use big_head_ai::{AgentAction, LogEmitter, ToolCall};

use crate::registry::ComponentActionRegistry;
use crate::scripting;
use crate::system_def::{SystemKind, SystemRegistry};

/// 分发 `run_system` 工具。返回 `(output, is_error, action)`。
pub(crate) fn dispatch_run_system(
    system_registry: &mut SystemRegistry,
    rhai_engine: &std::sync::Arc<rhai::Engine>,
    world: &mut bevy_ecs::world::World,
    registry: &mut ComponentActionRegistry,
    spark_queue: Option<&mut VecDeque<Entity>>,
    log_emitter: Option<LogEmitter>,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let id = match call.input.get("id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return ("run_system: missing 'id' parameter".into(), true, None),
    };

    let source = match system_registry.get(&id) {
        Some(sys) => match &sys.kind {
            SystemKind::RhaiScript { source } => source.clone(),
            SystemKind::Llm => {
                return (
                    format!("run_system: '{}' is an Llm system, not a Rhai script", id),
                    true,
                    None,
                );
            }
        },
        None => return (format!("run_system: system '{}' not found", id), true, None),
    };

    let (result, logs) = scripting::eval_with_world_captured_args(
        rhai_engine,
        &source,
        world,
        registry,
        system_registry,
        spark_queue,
        log_emitter,
        call.input.get("args"),
    );

    let is_error = result.is_err();

    let output = match result {
        Ok(val) => {
            let val_str = format!("{:?}", val);
            if logs.is_empty() {
                format!(
                    "System '{}' executed successfully. Return value: {}",
                    id, val_str
                )
            } else {
                format!(
                    "System '{}' executed successfully.\n\nLog output:\n{}\n\nReturn value: {}",
                    id,
                    logs.join("\n"),
                    val_str
                )
            }
        }
        Err(e) => {
            if logs.is_empty() {
                format!("System '{}' failed: {}", id, e)
            } else {
                format!(
                    "System '{}' failed: {}\n\nLog output before error:\n{}",
                    id,
                    e,
                    logs.join("\n")
                )
            }
        }
    };
    let action = AgentAction::ExecuteSystem {
        id: id.clone(),
        result_summary: if is_error {
            "error".into()
        } else {
            "ok".into()
        },
    };

    (output, is_error, Some(action))
}
