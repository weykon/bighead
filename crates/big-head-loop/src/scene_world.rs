//! Generic scene projection and scheduling. Behaviors are authored as ordinary Rhai systems.
use crate::{
    AgentRuntime, SystemKind,
    character_scene::{RoomPlacement, RoomSceneItem},
    registry::ComponentActionRegistry,
    system_def::SystemRegistry,
};
use bevy_ecs::{component::Component, entity::Entity, resource::Resource, world::World};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Instant};

#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct SceneName {
    pub id: String,
    pub name: String,
}
impl Default for SceneName {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "物件".into(),
        }
    }
}

#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct SceneTransform {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub rx: f64,
    pub ry: f64,
    pub rz: f64,
    pub sx: f64,
    pub sy: f64,
    pub sz: f64,
}
impl Default for SceneTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            rx: 0.0,
            ry: 0.0,
            rz: 0.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
        }
    }
}

#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct SceneRenderable {
    pub kind: String,
    pub color: String,
    pub visible: bool,
    pub intensity: f64,
}
impl Default for SceneRenderable {
    fn default() -> Self {
        Self {
            kind: "box".into(),
            color: "#df849d".into(),
            visible: true,
            intensity: 0.5,
        }
    }
}

#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct SceneSystem {
    pub system_id: String,
    pub enabled: bool,
    pub elapsed: f64,
    pub runs: u64,
    pub error: String,
}
impl Default for SceneSystem {
    fn default() -> Self {
        Self {
            system_id: String::new(),
            enabled: true,
            elapsed: 0.0,
            runs: 0,
            error: String::new(),
        }
    }
}

#[derive(Resource)]
struct SceneLoop {
    last: Instant,
    last_publish: Instant,
    last_projection: Value,
    engine: Arc<rhai::Engine>,
    cache: HashMap<String, (String, rhai::AST)>,
}
impl Default for SceneLoop {
    fn default() -> Self {
        Self {
            last: Instant::now(),
            last_publish: Instant::now(),
            last_projection: Value::Null,
            engine: bounded_engine(),
            cache: HashMap::new(),
        }
    }
}

pub(crate) fn bounded_engine() -> Arc<rhai::Engine> {
    crate::scripting::bounded_ecs_engine()
}

pub(crate) fn projection(world: &mut World) -> Value {
    let mut objects = Vec::new();
    let mut warnings = Vec::new();
    for (e, t, r, name) in world
        .query::<(
            Entity,
            &SceneTransform,
            &SceneRenderable,
            Option<&SceneName>,
        )>()
        .iter(world)
    {
        if objects.len() >= 128 {
            warnings.push("Only the first 128 renderable entities are projected".to_string());
            break;
        }
        let valid = [
            t.x,
            t.y,
            t.z,
            t.rx,
            t.ry,
            t.rz,
            t.sx,
            t.sy,
            t.sz,
            r.intensity,
        ]
        .iter()
        .all(|v| v.is_finite())
            && [t.x, t.y, t.z].iter().all(|v| v.abs() <= 100.0)
            && [t.sx, t.sy, t.sz].iter().all(|v| *v > 0.0 && *v <= 20.0)
            && (0.0..=20.0).contains(&r.intensity)
            && r.color.len() == 7
            && r.color.starts_with('#')
            && r.color[1..].bytes().all(|b| b.is_ascii_hexdigit())
            && [
                "box", "sphere", "cylinder", "cone", "torus", "plane", "lamp", "desk", "shelf",
                "floor",
            ]
            .contains(&r.kind.as_str());
        if !valid {
            warnings.push(format!(
                "Entity {} has unsupported/invalid rendering data",
                e.index().index()
            ));
            continue;
        }
        objects
            .push(json!({"entity_id":e.index().index(),"name":name,"transform":t,"renderable":r}));
    }
    objects.sort_by_key(|v| v["entity_id"].as_u64());
    let systems: Vec<_> = world
        .query::<(Entity, &SceneSystem)>()
        .iter(world)
        .map(|(e, s)| json!({"entity_id":e.index().index(),"state":s}))
        .collect();
    json!({"objects":objects,"systems":systems,"warnings":warnings,"work":crate::work_world::projection(world)})
}

fn step(
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    systems: &mut SystemRegistry,
    state: &mut SceneLoop,
    dt: f64,
) -> bool {
    let mut active: Vec<_> = world
        .query::<(Entity, &SceneSystem)>()
        .iter(world)
        .filter(|(_, s)| s.enabled)
        .map(|(e, s)| (e, s.clone()))
        .collect();
    active.sort_by_key(|(e, _)| e.index().index());
    let active_ids: Vec<_> = active.iter().map(|(_, s)| s.system_id.clone()).collect();
    state.cache.retain(|id, _| active_ids.contains(id));
    let changed = !active.is_empty();
    for (index, (entity, mut schedule)) in active.into_iter().enumerate() {
        let result = (|| -> Result<(), String> {
            if index >= 16 {
                return Err("Scene scheduler supports at most 16 active systems".into());
            }
            let source = match systems.get(&schedule.system_id).map(|s| &s.kind) {
                Some(SystemKind::RhaiScript { source }) => source.clone(),
                _ => return Err("Scheduled Rhai system does not exist".into()),
            };
            if state
                .cache
                .get(&schedule.system_id)
                .is_none_or(|(old, _)| old != &source)
            {
                let ast = state.engine.compile(&source).map_err(|e| e.to_string())?;
                state
                    .cache
                    .insert(schedule.system_id.clone(), (source, ast));
            }
            let args =
                json!({"dt":dt,"elapsed":schedule.elapsed+dt,"entity_id":entity.index().index()});
            let _ = crate::scripting::eval_ast_with_world_args(
                &state.engine,
                &state.cache[&schedule.system_id].1,
                world,
                registry,
                systems,
                &args,
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })();
        // Scripts may deliberately disable or remove their own scheduler entity.
        if let Some(current) = world.get::<SceneSystem>(entity) {
            if current.system_id != schedule.system_id {
                continue;
            }
            schedule.enabled = current.enabled;
            match result {
                Ok(()) => {
                    schedule.elapsed += dt;
                    schedule.runs += 1;
                    schedule.error.clear();
                }
                Err(e) => {
                    schedule.enabled = false;
                    schedule.error = e.chars().take(500).collect();
                }
            }
            world.entity_mut(entity).insert(schedule);
        }
    }
    changed
}

impl AgentRuntime {
    pub(crate) fn ensure_programmable_scene(&mut self) {
        if self
            .world
            .query::<&crate::character_demo::CharacterDemo>()
            .iter(&self.world)
            .any(|s| s.scene_initialized)
        {
            return;
        }
        self.ensure_room_scene();
        let missing: Vec<_> = self
            .world
            .query::<(Entity, &RoomSceneItem, &RoomPlacement)>()
            .iter(&self.world)
            .filter(|(e, _, _)| self.world.get::<SceneName>(*e).is_none())
            .map(|(e, i, p)| (e, i.clone(), p.clone()))
            .collect();
        for (e, i, p) in missing {
            self.world.entity_mut(e).insert((
                SceneName {
                    id: i.id.clone(),
                    name: i.name,
                },
                SceneTransform {
                    x: p.position[0] as f64,
                    y: p.position[1] as f64,
                    z: p.position[2] as f64,
                    ..Default::default()
                },
                SceneRenderable {
                    kind: i.kind,
                    color: if i.id == "lamp" { "#f0cc69" } else { "#d8e4df" }.into(),
                    ..Default::default()
                },
            ));
        }
        for mut character in self
            .world
            .query::<&mut crate::character_demo::CharacterDemo>()
            .iter_mut(&mut self.world)
        {
            character.scene_initialized = true;
        }
    }

    pub fn tick_programmable_scene(&mut self) {
        let mut state = self
            .world
            .remove_resource::<SceneLoop>()
            .unwrap_or_default();
        let now = Instant::now();
        let dt = (now - state.last).as_secs_f64().clamp(0.0, 0.1);
        state.last = now;
        if step(
            &mut self.world,
            &mut self.component_registry,
            &mut self.system_registry,
            &mut state,
            dt,
        ) {
            self.mark_scene_dirty();
        }
        let publish = now.duration_since(state.last_publish).as_millis() >= 100;
        let snapshot = if publish {
            projection(&mut self.world)
        } else {
            Value::Null
        };
        let changed = publish && snapshot != state.last_projection;
        if publish {
            state.last_publish = now;
            state.last_projection = snapshot;
        }
        self.world.insert_resource(state);
        if changed {
            self.publish_character_demo(false);
        }
    }
}

pub(crate) fn tool_definition() -> big_head_ai::ToolDefinition {
    big_head_ai::ToolDefinition {name:"inspect_scene".into(),description:"Read the actual browser scene projection: renderable ECS entities, transforms, scheduler run counts/errors and rejected visual data. No mutation.".into(),input_schema:json!({"type":"object","properties":{}})}
}

pub(crate) const WORLD_TOOLS: &[&str] = &[
    "inspect_work",
    "record_work",
    "arrange_work",
    "inspect_scene",
    "inspect_ecs_capabilities",
    "inspect_world_shape",
    "get_entity",
    "get_component",
    "query_entities",
    "query_ecs",
    "define_component",
    "undefine_component",
    "spawn_entity",
    "modify_component",
    "remove_component",
    "despawn_entity",
    "define_system",
    "run_system",
    "update_system",
    "remove_system",
];

pub(crate) const WORLD_PROMPT: &str = include_str!("character_world_prompt.md");

#[cfg(test)]
mod tests;
