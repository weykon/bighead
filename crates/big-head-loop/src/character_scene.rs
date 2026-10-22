//! Scene identity and placement live in ECS; language and page actions share this state.
use crate::{AgentRuntime, character_demo::CharacterDemo, character_room::RoomReceipt};
use bevy_ecs::{component::Component, entity::Entity};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct RoomSceneItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size: [f32; 3],
    pub footprint: [f32; 2],
    pub allowed_surfaces: Vec<String>,
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RoomPlacement {
    pub surface: String,
    pub position: [f32; 3],
    pub revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoomMove {
    pub object_id: String,
    pub target: String,
    pub expected_revision: u64,
    /// Optional world-space position. Omit to let the runtime choose a free spot.
    pub position: Option<[f32; 3]>,
}

pub(crate) fn location_name(id: &str) -> &str {
    match id {
        "floor" => "地上",
        "desk" => "桌上",
        "shelf" => "书架上",
        _ => id,
    }
}

impl AgentRuntime {
    pub(crate) fn ensure_room_scene(&mut self) {
        let existing: Vec<String> = self
            .world
            .query::<&RoomSceneItem>()
            .iter(&self.world)
            .map(|o| o.id.clone())
            .collect();
        for (id, name, size, position, surface, movable) in [
            (
                "floor",
                "地面",
                [5.9, 0.1, 4.3],
                [0.0, 0.0, 0.0],
                "room",
                false,
            ),
            (
                "desk",
                "桌子",
                [2.7, 1.0, 1.3],
                [0.45, 0.1, 0.45],
                "floor",
                false,
            ),
            (
                "shelf",
                "书架",
                [2.65, 1.0, 0.48],
                [1.1, 0.75, -1.68],
                "wall",
                false,
            ),
            (
                "lamp",
                "台灯",
                [0.58, 0.87, 0.58],
                [1.55, 1.1, 0.18],
                "desk",
                true,
            ),
        ] {
            if !existing.iter().any(|old| old == id) {
                self.world.spawn((
                    RoomSceneItem {
                        id: id.into(),
                        name: name.into(),
                        kind: id.into(),
                        size,
                        footprint: if movable {
                            [0.38, 0.38]
                        } else {
                            [size[0], size[2]]
                        },
                        allowed_surfaces: if movable {
                            vec!["desk".into(), "floor".into()]
                        } else {
                            vec![]
                        },
                    },
                    RoomPlacement {
                        surface: surface.into(),
                        position,
                        revision: 1,
                    },
                ));
            }
        }
    }

    pub(crate) fn scene_placements(&mut self) -> Vec<(Entity, RoomPlacement)> {
        self.world
            .query::<(Entity, &RoomPlacement)>()
            .iter(&self.world)
            .map(|(e, p)| (e, p.clone()))
            .collect()
    }

    pub(crate) fn restore_scene_placements(&mut self, before: Vec<(Entity, RoomPlacement)>) {
        for (e, p) in before {
            self.world.entity_mut(e).insert(p);
        }
    }

    pub(crate) fn scene_inventory(&mut self) -> Value {
        let mut rows: Vec<_> = self.world.query::<(Entity, &RoomSceneItem, &RoomPlacement)>().iter(&self.world)
            .map(|(e,item,placement)| json!({"entity_id":e.index().index(), "item":item, "placement":placement})).collect();
        rows.sort_by_key(|r| r["item"]["id"].as_str().unwrap_or_default().to_owned());
        json!(rows)
    }

    pub(crate) fn apply_room_move(
        &mut self,
        character: Entity,
        action: &RoomMove,
    ) -> Result<String, String> {
        let turn_id = self
            .world
            .get::<CharacterDemo>(character)
            .and_then(|s| s.turns.last())
            .ok_or("Missing source turn")?
            .id
            .clone();
        let scene: Vec<_> = self
            .world
            .query::<(Entity, &RoomSceneItem, &RoomPlacement)>()
            .iter(&self.world)
            .map(|(e, i, p)| (e, i.clone(), p.clone()))
            .collect();
        let origin;
        let message = if let Some((entity, item, before)) =
            scene.iter().find(|(_, i, _)| i.id == action.object_id)
        {
            origin = serde_json::to_value(before).unwrap();
            if before.revision != action.expected_revision {
                return Err("Scene object changed; use its current revision".into());
            }
            if !item.allowed_surfaces.contains(&action.target) {
                return Err(format!(
                    "{} cannot be placed on {}; allowed surfaces: {:?}",
                    item.name, action.target, item.allowed_surfaces
                ));
            }
            let (_, support, base) = scene
                .iter()
                .find(|(_, i, _)| i.id == action.target)
                .ok_or("Target surface is missing")?;
            let height = base.position[1] + support.size[1];
            let candidates = match action.position {
                Some(p) => vec![p],
                None if action.target == "desk" => {
                    vec![[base.position[0] + 1.1, height, base.position[2] - 0.27]]
                }
                None => vec![
                    [base.position[0] + 2.25, height, base.position[2] + 0.55],
                    [base.position[0] - 0.3, height, base.position[2] + 1.65],
                ],
            };
            let position = candidates.into_iter().map(|p| p.map(|v| (v * 1000.0).round() / 1000.0)).find(|p| {
                p.iter().all(|v|v.is_finite()) && (p[1]-height).abs()<0.001
                    && [0,2].iter().enumerate().all(|(n,&axis)| (p[axis]-base.position[axis]).abs()+item.footprint[n]/2.0 <= support.size[axis]/2.0+0.001)
                    && !scene.iter().any(|(e,other,other_p)| e!=entity && other.id!=action.target && other.id!="floor"
                        && overlaps(*p,item.size,other_p.position,other.size))
                    // The companion occupies this standing area beside the desk.
                    && !overlaps(*p,item.size,[-1.8,0.1,0.5],[1.15,2.0,0.8])
                    // Reserve note slots too, so later note creation cannot intersect furniture.
                    && !["desk","shelf","floor"].iter().any(|location| (0..8).any(|index| {
                        let (position,size)=note_bounds(location,index);
                        overlaps(*p,item.size,position,size)
                    }))
            }).ok_or("No valid free position: outside support, wrong height, or overlaps another object")?;
            let after = RoomPlacement {
                surface: action.target.clone(),
                position,
                revision: before.revision + 1,
            };
            if before.surface == after.surface && before.position == after.position {
                format!(
                    "{}已经在{}，位置没有改变。",
                    item.name,
                    location_name(&after.surface)
                )
            } else {
                let operation = big_head_ai::AgentAction::ModifyComponent {
                    entity_id: entity.index().index() as u64,
                    component: "RoomPlacement".into(),
                    value: serde_json::to_value(&after).unwrap(),
                };
                let result = crate::registry::apply_action(
                    &mut self.world,
                    &operation,
                    &mut self.component_registry,
                );
                if result.is_err() || self.world.get::<RoomPlacement>(*entity) != Some(&after) {
                    self.world.entity_mut(*entity).insert(before.clone());
                    return Err(result
                        .err()
                        .unwrap_or_else(|| "Placement readback did not match".into()));
                }
                format!("已把{}放到{}。", item.name, location_name(&after.surface))
            }
        } else {
            let (entity, mut object) = self
                .room_objects()
                .into_iter()
                .find(|(_, o)| o.id == action.object_id)
                .ok_or("Unknown room object; use an ID from the inventory")?;
            origin = json!({"surface":object.location,"revision":object.revision});
            if object.revision != action.expected_revision {
                return Err("Room object changed; use its current revision".into());
            }
            if !["desk", "shelf", "floor"].contains(&action.target.as_str())
                || action.position.is_some()
            {
                return Err("Notes support desk/shelf/floor with automatic placement only".into());
            }
            object.location = action.target.clone();
            object.revision += 1;
            let message = format!(
                "已把「{}」放到{}。",
                object.title,
                location_name(&object.location)
            );
            self.world.entity_mut(entity).insert(object);
            message
        };
        self.world
            .get_mut::<CharacterDemo>(character)
            .unwrap()
            .record_room_receipt(RoomReceipt {
                id: format!("move-{turn_id}"),
                input_json:
                    json!({"action":"move","from":origin,"to":action.target,"verified":true})
                        .to_string(),
                message: message.clone(),
                object_id: action.object_id.clone(),
            });
        Ok(message)
    }
}

fn overlaps(a: [f32; 3], size_a: [f32; 3], b: [f32; 3], size_b: [f32; 3]) -> bool {
    a[1] < b[1] + size_b[1] - 0.001
        && a[1] + size_a[1] > b[1] + 0.001
        && [0, 2]
            .iter()
            .all(|&i| (a[i] - b[i]).abs() < (size_a[i] + size_b[i]) / 2.0 - 0.001)
}

fn note_bounds(location: &str, index: usize) -> ([f32; 3], [f32; 3]) {
    match location {
        "shelf" => (
            [
                0.12 + (index % 4) as f32 * 0.6,
                if index < 4 { 1.745 } else { 0.845 },
                -1.65,
            ],
            [0.4, 0.29, 0.14],
        ),
        "floor" => (
            [
                -0.8 + (index % 4) as f32 * 0.49,
                0.1,
                1.45 + (index / 4) as f32 * 0.4,
            ],
            [0.41, 0.08, 0.34],
        ),
        _ => (
            [
                -0.53 + (index % 4) as f32 * 0.49,
                1.1,
                0.22 + (index / 4) as f32 * 0.5,
            ],
            [0.41, 0.08, 0.34],
        ),
    }
}

#[cfg(test)]
mod tests;
