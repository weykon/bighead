//! `define_system` 工具分发 —— 向 [`SystemRegistry`] 注册新的
//! Rhai 脚本系统。

use std::borrow::Cow;

use big_head_ai::{AgentAction, ToolCall};
use rhai::Engine;

use crate::scripting;
use crate::system_def::{SystemDefinition, SystemKind, SystemRegistry};

/// 分发 `define_system` 工具。返回 `(output, is_error, action)`。
pub(crate) fn dispatch_define_system(
    system_registry: &mut SystemRegistry,
    rhai_engine: &Engine,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let id = match call.input.get("id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return ("define_system: missing 'id' parameter".into(), true, None),
    };

    let description = call
        .input
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let source = match call.input.get("source").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return (
                "define_system: missing 'source' parameter (Rhai script)".into(),
                true,
                None,
            );
        }
    };

    // 注册前校验语法
    if let Err(e) = scripting::check_syntax(rhai_engine, &source) {
        return (format!("define_system: {}", e), true, None);
    }

    let max_iterations = call
        .input
        .get("max_iterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    let tool_names: Vec<String> = call
        .input
        .get("tool_names")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let is_tool = call
        .input
        .get("is_tool")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let def = SystemDefinition {
        id: id.clone(),
        description: description.clone(),
        system_prompt: Cow::Owned(String::new()),
        kind: SystemKind::RhaiScript {
            source: source.clone(),
        },
        tool_names: tool_names.clone(),
        max_iterations,
        is_tool,
    };

    if let Err(e) = system_registry.register(def) {
        return (format!("define_system failed: {}", e), true, None);
    }

    let action = AgentAction::DefineSystem {
        id: id.clone(),
        description: description.clone(),
        kind: "rhai".into(),
        source: Some(source.clone()),
        tool_names: tool_names.clone(),
        max_iterations,
        is_tool,
    };
    (
        format!(
            "System '{}' defined successfully (Rhai script). Use run_system(\"{}\") to execute.\nDescription: {}",
            id, id, description
        ),
        false,
        Some(action),
    )
}

/// 分发 `define_tool` 工具 —— 注册 Rhai 脚本为 LLM 可见 tool。
/// 等价于 `define_system` 且 `is_tool: true`。
pub(crate) fn dispatch_define_tool(
    system_registry: &mut SystemRegistry,
    rhai_engine: &Engine,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let id = match call.input.get("id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return ("define_tool: missing 'id' parameter".into(), true, None),
    };

    let description = call
        .input
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let source = match call.input.get("source").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return (
                "define_tool: missing 'source' parameter (Rhai script)".into(),
                true,
                None,
            );
        }
    };

    if let Err(e) = scripting::check_syntax(rhai_engine, &source) {
        return (format!("define_tool: {}", e), true, None);
    }

    let def = SystemDefinition {
        id: id.clone(),
        description: description.clone(),
        system_prompt: Cow::Owned(String::new()),
        kind: SystemKind::RhaiScript {
            source: source.clone(),
        },
        tool_names: Vec::new(),
        max_iterations: 1,
        is_tool: true,
    };

    if let Err(e) = system_registry.register(def) {
        return (format!("define_tool failed: {}", e), true, None);
    }

    let action = AgentAction::DefineSystem {
        id: id.clone(),
        description: description.clone(),
        kind: "rhai".into(),
        source: Some(source.clone()),
        tool_names: Vec::new(),
        max_iterations: 1,
        is_tool: true,
    };
    (
        format!(
            "Tool '{}' defined successfully. It will appear in the next tool wave, subject to the current stage's allowlist.\nDescription: {}",
            id, description
        ),
        false,
        Some(action),
    )
}
