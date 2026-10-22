//! A deliberately local interaction path for the character demo.
use bevy_ecs::{component::Component, entity::Entity};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AgentRuntime, SystemDefinition, SystemKind,
    character_room::{RoomReceipt, public_objects},
    scene_world::{WORLD_PROMPT, WORLD_TOOLS},
};

pub const PANEL_ID: &str = "character-demo";
const CHARACTER_MAX_ITERATIONS_ENV: &str = "BIGHEAD_CHARACTER_MAX_ITERATIONS";
const DEFAULT_CHARACTER_MAX_ITERATIONS: u32 = 100;

fn parse_character_max_iterations(value: Option<&str>) -> u32 {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| (1..=1000).contains(value))
        .unwrap_or(DEFAULT_CHARACTER_MAX_ITERATIONS)
}

fn character_max_iterations() -> u32 {
    parse_character_max_iterations(std::env::var(CHARACTER_MAX_ITERATIONS_ENV).ok().as_deref())
}

fn character_failure(error: &str) -> (&'static str, String) {
    let detail: String = error.chars().take(300).collect();
    if error.contains("IterationLimit") {
        (
            "partial",
            format!(
                "本轮达到角色执行预算，未能完整收束。已经成功执行的世界变化仍然保留；继续当前任务时不需要重做。\n\n运行原因：{detail}"
            ),
        )
    } else {
        (
            "error",
            format!("这次回应未完成；已执行的世界变化仍然保留。\n\n运行原因：{detail}"),
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct CharacterTurn {
    pub id: String,
    pub text: String,
    pub reply: String,
    pub status: String,
}

#[derive(Component, Clone, Default, Serialize, Deserialize, Reflect)]
pub struct CharacterDemo {
    #[serde(default)]
    pub scene_initialized: bool,
    pub visits: u64,
    pub turns: Vec<CharacterTurn>,
    #[serde(default)]
    pub activity: String,
    #[serde(default)]
    pub room_changes: Vec<RoomReceipt>,
    #[serde(default)]
    pub last_event_id: String,
    #[serde(default)]
    pub notice: String,
}

impl CharacterDemo {
    fn accept(&mut self, input: &Value) -> Result<bool, String> {
        let id = input["interaction_id"].as_str().unwrap_or_default();
        let text = input["text"].as_str().unwrap_or_default().trim();
        if id.is_empty()
            || id.len() > 80
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("Invalid interaction ID".into());
        }
        if text.is_empty() || text.chars().count() > 1000 {
            return Err("Message must contain 1..1000 characters".into());
        }
        if let Some(previous) = self.turns.iter().find(|turn| turn.id == id) {
            return if previous.text == text {
                Ok(false)
            } else {
                Err("Interaction ID already used for another message".into())
            };
        }
        self.visits += 1;
        self.last_event_id = id.into();
        self.notice.clear();
        self.turns.push(CharacterTurn {
            id: id.into(),
            text: text.into(),
            reply: String::new(),
            status: "thinking".into(),
        });
        if self.turns.len() > 16 {
            self.turns.remove(0);
        }
        Ok(true)
    }
}

impl AgentRuntime {
    fn character_entity(&mut self) -> Entity {
        if let Some(entity) = self
            .world_mut()
            .query_filtered::<Entity, bevy_ecs::query::With<CharacterDemo>>()
            .iter(self.world())
            .next()
        {
            entity
        } else {
            self.world_mut().spawn(CharacterDemo::default()).id()
        }
    }

    pub fn publish_character_demo(&mut self, recover: bool) {
        let entity = self.character_entity();
        self.ensure_programmable_scene();
        let mut state = self.world_mut().get_mut::<CharacterDemo>(entity).unwrap();
        if recover {
            for turn in &mut state.turns {
                if turn.status == "thinking" {
                    turn.status = "interrupted".into();
                    turn.reply = "上次回应因重启中断，请重新发送。".into();
                }
            }
        }
        let mut state = serde_json::to_value(&*state).unwrap();
        state["objects"] = public_objects(self.room_objects());
        state["scene"] = self.scene_inventory();
        state["world_projection"] = crate::scene_world::projection(&mut self.world);
        if let Some(channel) = self.view_handler.as_ref().and_then(|h| h.channel.as_ref()) {
            let command = serde_json::from_value(json!({
                "op":"spawn", "id":PANEL_ID, "surface":"html2d", "kind":"a2ui",
                "title":"小小世界", "position":[0,0,0],
                "content":{"kind":"a2ui", "catalog_id":"default", "components":[
                    {"id":"character-state", "component":"CharacterDemo", "state":state}
                ]}
            }))
            .expect("character panel command");
            channel.send(command);
        }
    }

    pub async fn interact_with_character(&mut self, input: Value) -> Result<(), String> {
        if input["action"] == "sync" {
            self.publish_character_demo(false);
            return Ok(());
        }
        if input["action"] == "work_initialize" || input["action"] == "work_enter" {
            let result = self.work_event(&input);
            let entity = self.character_entity();
            let mut state = self.world.get_mut::<CharacterDemo>(entity).unwrap();
            state.last_event_id = input["interaction_id"].as_str().unwrap_or_default().into();
            state.notice = result
                .as_ref()
                .err()
                .map(|e| format!("街区操作未完成：{e}；已执行的改变可能仍然保留。"))
                .unwrap_or_default();
            self.publish_character_demo(false);
            return result;
        }
        if input["action"]
            .as_str()
            .is_some_and(|s| s.starts_with("room_"))
        {
            let entity = self.character_entity();
            let result = self.room_event(entity, &input);
            if let Err(error) = &result {
                let mut state = self.world.get_mut::<CharacterDemo>(entity).unwrap();
                state.last_event_id = input["interaction_id"].as_str().unwrap_or_default().into();
                state.notice = error.clone();
            }
            self.publish_character_demo(false);
            return result;
        }
        if input["action"] != "greet" && input["action"] != "say" {
            return Err("Unknown character action".into());
        }
        let entity = self.character_entity();
        let accepted = self
            .world_mut()
            .get_mut::<CharacterDemo>(entity)
            .unwrap()
            .accept(&input)?;
        self.publish_character_demo(false);
        if !accepted {
            return Ok(());
        }
        // Persist receipt before the API call; a crash must not look like an unseen click.
        if let Err(error) = self.save_delivery_state() {
            let mut state = self.world_mut().get_mut::<CharacterDemo>(entity).unwrap();
            let turn = state.turns.last_mut().unwrap();
            turn.status = "error".into();
            turn.reply = "暂时无法保存这次互动，尚未请求回复。".into();
            self.publish_character_demo(false);
            return Err(error);
        }
        let definition = SystemDefinition {
            id: "character-world-agent-v1".into(),
            description: "Operate the programmable ECS room through tools".into(),
            kind: SystemKind::Llm,
            tool_names: WORLD_TOOLS.iter().map(|s| s.to_string()).collect(),
            max_iterations: character_max_iterations(),
            is_tool: false,
            system_prompt: WORLD_PROMPT.into(),
        };
        let registration = if self.system(&definition.id).is_some() {
            self.system_registry.replace(definition).map(|_| ())
        } else {
            self.register_system(definition)
        };
        let mut context =
            serde_json::to_value(self.world().get::<CharacterDemo>(entity).unwrap()).unwrap();
        context["objects"] = public_objects(self.room_objects());
        context["scene"] = self.scene_inventory();
        context["world_projection"] = crate::scene_world::projection(&mut self.world);
        context["selected_object_id"] = input["object_id"].clone();
        context["current_message"] = input["text"].clone();
        context["active_view"] = input["view"].clone();
        let before_state = self.world().get::<CharacterDemo>(entity).unwrap().clone();
        let old_engine =
            std::mem::replace(&mut self.rhai_engine, crate::scene_world::bounded_engine());
        let result = match registration {
            Ok(()) => {
                self.run_llm_system("character-world-agent-v1", context.to_string())
                    .await
            }
            Err(error) => Err(error),
        };
        self.rhai_engine = old_engine;
        // Keep the interaction receipt even if a script removed the conversation entity.
        let entity = self.character_entity();
        self.world.entity_mut(entity).insert(before_state);
        let mut state = self.world_mut().get_mut::<CharacterDemo>(entity).unwrap();
        let turn = state.turns.last_mut().unwrap();
        match &result {
            Ok(reply) => {
                turn.reply = reply.chars().take(2000).collect();
                turn.status = "replied".into();
            }
            Err(error) => {
                let (status, reply) = character_failure(error);
                turn.reply = reply;
                turn.status = status.into();
            }
        }
        if let Err(error) = self.save_delivery_state() {
            self.mark_scene_dirty();
            let mut state = self.world_mut().get_mut::<CharacterDemo>(entity).unwrap();
            let turn = state.turns.last_mut().unwrap();
            turn.status = "error".into();
            turn.reply
                .push_str("\n这次互动尚未成功保存，重启后可能丢失。\n");
            self.publish_character_demo(false);
            return Err(error);
        }
        self.publish_character_demo(false);
        result.map(|_| ())
    }

    #[cfg(test)]
    fn apply_character_reply(&mut self, entity: Entity, text: &str) -> Result<String, String> {
        let reply = crate::character_room::parse_reply(text)?;
        if reply.change.is_some() && reply.movement.is_some() {
            return Err("Propose a move OR a note change, not both in one response".into());
        }
        let outcome = if let Some(movement) = &reply.movement {
            Some(self.apply_room_move(entity, movement)?)
        } else {
            None
        };
        if let Some(proposal) = &reply.change {
            self.apply_room_proposal(entity, proposal)?;
        }
        self.world
            .get_mut::<CharacterDemo>(entity)
            .unwrap()
            .activity = reply.activity;
        // Movement completion is described from verified ECS readback, not the proposed prose.
        Ok(outcome.unwrap_or(reply.reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_deduplicate_validate_and_bound_history() {
        let mut state = CharacterDemo::default();
        let request = json!({"interaction_id":"one", "text":"早上好"});
        assert!(state.accept(&request).unwrap());
        assert!(!state.accept(&request).unwrap());
        assert!(
            state
                .accept(&json!({"interaction_id":"one", "text":"changed"}))
                .is_err()
        );
        assert!(
            state
                .accept(&json!({"interaction_id":"two", "text":" "}))
                .is_err()
        );
        for n in 0..30 {
            state
                .accept(&json!({"interaction_id":format!("id-{n}"), "text":"hello"}))
                .unwrap();
        }
        assert_eq!(state.turns.len(), 16);
        assert_eq!(state.visits, 31);
    }

    #[test]
    fn character_iteration_budget_defaults_to_100_and_rejects_invalid_values() {
        assert_eq!(parse_character_max_iterations(None), 100);
        assert_eq!(parse_character_max_iterations(Some("250")), 250);
        assert_eq!(parse_character_max_iterations(Some("0")), 100);
        assert_eq!(parse_character_max_iterations(Some("1001")), 100);
        assert_eq!(parse_character_max_iterations(Some("nope")), 100);
    }

    #[test]
    fn iteration_limit_is_reported_as_partial_progress_with_the_real_cause() {
        let (status, reply) = character_failure(
            "character-world-agent-v1: incomplete stage (IterationLimit): reached 100 iteration limit",
        );
        assert_eq!(status, "partial");
        assert!(reply.contains("已经成功执行的世界变化仍然保留"));
        assert!(reply.contains("IterationLimit"));
    }

    #[test]
    fn character_survives_restore_and_interrupted_turn_is_visible() {
        let mut rt = AgentRuntime::new_for_test();
        let entity = rt.character_entity();
        rt.world_mut()
            .get_mut::<CharacterDemo>(entity)
            .unwrap()
            .accept(&json!({"interaction_id":"one", "text":"hello"}))
            .unwrap();
        let path = std::env::temp_dir().join(format!("character-{}.json", uuid::Uuid::new_v4()));
        rt.checkpoint(&path).unwrap();
        let mut restored = AgentRuntime::new_for_test();
        restored.restore(&path).unwrap();
        restored.publish_character_demo(true);
        let entity = restored.character_entity();
        let state = restored.world().get::<CharacterDemo>(entity).unwrap();
        assert_eq!(state.visits, 1);
        assert_eq!(state.turns[0].status, "interrupted");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejected_reply_can_be_corrected_without_duplicate_room_writes() {
        let mut rt = AgentRuntime::new_for_test();
        let entity = rt.character_entity();
        rt.world
            .get_mut::<CharacterDemo>(entity)
            .unwrap()
            .accept(&json!({"interaction_id":"proposal-1", "text":"雨天散步"}))
            .unwrap();
        let mut reply = json!({"reply":"留张便签", "activity":"listening", "change":{
            "object_id":null, "title":"雨天", "body":"雨天散步", "kind":"note", "source_quote":"并不存在的原话", "reason":"稍后继续聊"
        }});
        assert!(
            rt.apply_character_reply(entity, &reply.to_string())
                .is_err()
        );
        assert!(rt.room_objects().is_empty());
        assert!(
            rt.world
                .get::<CharacterDemo>(entity)
                .unwrap()
                .room_changes
                .is_empty()
        );
        reply["change"]["source_quote"] = json!("雨天散步");
        rt.apply_character_reply(entity, &reply.to_string())
            .unwrap();
        assert_eq!(rt.room_objects().len(), 1);
        assert_eq!(
            rt.world
                .get::<CharacterDemo>(entity)
                .unwrap()
                .room_changes
                .len(),
            1
        );
    }

    #[test]
    fn movement_reply_comes_from_readback_and_mixed_operations_are_rejected() {
        let mut rt = AgentRuntime::new_for_test();
        rt.ensure_room_scene();
        let entity = rt.character_entity();
        rt.world
            .get_mut::<CharacterDemo>(entity)
            .unwrap()
            .accept(&json!({"interaction_id":"move-1","text":"把台灯放到地上"}))
            .unwrap();
        let mut reply = json!({"reply":"我没有移动能力", "activity":"listening", "change":null, "move":{"object_id":"lamp","target":"floor","expected_revision":1}});
        reply["change"] = json!({"object_id":null,"title":"台灯","body":"台灯","kind":"note","source_quote":"台灯","reason":"test"});
        let before = rt.scene_inventory();
        assert!(
            rt.apply_character_reply(entity, &reply.to_string())
                .is_err()
        );
        assert_eq!(before, rt.scene_inventory());
        assert!(rt.room_objects().is_empty());
        reply["change"] = Value::Null;
        assert_eq!(
            rt.apply_character_reply(entity, &reply.to_string())
                .unwrap(),
            "已把台灯放到地上。"
        );
        assert_eq!(
            rt.world
                .get::<CharacterDemo>(entity)
                .unwrap()
                .room_changes
                .len(),
            1
        );
    }
}
