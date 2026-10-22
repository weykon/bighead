use super::*;
use crate::character_demo::CharacterTurn;

fn setup() -> (AgentRuntime, Entity) {
    let mut rt = AgentRuntime::new_for_test();
    rt.ensure_room_scene();
    let character = rt
        .world
        .spawn(CharacterDemo {
            turns: vec![CharacterTurn {
                id: "test-1".into(),
                text: "把台灯放到地上".into(),
                reply: String::new(),
                status: "thinking".into(),
            }],
            ..Default::default()
        })
        .id();
    (rt, character)
}
fn movement(target: &str, revision: u64) -> RoomMove {
    RoomMove {
        object_id: "lamp".into(),
        target: target.into(),
        expected_revision: revision,
        position: None,
    }
}
fn lamp(rt: &mut AgentRuntime) -> RoomPlacement {
    rt.world
        .query::<(&RoomSceneItem, &RoomPlacement)>()
        .iter(&rt.world)
        .find(|(i, _)| i.id == "lamp")
        .unwrap()
        .1
        .clone()
}

#[test]
fn scene_initialization_is_idempotent_and_never_resets_saved_placement() {
    let (mut rt, character) = setup();
    assert!(
        rt.apply_room_move(character, &movement("floor", 1))
            .unwrap()
            .contains("地上")
    );
    rt.ensure_room_scene();
    assert_eq!(rt.scene_inventory().as_array().unwrap().len(), 4);
    assert_eq!(lamp(&mut rt).surface, "floor");
    assert_eq!(lamp(&mut rt).position[1], 0.1);
    assert!(
        rt.apply_room_move(character, &movement("desk", 2))
            .unwrap()
            .contains("桌上")
    );
    assert_eq!(lamp(&mut rt).position, [1.55, 1.1, 0.18]);
    let receipt = rt
        .world
        .get::<CharacterDemo>(character)
        .unwrap()
        .room_changes
        .last()
        .unwrap();
    let evidence: Value = serde_json::from_str(&receipt.input_json).unwrap();
    assert_eq!(evidence["from"]["surface"], "floor");
    assert_eq!(evidence["verified"], true);
}

#[test]
fn invalid_stale_and_unsupported_moves_leave_state_unchanged() {
    let (mut rt, c) = setup();
    let before = lamp(&mut rt);
    for action in [
        movement("floor", 0),
        movement("shelf", 1),
        RoomMove {
            object_id: "desk".into(),
            ..movement("floor", 1)
        },
        RoomMove {
            object_id: "missing".into(),
            ..movement("floor", 1)
        },
    ] {
        assert!(rt.apply_room_move(c, &action).is_err());
        assert_eq!(lamp(&mut rt), before);
    }
    for position in [
        [100.0, 0.1, 0.0],
        [2.25, 0.0, 0.55],
        [0.45, 0.1, 0.45],
        [-1.8, 0.1, 0.5],
        [-0.8, 0.1, 1.45],
        [f32::NAN, 0.1, 0.0],
    ] {
        assert!(
            rt.apply_room_move(
                c,
                &RoomMove {
                    position: Some(position),
                    ..movement("floor", 1)
                }
            )
            .is_err(),
            "{position:?}"
        );
        assert_eq!(lamp(&mut rt), before);
    }
    assert!(
        rt.world
            .get::<CharacterDemo>(c)
            .unwrap()
            .room_changes
            .is_empty()
    );
}

#[test]
fn same_placement_does_not_increment_revision_and_stale_retries_cannot_move_again() {
    let (mut rt, c) = setup();
    assert!(
        rt.apply_room_move(c, &movement("desk", 1))
            .unwrap()
            .contains("没有改变")
    );
    assert_eq!(lamp(&mut rt).revision, 1);
    rt.apply_room_move(c, &movement("floor", 1)).unwrap();
    assert!(rt.apply_room_move(c, &movement("desk", 1)).is_err());
    assert_eq!(lamp(&mut rt).surface, "floor");
}

#[test]
fn placement_survives_delivery_journal_restore_and_snapshot_rollback() {
    let (mut rt, c) = setup();
    let dir = std::env::temp_dir().join(format!("room-scene-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("checkpoint.json");
    rt.set_checkpoint_path(&path);
    rt.save_delivery_state().unwrap();
    let before = rt.scene_placements();
    rt.apply_room_move(c, &movement("floor", 1)).unwrap();
    rt.save_delivery_state().unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    restored.ensure_room_scene();
    assert_eq!(lamp(&mut restored), lamp(&mut rt));
    assert_eq!(restored.scene_inventory().as_array().unwrap().len(), 4);
    rt.restore_scene_placements(before);
    assert_eq!(lamp(&mut rt).surface, "desk");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn confirmed_note_can_move_without_rewriting_its_content() {
    let (mut rt, c) = setup();
    let entity = rt
        .world
        .spawn(crate::character_room::RoomObject {
            id: "note".into(),
            title: "保留".into(),
            body: "原文".into(),
            kind: "note".into(),
            source_turn_id: "source".into(),
            source_quote: "原文".into(),
            reason: "test".into(),
            location: "desk".into(),
            revision: 1,
            user_confirmed: true,
        })
        .id();
    rt.apply_room_move(
        c,
        &RoomMove {
            object_id: "note".into(),
            ..movement("floor", 1)
        },
    )
    .unwrap();
    let note = rt
        .world
        .get::<crate::character_room::RoomObject>(entity)
        .unwrap();
    assert!(note.user_confirmed);
    assert_eq!(note.body, "原文");
    assert_eq!(note.location, "floor");
}
