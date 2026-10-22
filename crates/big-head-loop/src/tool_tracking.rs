//! Runtime receipts for attempts, independent of model completion claims.
use bevy_ecs::{component::Component, entity::Entity, resource::Resource, world::World};
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Serialize, Deserialize)]
pub struct ToolExecution {
    #[serde(default)]
    pub sequence: u64,
    pub trace_id: String,
    pub tx_id: String,
    pub tool_call_id: String,
    pub tool: String,
    pub wave: usize,
    pub status: String,
    pub input_excerpt: String,
    pub output_excerpt: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub elapsed_ms: Option<u64>,
}

#[derive(Resource)]
struct Sequence(u64);

pub(crate) fn start(
    world: &mut World,
    tx_id: &str,
    call: &big_head_ai::ToolCall,
    wave: usize,
) -> Entity {
    if !world.contains_resource::<Sequence>() {
        let last = world
            .query::<&ToolExecution>()
            .iter(world)
            .map(|t| t.sequence)
            .max()
            .unwrap_or(0);
        world.insert_resource(Sequence(last));
    }
    let sequence = {
        let mut counter = world.resource_mut::<Sequence>();
        counter.0 += 1;
        counter.0
    };
    let trace = ToolExecution {
        sequence,
        trace_id: uuid::Uuid::new_v4().to_string(),
        tx_id: tx_id.into(),
        tool_call_id: call.id.clone(),
        tool: call.name.clone(),
        wave,
        status: "running".into(),
        input_excerpt: crate::experience::excerpt(&call.input.to_string(), 4000),
        output_excerpt: String::new(),
        started_at: crate::audit::now_secs(),
        finished_at: None,
        elapsed_ms: None,
    };
    crate::audit::record_tool_execution(&trace);
    world.spawn(trace).id()
}

pub(crate) fn recover(world: &mut World) {
    world.remove_resource::<Sequence>();
    for mut trace in world.query::<&mut ToolExecution>().iter_mut(world) {
        if trace.status == "running" {
            trace.status = "interrupted_unknown".into();
            trace.output_excerpt = "Restored an unfinished attempt. Check external state before retrying; interruption does not prove no effect.".into();
        }
    }
}

pub(crate) fn event(trace: &ToolExecution) -> big_head_state::AgentActivity {
    big_head_state::AgentActivity::ToolExecution {
        trace_id: trace.trace_id.clone(),
        tx_id: trace.tx_id.clone(),
        tool_call_id: trace.tool_call_id.clone(),
        tool: trace.tool.clone(),
        status: trace.status.clone(),
        elapsed_ms: trace.elapsed_ms,
    }
}
