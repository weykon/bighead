//! 系统定义类型从 [`big_head_ecs`] 重新导出。
//! 此模块仅保留需要 AI 依赖的工厂函数。

pub use big_head_ecs::{SystemDefinition, SystemKind, SystemRegistry};

/// 通用 ECS coordinator 系统定义。
pub fn coordinator_system() -> SystemDefinition {
    SystemDefinition {
        id: "coordinator".into(),
        description: "General-purpose ECS world manipulation agent".into(),
        system_prompt: std::borrow::Cow::Borrowed(big_head_ai::COORDINATOR_SYSTEM_PROMPT),
        kind: SystemKind::Llm,
        tool_names: vec![
            "create_activity".into(),
            "update_activity_plan".into(),
            "control_activity".into(),
            "inspect_activity".into(),
            "dream_task".into(),
            "dream_policy".into(),
            "get_world_overview".into(),
            "inspect_runtime".into(),
            "inspect_transaction".into(),
            "inspect_ecs_capabilities".into(),
            "update_system".into(),
            "remove_system".into(),
            "inspect_project".into(),
            "inspect_shard_retention".into(),
            "inspect_shards".into(),
            "link_shard_sources".into(),
            "set_shard_retention".into(),
            "define_lens_app".into(),
            "describe_lens_contract".into(),
            "inspect_world_shape".into(),
            "discover_lenses".into(),
            "record_lens_feedback".into(),
            "propose_lens_candidate".into(),
            "trial_lens_candidate".into(),
            "adopt_lens_candidate".into(),
            "inspect_lens_learning".into(),
            "define_lens_binding".into(),
            "inspect_lens_bindings".into(),
            "run_lens_app".into(),
            "inspect_lens_apps".into(),
            "record_fact".into(),
            "record_interpretation".into(),
            "define_interpretation_method".into(),
            "inspect_interpretation_methods".into(),
            "select_interpretation_path".into(),
            "materialize_interpretation".into(),
            "inspect_interpretations".into(),
            "query_entities".into(),
            "query_ecs".into(),
            "get_entity".into(),
            "get_component".into(),
            "spawn_entity".into(),
            "modify_component".into(),
            "remove_component".into(),
            "despawn_entity".into(),
            "define_component".into(),
            "define_system".into(),
            "run_system".into(),
            "execute_cli".into(),
            "push_spark".into(),
            "set_session_context".into(),
            "push_ambient_signal".into(),
            "shard_content".into(),
            "get_shards".into(),
            "get_shard_content".into(),
            "search".into(),
            "fetch_url".into(),
        ],
        max_iterations: 100,
        is_tool: false,
    }
}

/// Built-in LLM stage that interprets TickAction execution facts and authors
/// durable understanding through the normal ECS tool surface.
pub fn tick_steward_system() -> SystemDefinition {
    SystemDefinition {
        id: "tick-steward".into(),
        description: "Observe TickAction reports and steward their ECS state".into(),
        system_prompt: std::borrow::Cow::Borrowed(big_head_ai::TICK_STEWARD_SYSTEM_PROMPT),
        kind: SystemKind::Llm,
        tool_names: vec![
            "inspect_runtime".into(),
            "get_world_overview".into(),
            "inspect_world_shape".into(),
            "query_entities".into(),
            "query_ecs".into(),
            "get_entity".into(),
            "get_component".into(),
            "define_component".into(),
            "spawn_entity".into(),
            "modify_component".into(),
            "update_tick_action".into(),
        ],
        max_iterations: std::env::var("BIGHEAD_TICK_STEWARD_MAX_ITERATIONS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| (1..=1000).contains(value))
            .unwrap_or(100),
        is_tool: false,
    }
}

/// A read-only observation pass over a bounded sequence of interpretations.
pub fn interpretation_observer_system() -> SystemDefinition {
    SystemDefinition {
        id: "interpretation-observer".into(),
        description: "Observe how a scoped sequence of interpretations changes over time".into(),
        system_prompt: std::borrow::Cow::Borrowed(include_str!("experience_observer_prompt.md")),
        kind: SystemKind::Llm,
        tool_names: vec![],
        max_iterations: 2,
        is_tool: false,
    }
}

/// 将所有内置指南 system 注册到指定注册表。
///
/// 指南清单由 `big_head_ai::all_builtin_guides()` 统一维护；这里不复制静态列表。
/// 全部 `is_tool: false` —— LLM 必须用 `run_system("guide_xxx")` 显式调用。
/// 重复注册会返回 error（调用方决定是否忽略）。
pub fn register_builtin_guides(reg: &mut SystemRegistry) -> Result<(), String> {
    for g in big_head_ai::all_builtin_guides() {
        if let Err(e) = reg.register(g) {
            return Err(e);
        }
    }
    Ok(())
}
