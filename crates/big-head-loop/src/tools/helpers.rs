//! 只读工具辅助函数和小型工具，供工具分发子模块共享。

use bevy_ecs::world::World;

pub(crate) use big_head_ecs::{query_reader, snapshot_reader};
use big_head_query::{execute_dynamic_query_with_reader, parse_query};
pub(crate) use big_head_save::read::{execute_get_component, execute_get_entity};

use big_head_ai::log::{LogEmitter, LogLevel, emit};

use crate::schema::{DynamicComponentStore, DynamicSchema};

/// 通过 emitter 或 stderr 记录工具执行错误。
pub(crate) fn log_tool_error(emitter: &Option<LogEmitter>, tool_name: &str, output: &str) {
    emit(
        emitter,
        "tool",
        LogLevel::Error,
        format!("{} -> {}", tool_name, output),
    );
}

/// 执行 `get_world_overview` —— 组件注册表 + 实体索引。
pub(crate) fn execute_get_world_overview(
    world: &World,
    store: &DynamicComponentStore,
) -> (String, bool) {
    use std::fmt::Write;
    let mut out = String::new();

    // 构建短名称 -> schema 查找表，以便在注册表列表中标注动态组件字段
    let dynamic_by_short: std::collections::HashMap<&str, &DynamicSchema> = store
        .iter()
        .map(|(name, schema)| {
            let short = name.rsplit("::").next().unwrap_or(name);
            (short, schema)
        })
        .collect();

    // 组件注册表
    let components = world.components();
    writeln!(out, "## Components").unwrap();
    for info in components.iter_registered() {
        let full = info.name().to_string();
        let short = full.rsplit("::").next().unwrap_or(&full);
        if let Some(schema) = dynamic_by_short.get(short) {
            let fields: Vec<String> = schema
                .fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v.as_str()))
                .collect();
            writeln!(
                out,
                "- {} (dynamic) — fields: {{{}}}",
                full,
                fields.join(", ")
            )
            .unwrap();
        } else {
            writeln!(out, "- {}", full).unwrap();
        }
    }

    // 实体索引（ID + 组件名称）
    writeln!(out, "\n## Entities").unwrap();
    for archetype in world.archetypes().iter() {
        let arch_components: Vec<String> = archetype
            .components()
            .iter()
            .filter_map(|&cid| {
                components.get_info(cid).map(|info| {
                    let full = info.name().to_string();
                    let short = full.rsplit("::").next().unwrap_or(&full).to_string();
                    short
                })
            })
            .collect();

        if arch_components.is_empty() {
            continue;
        }

        for arch_entity in archetype.entities() {
            let entity = arch_entity.id();
            let id = entity.index().index();
            writeln!(out, "- Entity {}: [{}]", id, arch_components.join(", ")).unwrap();
        }
    }

    (out, false)
}

/// 执行 `query_entities` —— 根据查询字符串查找实体。
pub(crate) fn execute_query_entities(
    world: &World,
    store: &DynamicComponentStore,
    query_string: &str,
) -> (String, bool) {
    match parse_query(world, query_string) {
        Ok(query) => {
            let string_reader = query_reader(store);
            let json_reader = snapshot_reader(store);
            let results = execute_dynamic_query_with_reader(
                world,
                &query,
                Some(&string_reader),
                Some(&json_reader),
            );
            if results.is_empty() {
                return ("No entities match the query.".to_string(), false);
            }
            let mut out = String::new();
            for r in &results {
                use std::fmt::Write;
                writeln!(out, "Entity {}:", r.entity.index().index()).unwrap();
                for (name, value) in &r.values {
                    let short_name = name.rsplit("::").next().unwrap_or(name);
                    let val = value.as_deref().unwrap_or("(opaque)");
                    writeln!(out, "  {}: {}", short_name, val).unwrap();
                }
            }
            (out, false)
        }
        Err(e) => (format!("Query error: {}", e), true),
    }
}

// execute_get_entity / execute_get_component 从 big_head_save::read 重新导出
