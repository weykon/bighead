use super::*;
use crate::character_demo::CharacterTurn;

fn setup() -> (AgentRuntime, Entity) {
    let mut rt = AgentRuntime::new_for_test();
    let entity = rt
        .world
        .spawn(CharacterDemo {
            turns: vec![CharacterTurn {
                id: "turn-1".into(),
                text: "想做雨天散步的小游戏".into(),
                reply: String::new(),
                status: "thinking".into(),
            }],
            ..Default::default()
        })
        .id();
    (rt, entity)
}

fn proposal() -> Proposal {
    Proposal {
        object_id: None,
        title: "雨天散步".into(),
        body: "也许是安静探索的游戏".into(),
        kind: "idea".into(),
        source_quote: "雨天散步".into(),
        reason: "一个还可以继续聊的想法".into(),
    }
}

fn event(object: &RoomObject, action: &str, id: &str) -> Value {
    json!({"action":action,"interaction_id":id,"object_id":object.id,"expected_revision":object.revision})
}

#[test]
fn model_requires_real_quote_and_distinguishes_notes_from_guesses() {
    let (mut rt, entity) = setup();
    let mut p = proposal();
    p.source_quote = "未说过的话".into();
    assert!(rt.apply_room_proposal(entity, &p).is_err());
    p = proposal();
    p.kind = "note".into();
    assert!(rt.room_objects().is_empty());
    p.body = "雨天散步。模型擅自添加的文字不属于原话".into();
    rt.apply_room_proposal(entity, &p).unwrap();
    let object = &rt.room_objects()[0].1;
    assert_eq!(object.body, "雨天散步");
    assert_eq!(object.source_turn_id, "turn-1");
    assert!(!object.user_confirmed);
}

#[test]
fn updates_keep_identity_and_confirmed_objects_are_protected() {
    let (mut rt, entity) = setup();
    rt.apply_room_proposal(entity, &proposal()).unwrap();
    let first = rt.room_objects()[0].clone();
    let mut p = proposal();
    p.object_id = Some(first.1.id.clone());
    p.body = "另一个猜想".into();
    rt.apply_room_proposal(entity, &p).unwrap();
    let current = rt.room_objects()[0].clone();
    assert_eq!(first.0, current.0);
    assert_eq!(first.1.id, current.1.id);
    assert_eq!(current.1.revision, 2);
    rt.room_event(entity, &event(&current.1, "room_confirm", "confirm-1"))
        .unwrap();
    assert!(rt.apply_room_proposal(entity, &p).is_err());
    assert_eq!(rt.room_objects()[0].1.body, "另一个猜想");
}

#[test]
fn room_capacity_is_enforced_but_existing_objects_can_still_change() {
    let (mut rt, entity) = setup();
    for _ in 0..8 {
        rt.apply_room_proposal(entity, &proposal()).unwrap();
    }
    assert!(rt.apply_room_proposal(entity, &proposal()).is_err());
    let mut p = proposal();
    p.object_id = Some(rt.room_objects()[0].1.id.clone());
    rt.apply_room_proposal(entity, &p).unwrap();
    assert_eq!(rt.room_objects().len(), 8);
}

#[test]
fn corrections_retries_stale_writes_and_removal_have_real_effects() {
    let (mut rt, entity) = setup();
    rt.apply_room_proposal(entity, &proposal()).unwrap();
    let first = rt.room_objects()[0].1.clone();
    let mut correction = event(&first, "room_correct", "correction-1");
    correction["body"] = json!("不是解谜，是安静探索");
    rt.room_event(entity, &correction).unwrap();
    rt.room_event(entity, &correction).unwrap();
    let fixed = rt.room_objects()[0].1.clone();
    assert_eq!(fixed.revision, 2);
    assert_eq!(fixed.body, fixed.source_quote);
    assert!(fixed.user_confirmed);
    assert!(
        rt.room_event(entity, &event(&first, "room_move", "stale-1"))
            .is_err()
    );
    correction["body"] = json!("reused ID");
    assert!(rt.room_event(entity, &correction).is_err());
    rt.room_event(entity, &event(&fixed, "room_move", "move-1"))
        .unwrap();
    let moved = rt.room_objects()[0].clone();
    assert_eq!(moved.1.location, "shelf");
    let removal = event(&moved.1, "room_remove", "remove-1");
    rt.room_event(entity, &removal).unwrap();
    rt.room_event(entity, &removal).unwrap();
    assert!(rt.room_objects().is_empty());
    assert!(
        rt.world.get_entity(moved.0).is_err(),
        "No empty ECS entity left behind"
    );
}

#[test]
fn failed_persistence_rolls_back_removal_and_receipts() {
    let (mut rt, entity) = setup();
    rt.apply_room_proposal(entity, &proposal()).unwrap();
    let before = rt.room_objects()[0].1.clone();
    let blocker = std::env::temp_dir().join(format!("room-blocker-{}", uuid::Uuid::new_v4()));
    std::fs::write(&blocker, "file, not directory").unwrap();
    rt.set_checkpoint_path(blocker.join("checkpoint.json"));
    assert!(
        rt.room_event(entity, &event(&before, "room_remove", "failed-1"))
            .is_err()
    );
    let restored = rt.room_objects()[0].1.clone();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
    assert_eq!(
        rt.world
            .get::<CharacterDemo>(entity)
            .unwrap()
            .room_changes
            .len(),
        1
    );
    std::fs::remove_file(blocker).unwrap();
}

#[test]
fn journal_restore_preserves_corrections_and_does_not_resurrect_removed_objects() {
    let (mut rt, entity) = setup();
    let dir = std::env::temp_dir().join(format!("room-restore-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("checkpoint.json");
    rt.set_checkpoint_path(&path);
    rt.apply_room_proposal(entity, &proposal()).unwrap();
    rt.save_delivery_state().unwrap();
    let first = rt.room_objects()[0].1.clone();
    let mut correction = event(&first, "room_correct", "correct-1");
    correction["body"] = json!("安静探索");
    rt.room_event(entity, &correction).unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    let fixed = restored.room_objects()[0].1.clone();
    assert_eq!(first.id, fixed.id);
    assert_eq!(fixed.body, "安静探索");
    assert!(fixed.user_confirmed);
    rt.room_event(entity, &event(&fixed, "room_remove", "remove-1"))
        .unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    assert!(restored.room_objects().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn withdrawal_preserves_components_added_by_other_workflows() {
    #[derive(Component)]
    struct BusinessMarker;
    let (mut rt, character) = setup();
    rt.apply_room_proposal(character, &proposal()).unwrap();
    let (entity, object) = rt.room_objects()[0].clone();
    rt.world.entity_mut(entity).insert(BusinessMarker);
    rt.room_event(character, &event(&object, "room_remove", "remove-composed"))
        .unwrap();
    assert!(rt.world.get::<RoomObject>(entity).is_none());
    assert!(rt.world.get::<BusinessMarker>(entity).is_some());
}

#[test]
fn strict_model_contract_and_legacy_character_state() {
    assert!(
        parse_reply(r#"{"reply":"it is "quiet"","activity":"listening","change":null}"#).is_err()
    );
    assert!(
        parse_reply(r#"{"reply":"早上好","activity":"listening","change":null}"#)
            .unwrap()
            .change
            .is_none()
    );
    assert!(parse_reply(r#"{"reply":"hi","activity":"executing","change":null}"#).is_err());
    assert!(parse_reply(r#"{"reply":"hi","activity":"listening","run_system":"bad"}"#).is_err());
    let state: CharacterDemo = serde_json::from_value(json!({"visits":3,"turns":[]})).unwrap();
    assert_eq!(state.visits, 3);
    assert!(state.room_changes.is_empty());
}
