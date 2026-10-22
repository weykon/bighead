//! Work facts and their spatial projection share stable IDs, not copied task state.
use crate::AgentRuntime;
use bevy_ecs::{component::Component, entity::Entity, world::World};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod tools;
pub(crate) use tools::{definitions, dispatch};
#[cfg(test)]
mod tests;

#[derive(Component, Clone, Default, Serialize, Deserialize, Reflect)]
pub struct WorkRecord {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub evidence_ids: Vec<String>,
    pub revision: u64,
}

#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct WorkAppearance {
    pub x: f64,
    pub z: f64,
    pub height: f64,
    pub color: String,
    pub style: String,
    pub glow: f64,
    pub revision: u64,
}
impl Default for WorkAppearance {
    fn default() -> Self {
        Self {
            x: 0.0,
            z: 0.0,
            height: 1.0,
            color: "#65b9ab".into(),
            style: "workshop".into(),
            glow: 0.0,
            revision: 1,
        }
    }
}

#[derive(Component, Clone, Default, Serialize, Deserialize, Reflect)]
pub struct WorkConnection {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Clone, Default, Serialize, Deserialize, Reflect)]
pub struct WorkEvidence {
    #[serde(default)]
    pub work_id: String,
    pub id: String,
    pub source: String,
    pub quote: String,
    pub digest: String,
    pub supports_completion: bool,
}

fn seed_subject(id: &str) -> &str {
    match id {
        "scene-acceptance" => "scene-runtime",
        "checkpoint-journal" => "checkpoint",
        "dream-practice" => "dream",
        "serial-limit" => "concurrent-simulation",
        "work-request" => "living-work",
        _ => "",
    }
}

pub(crate) fn supports_work(e: &WorkEvidence, id: &str) -> bool {
    e.supports_completion
        && (if e.work_id.is_empty() {
            seed_subject(&e.id)
        } else {
            &e.work_id
        }) == id
}

#[derive(Clone, Default, Serialize, Deserialize, Reflect)]
pub struct WorkEvent {
    #[serde(default)]
    pub status: String,
    pub id: String,
    pub target_id: String,
    pub kind: String,
    pub sequence: u64,
}

#[derive(Component, Clone, Default, Serialize, Deserialize, Reflect)]
pub struct WorkDistrict {
    pub evidence: Vec<WorkEvidence>,
    pub events: Vec<WorkEvent>,
    pub sequence: u64,
}

#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct WorkTrigger {
    pub target_id: String,
    pub event: String,
    pub system_id: String,
    pub enabled: bool,
    pub runs: u64,
    pub error: String,
}
impl Default for WorkTrigger {
    fn default() -> Self {
        Self {
            target_id: String::new(),
            event: "enter".into(),
            system_id: String::new(),
            enabled: true,
            runs: 0,
            error: String::new(),
        }
    }
}

pub(crate) fn valid_appearance(a: &WorkAppearance) -> bool {
    a.x.is_finite()
        && a.z.is_finite()
        && a.x.abs() <= 5.0
        && a.z.abs() <= 4.0
        && (0.5..=2.4).contains(&a.height)
        && (0.0..=2.0).contains(&a.glow)
        && a.color.len() == 7
        && a.color.starts_with('#')
        && a.color[1..].bytes().all(|c| c.is_ascii_hexdigit())
        && ["archive", "workshop", "lab", "tower", "garden"].contains(&a.style.as_str())
}

pub(crate) fn record_entity(world: &mut World, id: &str) -> Option<Entity> {
    world
        .query::<(Entity, &WorkRecord)>()
        .iter(world)
        .find(|(_, r)| r.id == id)
        .map(|(e, _)| e)
}

pub(crate) fn projection(world: &mut World) -> Value {
    let district = world.query::<&WorkDistrict>().iter(world).next().cloned();
    let evidence = district
        .as_ref()
        .map(|d| d.evidence.clone())
        .unwrap_or_default();
    let mut warnings = vec![];
    let mut records = vec![];
    for (e, r, a) in world
        .query::<(Entity, &WorkRecord, Option<&WorkAppearance>)>()
        .iter(world)
    {
        if records.len() >= 64 {
            warnings.push("街区最多展示 64 项工作".to_string());
            break;
        }
        let sources: Vec<_> = r
            .evidence_ids
            .iter()
            .filter_map(|id| evidence.iter().find(|s| s.id == *id))
            .collect();
        let status = if r.status == "completed" && !sources.iter().any(|s| supports_work(s, &r.id))
        {
            "unverified"
        } else {
            r.status.as_str()
        };
        let appearance = a.filter(|a| valid_appearance(a));
        if appearance.is_none() {
            warnings.push(format!("{} 尚无有效的建筑布局", r.title));
        }
        records.push(json!({"entity_id":e.index().index(),"record":r,"status":status,"appearance":appearance,"evidence":sources}));
    }
    records.sort_by_key(|r| r["record"]["id"].as_str().unwrap_or_default().to_string());
    let connections: Vec<_> = world
        .query::<&WorkConnection>()
        .iter(world)
        .filter(|c| {
            records.iter().any(|r| r["record"]["id"] == c.from)
                && records.iter().any(|r| r["record"]["id"] == c.to)
        })
        .take(128)
        .cloned()
        .collect();
    let triggers: Vec<_> = world
        .query::<(Entity, &WorkTrigger)>()
        .iter(world)
        .take(32)
        .map(|(e, t)| json!({"entity_id":e.index().index(),"trigger":t}))
        .collect();
    json!({"initialized":district.is_some(),"records":records,"connections":connections,"events":district.map(|d|d.events).unwrap_or_default(),"triggers":triggers,"warnings":warnings})
}

fn evidence(id: &str, source: &str, text: &str, quote: &str, completed: bool) -> WorkEvidence {
    assert!(
        text.contains(quote),
        "seed evidence must be present in the source"
    );
    WorkEvidence {
        work_id: seed_subject(id).into(),
        id: id.into(),
        source: source.into(),
        quote: quote.into(),
        digest: big_head_save::persistence::digest(text.as_bytes()),
        supports_completion: completed,
    }
}

impl AgentRuntime {
    pub(crate) fn initialize_work_district(&mut self) {
        if self
            .world
            .query::<&WorkDistrict>()
            .iter(&self.world)
            .next()
            .is_some()
        {
            return;
        }
        let scene = include_str!("../../../docs/character-demo.md");
        let checkpoint = include_str!("../../../docs/checkpoint-lifecycle.md");
        let dream = include_str!("../../../docs/dream-rsi.md");
        let evidence = vec![
            evidence(
                "scene-acceptance",
                "docs/character-demo.md",
                scene,
                "The completed run passed both MiniMax requests and a graceful-save/restart check.",
                true,
            ),
            evidence(
                "checkpoint-journal",
                "docs/checkpoint-lifecycle.md",
                checkpoint,
                "Unchanged saves write nothing.",
                true,
            ),
            evidence(
                "dream-practice",
                "docs/dream-rsi.md",
                dream,
                "This first implementation connects a fixed task, real program execution, recorded\nexploration trees, offline policy comparison, and the next online round.",
                true,
            ),
            evidence(
                "serial-limit",
                "docs/character-demo.md",
                scene,
                "while a model request or other main-loop work is in flight, scene scripts pause.",
                false,
            ),
            evidence(
                "work-request",
                "conversation:2026-09-22",
                "一个工作可以化为游戏中的一本书或者一座小城市，里面每一个道路或者建筑都是一个结果或者成果，我们的agent可以自行安排",
                "一个工作可以化为游戏中的一本书或者一座小城市，里面每一个道路或者建筑都是一个结果或者成果，我们的agent可以自行安排",
                false,
            ),
        ];
        self.world.spawn(WorkDistrict {
            evidence,
            ..Default::default()
        });
        for (id, title, summary, status, source, x, z, style, color) in [
            (
                "scene-runtime",
                "场景工坊",
                "已跑通三方块运动、单个行为热更新和重启恢复。",
                "completed",
                "scene-acceptance",
                -2.8,
                -1.6,
                "workshop",
                "#e89578",
            ),
            (
                "checkpoint",
                "分片档案馆",
                "持久化使用增量日志，未变化的保存不再重复写入；引用检查仍有边界。",
                "completed",
                "checkpoint-journal",
                0.0,
                -1.6,
                "archive",
                "#69b3aa",
            ),
            (
                "dream",
                "梦境实验室",
                "Rhai 练习已连接在线探索、离线回放和下一轮验证；不代表通用能力提升。",
                "completed",
                "dream-practice",
                2.8,
                -1.6,
                "lab",
                "#8c9fd1",
            ),
            (
                "living-work",
                "工作街区",
                "正在把真实工作、证据与建筑、书页、交互连接起来。",
                "in_progress",
                "work-request",
                -1.5,
                1.5,
                "tower",
                "#d9b953",
            ),
            (
                "concurrent-simulation",
                "不停摆的世界",
                "模型请求占用主循环时，场景脚本暂停；并发调度仍待实现。",
                "blocked",
                "serial-limit",
                1.5,
                1.5,
                "workshop",
                "#d78c9e",
            ),
        ] {
            self.world.spawn((
                WorkRecord {
                    id: id.into(),
                    title: title.into(),
                    summary: summary.into(),
                    status: status.into(),
                    evidence_ids: vec![source.into()],
                    revision: 1,
                },
                WorkAppearance {
                    x,
                    z,
                    style: style.into(),
                    color: color.into(),
                    ..Default::default()
                },
            ));
        }
        for (from, to, label) in [
            ("checkpoint", "scene-runtime", "保存与恢复"),
            ("scene-runtime", "living-work", "场景投影"),
            ("dream", "living-work", "探索方法"),
            ("living-work", "concurrent-simulation", "持续运行的缺口"),
        ] {
            self.world.spawn(WorkConnection {
                from: from.into(),
                to: to.into(),
                label: label.into(),
            });
        }
        self.mark_scene_dirty();
    }

    pub(crate) fn work_event(&mut self, input: &Value) -> Result<(), String> {
        if input["action"] == "work_initialize" {
            self.initialize_work_district();
            return self.save_delivery_state();
        }
        let target = input["object_id"].as_str().ok_or("Missing work id")?;
        let id = input["interaction_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 80)
            .ok_or("Invalid event id")?;
        if record_entity(&mut self.world, target).is_none() {
            return Err("Work no longer exists".into());
        }
        let (entity, mut district) = self
            .world
            .query::<(Entity, &WorkDistrict)>()
            .iter(&self.world)
            .next()
            .map(|(e, d)| (e, d.clone()))
            .ok_or("Initialize the district first")?;
        if let Some(old) = district.events.iter().find(|e| e.id == id) {
            return if old.target_id == target {
                self.save_delivery_state()
            } else {
                Err("Event ID reused for another target".into())
            };
        }
        district.sequence += 1;
        let event = WorkEvent {
            status: "accepted".into(),
            id: id.into(),
            target_id: target.into(),
            kind: "enter".into(),
            sequence: district.sequence,
        };
        district.events.push(event.clone());
        if district.events.len() > 32 {
            district.events.remove(0);
        }
        self.world.entity_mut(entity).insert(district);
        // Commit the receipt before running scripts. Interrupted events are not replayed automatically.
        self.mark_scene_dirty();
        self.save_delivery_state()?;
        let triggers: Vec<_> = self
            .world
            .query::<(Entity, &WorkTrigger)>()
            .iter(&self.world)
            .filter(|(_, t)| {
                t.enabled && t.event == "enter" && (t.target_id == target || t.target_id == "*")
            })
            .map(|(e, t)| (e, t.clone()))
            .collect();
        let engine = crate::scene_world::bounded_engine();
        let mut event_status = if triggers.is_empty() {
            "no_handler"
        } else {
            "executed"
        };
        for (index, (e, mut trigger)) in triggers.into_iter().enumerate() {
            let result = (|| -> Result<(), String> {
                if index >= 16 {
                    return Err("At most 16 triggers per event".into());
                }
                let source = match self
                    .system_registry
                    .get(&trigger.system_id)
                    .map(|s| &s.kind)
                {
                    Some(crate::SystemKind::RhaiScript { source }) => source.clone(),
                    _ => return Err("Trigger requires an existing Rhai system".into()),
                };
                let ast = engine.compile(&source).map_err(|e| e.to_string())?;
                let _ = crate::scripting::eval_ast_with_world_args(
                    &engine,
                    &ast,
                    &mut self.world,
                    &mut self.component_registry,
                    &mut self.system_registry,
                    &json!({"event":event}),
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })();
            if result.is_err() {
                event_status = "failed";
            }
            if let Some(current) = self.world.get::<WorkTrigger>(e) {
                if current.system_id != trigger.system_id {
                    continue;
                }
                trigger.enabled = current.enabled;
                match result {
                    Ok(()) => {
                        trigger.runs += 1;
                        trigger.error.clear();
                    }
                    Err(error) => {
                        trigger.enabled = false;
                        trigger.error = error.chars().take(500).collect();
                    }
                }
                self.world.entity_mut(e).insert(trigger);
            }
        }
        if let Some(mut district) = self.world.get_mut::<WorkDistrict>(entity) {
            if let Some(receipt) = district.events.iter_mut().find(|e| e.id == id) {
                receipt.status = event_status.into();
            }
        }
        self.mark_scene_dirty();
        self.save_delivery_state()
    }
}
