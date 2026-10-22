//! 内置指南 system —— 文档随 binary 打包，LLM 通过 `run_system` 按需获取。
//!
//! 设计动机：coordinator 主 prompt 不再嵌入长文档，而是指向这些 system。
//! 真正需要时 LLM 调 `run_system("guide_xxx")`，指南内容通过 Rhai `log()` 返回到
//! 上下文一次。静态 prompt 瘦身 ~1500 tokens；首次访问某指南 +500 tokens 一次性成本。
//!
//! 实现细节：每个指南是一个 `SystemDefinition { kind: RhaiScript, is_tool: false }`，
//! script 主体只做一件事 —— `log(r###"<markdown content>"###)`。
//!
//! Markdown 内容约束：不能包含 `"###` 序列（会提前关闭外层 raw string）。
//! 编译时 `concat_guide!` 宏会检查，发现冲突时 panic 提示。

use std::borrow::Cow;

use big_head_ecs::{SystemDefinition, SystemKind};

// ── Markdown 内容（include_str! 编译期内联）──────────────────────────

const GUIDE_RHAI_SYNTAX: &str = include_str!("../prompts/guides/rhai_syntax.md");
const GUIDE_QUERY_DSL: &str = include_str!("../prompts/guides/query_dsl.md");
const GUIDE_SEARCH: &str = include_str!("../prompts/guides/search_guide.md");
const GUIDE_SHARD: &str = include_str!("../prompts/guides/shard_guide.md");
const GUIDE_TICK_ACTION: &str = include_str!("../prompts/guides/tick_action_guide.md");
const GUIDE_PUSH_SPARK: &str = include_str!("../prompts/guides/push_spark_guide.md");
const GUIDE_CLI: &str = include_str!("../prompts/guides/cli_guide.md");
const GUIDE_RENDER_VIEW: &str = include_str!("../prompts/guides/render_view_guide.md");
const GUIDE_AMBIENT: &str = include_str!("../prompts/guides/ambient_guide.md");
const GUIDE_AGENT_MODES: &str = include_str!("../prompts/guides/agent_modes_guide.md");
const GUIDE_VIEW_SYSTEM: &str = include_str!("../prompts/guides/view_system_guide.md");
const GUIDE_SCENE_WORLD: &str = include_str!("../prompts/guides/scene_world_guide.md");

// ── 工厂函数 ──────────────────────────────────────────────────────────

/// Rhai 脚本：`log("...")` —— 将 markdown 内容 escape 后嵌入普通字符串字面量。
fn build_log_script(content: &str) -> String {
    let escaped = content
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "");
    format!("log(\"{}\");", escaped)
}

fn doc_system(id: &str, description: &str, content: &str) -> SystemDefinition {
    SystemDefinition {
        id: id.to_string(),
        description: description.to_string(),
        system_prompt: Cow::Borrowed(""),
        kind: SystemKind::RhaiScript {
            source: build_log_script(content),
        },
        tool_names: vec![],
        max_iterations: 1,
        is_tool: false,
    }
}

pub fn guide_rhai_syntax() -> SystemDefinition {
    doc_system(
        "guide_rhai_syntax",
        "Rhai 脚本语法参考 —— define_system 写脚本前必读",
        GUIDE_RHAI_SYNTAX,
    )
}

pub fn guide_query_dsl() -> SystemDefinition {
    doc_system(
        "guide_query_dsl",
        "query_entities 查询语法 —— 组件过滤、值比较、字段路径",
        GUIDE_QUERY_DSL,
    )
}

pub fn guide_search() -> SystemDefinition {
    doc_system(
        "guide_search",
        "联网搜索 search 工具完整用法（4 个 kind、调用格式、行为规则）",
        GUIDE_SEARCH,
    )
}

pub fn guide_shard() -> SystemDefinition {
    doc_system(
        "guide_shard",
        "上下文分片 shard_content / get_shards 用法 + Spark 维护",
        GUIDE_SHARD,
    )
}

pub fn guide_tick_action() -> SystemDefinition {
    doc_system(
        "guide_tick_action",
        "TickAction 自主触发 component 完整文档",
        GUIDE_TICK_ACTION,
    )
}

pub fn guide_push_spark() -> SystemDefinition {
    doc_system(
        "guide_push_spark",
        "push_spark 任务拆解工具用法",
        GUIDE_PUSH_SPARK,
    )
}

pub fn guide_cli() -> SystemDefinition {
    doc_system(
        "guide_cli",
        "execute_cli shell 执行工具 + background 托管进程登记 + TUI Processes 观察 + 安全规则",
        GUIDE_CLI,
    )
}

pub fn guide_render_view() -> SystemDefinition {
    doc_system(
        "guide_render_view",
        "render_view 可视化面板完整用法（table/chart/card/form/a2ui + 交互 handler）",
        GUIDE_RENDER_VIEW,
    )
}

pub fn guide_ambient() -> SystemDefinition {
    doc_system(
        "guide_ambient",
        "Ambient Dwelling 上下文 cue / signal / 隐私边界 / 命名规则",
        GUIDE_AMBIENT,
    )
}

pub fn guide_agent_modes() -> SystemDefinition {
    doc_system(
        "guide_agent_modes",
        "Agent mode 选择：ReAct / Plan / Supervisor / Critic / Projection / TickAction 通道",
        GUIDE_AGENT_MODES,
    )
}

pub fn guide_view_system() -> SystemDefinition {
    doc_system(
        "guide_view_system",
        "View System：ECS 组件到 3030 display slot / inspector / mind-view 的映射规则",
        GUIDE_VIEW_SYSTEM,
    )
}

pub fn guide_scene_world() -> SystemDefinition {
    doc_system(
        "guide_scene_world",
        "Programmable scene and work world: visual ECS components, Rhai scheduling, hot updates, triggers and verification",
        GUIDE_SCENE_WORLD,
    )
}

/// 返回全部内置指南 system，按 id 排序。
/// 启动时由 runtime 注册到 SystemRegistry，agent 通过 `run_system("guide_xxx")` 访问。
pub fn all_builtin_guides() -> Vec<SystemDefinition> {
    vec![
        doc_system(
            "guide_laya",
            "Local Laya: advisory ECS operation selection, baseline evaluation and evidence boundaries",
            include_str!("../prompts/guides/laya.md"),
        ),
        doc_system(
            "guide_ecs_operation_model",
            "ECS operation model: discover, compose, hot-edit, verify and collect training evidence",
            include_str!("../prompts/guides/ecs_operation_model.md"),
        ),
        doc_system(
            "guide_world_operation_loop",
            "World-owned dynamic facts, Rhai behavior, operation checks, Laya gates and sparse scheduling",
            include_str!("../prompts/guides/world_operation_loop.md"),
        ),
        guide_rhai_syntax(),
        doc_system(
            "guide_activity",
            "Durable business activities: plan, start, wait, recover, independently verify; separate from Spark completion",
            include_str!("../prompts/guides/activity.md"),
        ),
        guide_query_dsl(),
        guide_search(),
        guide_shard(),
        guide_tick_action(),
        guide_push_spark(),
        guide_cli(),
        guide_render_view(),
        guide_ambient(),
        guide_agent_modes(),
        guide_view_system(),
        guide_scene_world(),
        doc_system(
            "guide_dream",
            "Dream-RSI: measured Rhai practice, historical replay and exploration-policy improvement",
            include_str!("../prompts/guides/dream_guide.md"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_guides_have_unique_ids() {
        let guides = all_builtin_guides();
        let mut ids: Vec<&str> = guides.iter().map(|g| g.id.as_str()).collect();
        ids.sort();
        let original_len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "guide ids must be unique");
    }

    #[test]
    fn all_guides_are_rhai_script_and_not_tools() {
        for g in all_builtin_guides() {
            assert!(
                matches!(g.kind, SystemKind::RhaiScript { .. }),
                "[{}] not Rhai",
                g.id
            );
            assert!(!g.is_tool, "[{}] is_tool must be false", g.id);
            assert!(
                g.id.starts_with("guide_"),
                "[{}] must use guide_ prefix",
                g.id
            );
        }
    }

    #[test]
    fn guide_scripts_emit_log_with_full_content() {
        let g = guide_rhai_syntax();
        if let SystemKind::RhaiScript { source } = &g.kind {
            assert!(source.starts_with("log(\""), "script prefix shape");
            assert!(source.ends_with("\");"), "script suffix shape");
            // The escaped content should be present (with \n for newlines)
            let escaped = GUIDE_RHAI_SYNTAX
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "");
            assert!(source.contains(&escaped), "content embedded");
        } else {
            panic!("expected RhaiScript");
        }
    }

    #[test]
    fn scene_world_guide_contains_runtime_contracts() {
        let guide = guide_scene_world();
        assert_eq!(guide.id, "guide_scene_world");
        let SystemKind::RhaiScript { source } = guide.kind else {
            panic!("expected RhaiScript");
        };
        for contract in [
            "SceneRenderable",
            "SceneSystem",
            "WorkAppearance",
            "WorkTrigger 仅提供 args.event，绝无 args.elapsed",
        ] {
            assert!(source.contains(contract), "missing contract: {contract}");
        }
    }

    #[test]
    fn guide_scripts_are_valid_rhai() {
        let mut engine = rhai::Engine::new();
        engine.register_fn("log", |_msg: &str| {});
        for g in all_builtin_guides() {
            if let SystemKind::RhaiScript { source } = &g.kind {
                if let Err(e) = engine.run(source) {
                    panic!("[{}] Rhai syntax error: {}", g.id, e);
                }
            }
        }
    }
}
