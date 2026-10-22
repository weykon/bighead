use super::*;
use big_head_ai::ToolDefinition;

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition{name:"inspect_work".into(),description:"Read the work district, shared book facts, stable IDs, evidence snapshots, layout revisions, connections and interaction triggers. Evidence is a recorded source, not fresh test execution.".into(),input_schema:json!({"type":"object","properties":{}})},
        ToolDefinition{name:"record_work".into(),description:"Create or replace one work fact. expected_revision=0 creates; updates require the current revision. Layout is independent. Completed requires a completion-supporting evidence ID from inspect_work; citing evidence does not automatically prove a new claim. Reuse IDs, never report visual changes as completed work.".into(),input_schema:json!({"type":"object","properties":{
            "id":{"type":"string"},"expected_revision":{"type":"integer"},"title":{"type":"string"},"summary":{"type":"string"},"status":{"enum":["idea","in_progress","blocked","completed","archived"]},"evidence_ids":{"type":"array","items":{"type":"string"}}
        },"required":["id","expected_revision","title","summary","status","evidence_ids"],"additionalProperties":false})},
        ToolDefinition{name:"arrange_work".into(),description:"Atomically validate then apply a partial building layout and upsert directed connections. Does not change work facts. Each placement needs its current appearance revision; minimum building separation 1.5 units. Omitted buildings/connections stay unchanged. Same directed connection updates its label; remove:true deletes it.".into(),input_schema:json!({"type":"object","properties":{
            "placements":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"expected_revision":{"type":"integer"},"x":{"type":"number"},"z":{"type":"number"},"height":{"type":"number","minimum":0.5,"maximum":2.4},"style":{"enum":["archive","workshop","lab","tower","garden"]},"color":{"type":"string"}},"required":["id","expected_revision","x","z","height","style","color"],"additionalProperties":false}},
            "connections":{"type":"array","items":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"},"label":{"type":"string"},"remove":{"type":"boolean"}},"required":["from","to","label"],"additionalProperties":false}}
        },"additionalProperties":false})},
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordRequest {
    id: String,
    expected_revision: u64,
    title: String,
    summary: String,
    status: String,
    evidence_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    id: String,
    expected_revision: u64,
    x: f64,
    z: f64,
    height: f64,
    style: String,
    color: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    from: String,
    to: String,
    label: String,
    #[serde(default)]
    remove: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arrangement {
    #[serde(default)]
    placements: Vec<Placement>,
    #[serde(default)]
    connections: Vec<Connection>,
}

pub(crate) fn dispatch(world: &mut World, name: &str, input: &Value) -> Result<Value, String> {
    let catalog = world
        .query::<&WorkDistrict>()
        .iter(world)
        .next()
        .cloned()
        .ok_or("Initialize the district from the page first")?;
    match name {
        "inspect_work" => {
            let mut value = projection(world);
            value["evidence_catalog"] = json!(catalog.evidence);
            Ok(value)
        }
        "record_work" => {
            let p: RecordRequest =
                serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
            if p.id.is_empty()
                || p.id.len() > 64
                || !p
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err("Work id must be 1..64 ASCII letters/digits/-/_".into());
            }
            if p.title.trim().is_empty()
                || p.title.chars().count() > 60
                || p.summary.chars().count() > 2000
            {
                return Err("Title 1..60 characters; summary at most 2000".into());
            }
            if !["idea", "in_progress", "blocked", "completed", "archived"]
                .contains(&p.status.as_str())
            {
                return Err("Invalid status".into());
            }
            if p.evidence_ids.len() > 8
                || p.evidence_ids
                    .iter()
                    .any(|id| !catalog.evidence.iter().any(|e| e.id == *id))
            {
                return Err("Unknown evidence id or more than 8 sources".into());
            }
            if p.status == "completed"
                && !catalog
                    .evidence
                    .iter()
                    .any(|e| supports_work(e, &p.id) && p.evidence_ids.contains(&e.id))
            {
                return Err("Completed work requires recorded completion evidence; an idea or building is not proof".into());
            }
            let entity = record_entity(world, &p.id);
            let revision = entity
                .and_then(|e| world.get::<WorkRecord>(e))
                .map_or(0, |r| r.revision);
            if revision != p.expected_revision {
                return Err(format!("Stale work revision: expected {revision}"));
            }
            if entity.is_none() && world.query::<&WorkRecord>().iter(world).count() >= 64 {
                return Err("District capacity: 64 work records".into());
            }
            let record = WorkRecord {
                id: p.id,
                title: p.title,
                summary: p.summary,
                status: p.status,
                evidence_ids: p.evidence_ids,
                revision: revision + 1,
            };
            let e = entity.unwrap_or_else(|| world.spawn_empty().id());
            world.entity_mut(e).insert(record);
            Ok(
                json!({"entity_id":e.index().index(),"revision":revision+1,"fact_updated":true,"layout_changed":false}),
            )
        }
        "arrange_work" => {
            let p: Arrangement =
                serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
            if p.placements.len() > 64 || p.connections.len() > 128 {
                return Err("Layout batch exceeds capacity".into());
            }
            let mut changes = vec![];
            let mut ids = std::collections::HashSet::new();
            for v in p.placements {
                if !ids.insert(v.id.clone()) {
                    return Err("Duplicate placement id".into());
                }
                let e = record_entity(world, &v.id).ok_or("Unknown work id")?;
                let old = world.get::<WorkAppearance>(e);
                if old.map_or(0, |a| a.revision) != v.expected_revision {
                    return Err(format!("Stale appearance revision for {}", v.id));
                }
                let appearance = WorkAppearance {
                    x: v.x,
                    z: v.z,
                    height: v.height,
                    color: v.color,
                    style: v.style,
                    glow: old.map_or(0.0, |a| a.glow),
                    revision: v.expected_revision + 1,
                };
                if !valid_appearance(&appearance) {
                    return Err("Unsupported appearance: x +/-5, z +/-4, height .5..2.4, hex color, known style".into());
                }
                changes.push((e, appearance));
            }
            let mut positions: Vec<_> = world
                .query::<(Entity, &WorkRecord, &WorkAppearance)>()
                .iter(world)
                .filter(|(e, _, _)| !changes.iter().any(|(c, _)| c == e))
                .map(|(e, _, a)| (e, a.x, a.z))
                .collect();
            positions.extend(changes.iter().map(|(e, a)| (*e, a.x, a.z)));
            for (i, (_, x, z)) in positions.iter().enumerate() {
                for (_, bx, bz) in positions.iter().skip(i + 1) {
                    if (x - bx).hypot(z - bz) < 1.5 {
                        return Err(
                            "Buildings overlap: keep centers at least 1.5 units apart".into()
                        );
                    }
                }
            }
            let mut links = vec![];
            let mut pairs = std::collections::HashSet::new();
            for c in p.connections {
                if c.from == c.to
                    || record_entity(world, &c.from).is_none()
                    || record_entity(world, &c.to).is_none()
                    || c.label.chars().count() > 100
                {
                    return Err("Connections require two existing distinct work IDs and a label <=100 characters".into());
                }
                if !pairs.insert((c.from.clone(), c.to.clone())) {
                    return Err("Duplicate directed connection".into());
                }
                let existing = world
                    .query::<(Entity, &WorkConnection)>()
                    .iter(world)
                    .find(|(_, old)| old.from == c.from && old.to == c.to)
                    .map(|(e, _)| e);
                links.push((existing, c));
            }
            let current = world.query::<&WorkConnection>().iter(world).count();
            let adding = links
                .iter()
                .filter(|(e, c)| e.is_none() && !c.remove)
                .count();
            let removing = links
                .iter()
                .filter(|(e, c)| e.is_some() && c.remove)
                .count();
            if current + adding - removing > 128 {
                return Err("Connection capacity: 128".into());
            }
            for (e, a) in changes {
                world.entity_mut(e).insert(a);
            }
            for (old, c) in links {
                if c.remove {
                    if let Some(e) = old {
                        world.entity_mut(e).remove::<WorkConnection>();
                        if world.entity(e).archetype().components().is_empty() {
                            world.despawn(e);
                        }
                    }
                } else {
                    let e = old.unwrap_or_else(|| world.spawn_empty().id());
                    world.entity_mut(e).insert(WorkConnection {
                        from: c.from,
                        to: c.to,
                        label: c.label,
                    });
                }
            }
            Ok(json!({"layout_updated":true,"facts_changed":false,"district":projection(world)}))
        }
        _ => Err("Unknown work tool".into()),
    }
}
