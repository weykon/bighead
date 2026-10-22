use super::*;
use crate::{SystemDefinition, character_demo::CharacterDemo};

#[test]
fn character_prompt_routes_detailed_knowledge_to_the_on_demand_guide() {
    assert!(WORLD_PROMPT.contains("guide_scene_world"));
    assert!(WORLD_PROMPT.contains("world_projection"));
    assert!(
        WORLD_PROMPT.len() < 1_500,
        "always-on prompt grew too large"
    );
    for manual_detail in ["SceneRenderable:", "WorkAppearance {", "args.dt"] {
        assert!(
            !WORLD_PROMPT.contains(manual_detail),
            "manual detail leaked into always-on prompt: {manual_detail}"
        );
    }
}

fn install(rt: &mut AgentRuntime, source: &str) {
    let def = SystemDefinition {
        id: "motion".into(),
        description: String::new(),
        system_prompt: "".into(),
        kind: SystemKind::RhaiScript {
            source: source.into(),
        },
        tool_names: vec![],
        max_iterations: 1,
        is_tool: false,
    };
    if rt.system("motion").is_some() {
        rt.system_registry.replace(def).unwrap();
    } else {
        rt.register_system(def).unwrap();
    }
}
fn advance(rt: &mut AgentRuntime, state: &mut SceneLoop) {
    step(
        &mut rt.world,
        &mut rt.component_registry,
        &mut rt.system_registry,
        state,
        0.05,
    );
}

#[test]
fn continuous_scripts_hot_update_and_stop_without_resetting_other_entities() {
    let mut rt = AgentRuntime::new_for_test();
    let a = rt
        .world
        .spawn((SceneTransform::default(), SceneRenderable::default()))
        .id();
    let b = rt
        .world
        .spawn((SceneTransform::default(), SceneRenderable::default()))
        .id();
    let scheduler = rt
        .world
        .spawn(SceneSystem {
            system_id: "motion".into(),
            ..Default::default()
        })
        .id();
    install(
        &mut rt,
        "for row in query(\"SceneTransform\") { let t=row.SceneTransform; set_component(row._entity_id,\"SceneTransform\",#{x:t.x+args.dt}); }",
    );
    let mut state = SceneLoop::default();
    advance(&mut rt, &mut state);
    assert_eq!(rt.world.get::<SceneSystem>(scheduler).unwrap().error, "");
    assert!((rt.world.get::<SceneTransform>(a).unwrap().x - 0.05).abs() < 1e-8);
    install(
        &mut rt,
        &format!(
            "for row in query(\"SceneTransform\") {{ let t=row.SceneTransform; if row._entity_id=={} {{ set_component(row._entity_id,\"SceneTransform\",#{{y:sin(args.elapsed)}}); }} else {{set_component(row._entity_id,\"SceneTransform\",#{{x:t.x+args.dt}});}} }}",
            a.index().index()
        ),
    );
    advance(&mut rt, &mut state);
    assert_eq!(rt.world.get::<SceneSystem>(scheduler).unwrap().error, "");
    assert!((rt.world.get::<SceneTransform>(a).unwrap().x - 0.05).abs() < 1e-8);
    assert!(rt.world.get::<SceneTransform>(a).unwrap().y > 0.0);
    assert!((rt.world.get::<SceneTransform>(b).unwrap().x - 0.1).abs() < 1e-8);
    rt.world.get_mut::<SceneSystem>(scheduler).unwrap().enabled = false;
    advance(&mut rt, &mut state);
    assert_eq!(rt.world.get::<SceneSystem>(scheduler).unwrap().runs, 2);
}

#[test]
fn runaway_script_is_disabled_and_invalid_visuals_are_reported() {
    let mut rt = AgentRuntime::new_for_test();
    install(&mut rt, "loop { let x=1+1; }");
    let scheduler = rt
        .world
        .spawn(SceneSystem {
            system_id: "motion".into(),
            ..Default::default()
        })
        .id();
    advance(&mut rt, &mut SceneLoop::default());
    let s = rt.world.get::<SceneSystem>(scheduler).unwrap();
    assert!(!s.enabled);
    assert!(!s.error.is_empty());
    rt.world.spawn((
        SceneTransform {
            x: f64::NAN,
            ..Default::default()
        },
        SceneRenderable::default(),
    ));
    let snapshot = projection(&mut rt.world);
    assert_eq!(snapshot["objects"].as_array().unwrap().len(), 0);
    assert_eq!(snapshot["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn migration_and_restore_preserve_deletions_transforms_and_schedules() {
    let mut rt = AgentRuntime::new_for_test();
    rt.world.spawn(CharacterDemo::default());
    rt.ensure_programmable_scene();
    let lamp = rt
        .world
        .query::<(Entity, &SceneName)>()
        .iter(&rt.world)
        .find(|(_, n)| n.id == "lamp")
        .unwrap()
        .0;
    rt.world.entity_mut(lamp).remove::<SceneRenderable>();
    rt.world.get_mut::<SceneTransform>(lamp).unwrap().x = 3.0;
    rt.ensure_programmable_scene();
    assert!(rt.world.get::<SceneRenderable>(lamp).is_none());
    install(
        &mut rt,
        "for row in query(\"SceneTransform\") {set_component(row._entity_id,\"SceneTransform\",#{y:args.elapsed});}",
    );
    rt.world.spawn(SceneSystem {
        system_id: "motion".into(),
        ..Default::default()
    });
    let dir = std::env::temp_dir().join(format!("scene-world-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("checkpoint.json");
    rt.checkpoint(&path).unwrap();
    let mut restored = AgentRuntime::new_for_test();
    restored.restore(&path).unwrap();
    restored.ensure_programmable_scene();
    let lamp = restored
        .world
        .query::<(Entity, &SceneName)>()
        .iter(&restored.world)
        .find(|(_, n)| n.id == "lamp")
        .unwrap()
        .0;
    assert!(restored.world.get::<SceneRenderable>(lamp).is_none());
    assert_eq!(restored.world.get::<SceneTransform>(lamp).unwrap().x, 3.0);
    advance(&mut restored, &mut SceneLoop::default());
    assert_eq!(restored.world.get::<SceneTransform>(lamp).unwrap().y, 0.05);
    std::fs::remove_dir_all(dir).unwrap();
}
