//! Shared executor fixtures; behavior tests are grouped by runtime responsibility.

use super::recovery::append_recovery_hint;
use super::*;
use crate::{AgentRuntime, components::Spark};
use bevy_ecs::component::Component;
use big_head_ai::{ToolCall, ToolExecutor};
use big_head_world::{WorldIdentity, WorldPaths};

mod core;
mod interaction_flow;
mod activity;
mod evidence;
mod lens;
mod laya;
mod projection;
mod systems;
mod view;

#[derive(Component, serde::Serialize, serde::Deserialize)]
struct TestHealth(f32);

#[derive(Component, serde::Serialize, serde::Deserialize)]
struct TestName(String);

fn make_executor<'a>(rt: &'a mut AgentRuntime) -> RuntimeToolExecutor<'a> {
    RuntimeToolExecutor {
        world: &mut rt.world,
        registry: &mut rt.component_registry,
        system_registry: &mut rt.system_registry,
        rhai_engine: &rt.rhai_engine,
        spark_queue: &mut rt.spark_queue,
        actions_applied: Vec::new(),
        log_emitter: None,
        state_hook: None,
        cli_executor: &rt.cli_executor,
        cli_security: &rt.cli_security,
        sharder: &rt.sharder,
        view_handler: rt.view_handler.clone(),
        world_manifest: rt.world_manifest.as_ref(),
        last_tool_result: None,
        tool_call_log: Vec::new(),
        current_wave: 0,
        tx_id: None,
        cancel_flag: &rt.cancel_flag,
        allowed_tools: None,
    }
}

fn temp_manifest(label: &str) -> WorldManifest {
    let root = std::env::temp_dir().join(format!("big-head-lens-{label}-{}", uuid::Uuid::new_v4()));
    WorldManifest {
        identity: WorldIdentity::local(format!("test:{label}")),
        paths: WorldPaths {
            checkpoint_path: root.join("checkpoint.json"),
            view_snapshot_path: root.join("view_snapshot.json"),
            inbox_dir: root.join("lens").join("inbox"),
            outbox_dir: root.join("lens").join("outbox"),
            root_dir: root,
        },
        peers: Vec::new(),
    }
}
