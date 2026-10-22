use crate::{journal::*, persistence::*};
use bevy_ecs::{component::Component, world::World};
use big_head_ecs::{ComponentActionRegistry, SystemRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;

#[test]
fn small_update_writes_only_changed_entity_and_ignores_uncommitted_tail() {
    let (mut world, registry, systems) = setup();
    let dir = dir();
    let path = dir.join("checkpoint.json");
    let entity = world.spawn(Note { value: 0 }).id();
    for value in 0..2000 {
        world.spawn(Note { value });
    }
    let mut journal = DeliveryJournal::default();
    journal
        .save(&path, &world, &registry, &systems, None)
        .unwrap();
    let base = fs::read(&path).unwrap();
    world.get_mut::<Note>(entity).unwrap().value = 99;
    journal
        .save(&path, &world, &registry, &systems, None)
        .unwrap();
    let log = path.with_extension("journal");
    let frame_path = fs::read_dir(&log)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let bytes = fs::read(&frame_path).unwrap();
    let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let delta: serde_json::Value =
        serde_json::from_str(frame["payload"].as_str().unwrap()).unwrap();
    assert_eq!(delta["upserts"].as_array().unwrap().len(), 1);
    assert!(bytes.len() * 20 < base.len());
    assert_eq!(fs::read(&path).unwrap(), base);
    let generation = delta["generation"].as_str().unwrap();
    fs::write(
        log.join(format!("{generation}-00000000000000000002.json")),
        "incomplete",
    )
    .unwrap();
    let (mut restored, mut registry, mut systems) = setup();
    restore(&path, &mut restored, &mut registry, &mut systems).unwrap();
    assert_eq!(restored.query::<&Note>().iter(&restored).count(), 2001);
    fs::write(log.join(format!("{generation}.head")), "broken").unwrap();
    assert!(restore(&path, &mut restored, &mut registry, &mut systems).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn journal_compacts_at_frame_limit_and_continues() {
    let (mut world, registry, systems) = setup();
    let dir = dir();
    let path = dir.join("checkpoint.json");
    let entity = world.spawn(Note { value: 0 }).id();
    let mut journal = DeliveryJournal::default();
    journal
        .save(&path, &world, &registry, &systems, None)
        .unwrap();
    let base = fs::read(&path).unwrap();
    for value in 1..=130 {
        world.get_mut::<Note>(entity).unwrap().value = value;
        journal
            .save(&path, &world, &registry, &systems, None)
            .unwrap();
    }
    assert_ne!(fs::read(&path).unwrap(), base);
    let (mut restored, mut registry, mut systems) = setup();
    restore(&path, &mut restored, &mut registry, &mut systems).unwrap();
    assert_eq!(
        restored.query::<&Note>().single(&restored).unwrap().value,
        130
    );
    fs::remove_dir_all(dir).unwrap();
}

#[derive(Component, Serialize, Deserialize)]
struct Note {
    value: u64,
}
fn setup() -> (World, ComponentActionRegistry, SystemRegistry) {
    let mut registry = ComponentActionRegistry::new();
    registry.register_rust_component::<Note>("Note");
    (World::new(), registry, SystemRegistry::new())
}
fn dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "journal-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

#[test]
fn delivery_deltas_restore_mutations_deletions_and_metadata_without_rewriting_base() {
    let (mut world, registry, systems) = setup();
    let dir = dir();
    let path = dir.join("checkpoint.json");
    let a = world.spawn(Note { value: 1 }).id();
    let b = world.spawn(Note { value: 2 }).id();
    let mut journal = DeliveryJournal::default();
    journal
        .save(&path, &world, &registry, &systems, Some(json!({"lease":1})))
        .unwrap();
    let baseline = fs::read(&path).unwrap();
    world.get_mut::<Note>(a).unwrap().value = 3;
    world.despawn(b);
    journal
        .save(&path, &world, &registry, &systems, Some(json!({"lease":2})))
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), baseline);
    world.spawn(Note { value: 4 });
    journal
        .save(&path, &world, &registry, &systems, Some(json!({"lease":3})))
        .unwrap();
    let files: Vec<_> = fs::read_dir(path.with_extension("journal"))
        .unwrap()
        .collect();
    assert_eq!(files.len(), 3);
    journal
        .save(&path, &world, &registry, &systems, Some(json!({"lease":3})))
        .unwrap();
    assert_eq!(
        fs::read_dir(path.with_extension("journal"))
            .unwrap()
            .count(),
        3
    );
    fs::write(path.with_extension("journal").join(".tmp-partial"), "{").unwrap();
    let (mut restored, mut reg, mut sys) = setup();
    let metadata = restore_with_metadata(&path, &mut restored, &mut reg, &mut sys).unwrap();
    assert_eq!(metadata, Some(json!({"lease":3})));
    let mut values: Vec<_> = restored
        .query::<&Note>()
        .iter(&restored)
        .map(|n| n.value)
        .collect();
    values.sort();
    assert_eq!(values, vec![3, 4]);
    // A full save starts a new generation; obsolete frames cannot resurrect old data.
    checkpoint_with_metadata(&path, &world, &registry, &systems, Some(json!({"lease":4}))).unwrap();
    let (mut restored, mut reg, mut sys) = setup();
    assert_eq!(
        restore_with_metadata(&path, &mut restored, &mut reg, &mut sys).unwrap(),
        Some(json!({"lease":4}))
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn corrupt_or_missing_committed_frame_fails_before_world_mutation() {
    let (mut world, registry, systems) = setup();
    let dir = dir();
    let path = dir.join("checkpoint.json");
    let a = world.spawn(Note { value: 1 }).id();
    let mut journal = DeliveryJournal::default();
    for n in 0..3 {
        world.get_mut::<Note>(a).unwrap().value = n;
        journal
            .save(&path, &world, &registry, &systems, None)
            .unwrap();
    }
    let mut files: Vec<_> = fs::read_dir(path.with_extension("journal"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let original = fs::read(&files[0]).unwrap();
    fs::write(&files[0], "{}").unwrap();
    let (mut restored, mut reg, mut sys) = setup();
    let before = restored.entities().len();
    assert!(restore(&path, &mut restored, &mut reg, &mut sys).is_err());
    assert_eq!(restored.entities().len(), before);
    fs::write(&files[0], original).unwrap();
    fs::remove_file(files.last().unwrap()).unwrap();
    assert!(
        restore(&path, &mut restored, &mut reg, &mut sys)
            .unwrap_err()
            .contains("sequence")
    );
    assert_eq!(restored.entities().len(), before);
    fs::remove_dir_all(dir).unwrap();
}
