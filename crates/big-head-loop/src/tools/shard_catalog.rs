//! Bounded discovery and explicit source links for agent-authored artifacts.
use bevy_ecs::{component::Component, world::World};
use big_head_shard::ShardData;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Serialize, Deserialize, bevy_reflect::Reflect)]
pub struct ShardSources {
    pub paradigms: Vec<String>,
}

pub fn definitions() -> Vec<big_head_ai::ToolDefinition> {
    vec![
    big_head_ai::ToolDefinition{name:"inspect_shards".into(),description:"Exact full-scope shard statistics plus optional paged metadata. Default view=summary reports all matching groups by family, total content bytes and largest groups, without reading text. view=groups sorts complete group sizes descending; view=shards lists entities. Page coverage is not full inspection. Zero explicit ShardSources links does not mean zero references or prior usage. Split strategy is unknown unless recorded; chunk sizes do not prove overlap. Dates are UTC. Use expected_snapshot to detect metadata changes between pages.".into(),input_schema:json!({"type":"object","properties":{"view":{"type":"string","enum":["summary","groups","shards"]},"paradigm_contains":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":50},"expected_snapshot":{"type":"string"}},"additionalProperties":false})},
    big_head_ai::ToolDefinition{name:"link_shard_sources".into(),description:"Attach source paradigms to an existing structured artifact entity after reading and creating it. Merges with existing links, never removes links. Links protect sources from retention GC and are queryable as ShardSources by Rhai/Lens. Records provenance, not proof of factual correctness. At most 16 source paradigms per artifact.".into(),input_schema:json!({"type":"object","properties":{"entity_id":{"type":"integer","minimum":0},"paradigms":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string"}}},"required":["entity_id","paradigms"],"additionalProperties":false})}
]
}

#[derive(Default)]
struct GroupStats {
    bytes: u64,
    shards: usize,
    indices: BTreeSet<u32>,
    totals: BTreeSet<u32>,
    earliest: u64,
    latest: u64,
}
impl GroupStats {
    fn complete(&self) -> bool {
        self.totals.len() == 1
            && self.totals.contains(&(self.shards as u32))
            && self.indices.len() == self.shards
            && self.indices.iter().copied().eq(0..self.shards as u32)
    }
}

fn utc(seconds: u64) -> Option<String> {
    // The existing formatter's offset-zero path handles UTC. Bound invalid epochs.
    (seconds <= 253402300799).then(|| crate::time::format_local_timestamp(seconds, 0))
}

fn family(paradigm: &str) -> String {
    let mut parts = paradigm.split(':');
    let first = parts.next().unwrap_or("");
    if first == "context" {
        format!("context:{}", parts.next().unwrap_or(""))
    } else {
        first.into()
    }
}

pub fn inspect(world: &mut World, input: &Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        view: Option<String>,
        expected_snapshot: Option<String>,
        #[serde(default)]
        paradigm_contains: String,
        #[serde(default)]
        offset: usize,
        limit: Option<usize>,
    }
    let r: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    let limit = r.limit.unwrap_or(20);
    let view = r.view.as_deref().unwrap_or("summary");
    if !["summary", "groups", "shards"].contains(&view) {
        return Err("invalid view".into());
    }
    if view == "summary" && r.offset != 0 {
        return Err("summary has no item pages; use view=groups or shards".into());
    }
    if !(1..=50).contains(&limit) {
        return Err("limit must be 1..50".into());
    }
    let policy_rows: Vec<_> = world
        .query::<&crate::shard_retention::ShardRetention>()
        .iter(world)
        .map(|p| {
            (
                p.paradigm.clone(),
                (p.pinned, p.retention_secs, p.last_accessed_at),
            )
        })
        .collect();
    let policy_count = policy_rows.len();
    let policies: BTreeMap<_, _> = policy_rows.into_iter().collect();
    if policies.len() != policy_count {
        return Err("duplicate retention policies; statistics blocked".into());
    }
    let mut derived = BTreeMap::<String, Vec<u64>>::new();
    for (e, s) in world
        .query::<(bevy_ecs::entity::Entity, &ShardSources)>()
        .iter(world)
    {
        for p in &s.paradigms {
            derived
                .entry(p.clone())
                .or_default()
                .push(e.index().index() as u64);
        }
    }
    let mut rows: Vec<_> = world
        .query::<(bevy_ecs::entity::Entity, &ShardData)>()
        .iter(world)
        .filter(|(_, s)| s.paradigm.contains(&r.paradigm_contains))
        .map(|(e, s)| (e.index().index() as u64, s))
        .collect();
    rows.sort_by(|a, b| {
        (&a.1.paradigm, a.1.shard_index, a.0).cmp(&(&b.1.paradigm, b.1.shard_index, b.0))
    });
    let mut indices = BTreeMap::<(&str, u32), usize>::new();
    for (_, s) in &rows {
        *indices.entry((&s.paradigm, s.shard_index)).or_default() += 1;
    }
    let mut groups = BTreeMap::<&str, GroupStats>::new();
    for (_, s) in &rows {
        let g = groups.entry(&s.paradigm).or_insert_with(|| GroupStats {
            earliest: s.created_at,
            ..Default::default()
        });
        g.bytes += s.content.len() as u64;
        g.shards += 1;
        g.indices.insert(s.shard_index);
        g.totals.insert(s.total_shards);
        g.earliest = g.earliest.min(s.created_at);
        g.latest = g.latest.max(s.created_at);
    }
    let metadata: Vec<_> = rows
        .iter()
        .map(|(id, s)| {
            (
                id,
                &s.paradigm,
                s.shard_index,
                s.total_shards,
                s.content.len(),
                &s.source_label,
                s.created_at,
            )
        })
        .collect();
    let snapshot = big_head_save::persistence::digest(
        &serde_json::to_vec(&(&metadata, &policies, &derived)).map_err(|e| e.to_string())?,
    );
    if r.expected_snapshot.as_ref().is_some_and(|s| s != &snapshot) {
        return Err(
            "catalog metadata changed; restart at offset=0 without expected_snapshot".into(),
        );
    }
    let mut ranked: Vec<_> = groups.iter().collect();
    ranked.sort_by(|(an, a), (bn, b)| b.bytes.cmp(&a.bytes).then(an.cmp(bn)));
    let group_value = |name: &&str, g: &&GroupStats| json!({"paradigm":name,"group_content_bytes":g.bytes,"shards":g.shards,"complete":g.complete(),"managed":policies.contains_key(*name),"pinned":policies.get(*name).is_some_and(|p|p.0),"earliest_created_at_utc":utc(g.earliest),"latest_created_at_utc":utc(g.latest),"split_strategy":"unknown_not_recorded","explicit_link_entity_count":derived.get(*name).map_or(0,Vec::len)});
    let mut families = BTreeMap::<String, (usize, usize, u64, usize, usize)>::new();
    for (name, g) in &groups {
        let f = families.entry(family(name)).or_default();
        f.0 += 1;
        f.1 += g.shards;
        f.2 += g.bytes;
        f.3 += usize::from(policies.contains_key(*name));
        f.4 += usize::from(policies.get(*name).is_some_and(|p| p.0));
    }
    let mut family_rows: Vec<_> = families.iter().collect();
    family_rows.sort_by(|(an, a), (bn, b)| b.2.cmp(&a.2).then(an.cmp(bn)));
    let family_items:Vec<_>=family_rows.iter().take(32).map(|(name,f)|json!({"family":name,"groups":f.0,"shards":f.1,"content_bytes":f.2,"managed_groups":f.3,"unmanaged_groups":f.0-f.3,"pinned_groups":f.4})).collect();
    let managed = groups.keys().filter(|p| policies.contains_key(**p)).count();
    let summary = json!({"scope":if r.paradigm_contains.is_empty(){"entire_world"}else{"all_matching_groups"},"paradigm_contains":r.paradigm_contains,"computed_over_all_matches":true,
        "groups":groups.len(),"shards":rows.len(),"logical_content_bytes":groups.values().map(|g|g.bytes).sum::<u64>(),
        "managed_groups":managed,"unmanaged_groups":groups.len()-managed,"pinned_groups":groups.keys().filter(|p|policies.get(**p).is_some_and(|v|v.0)).count(),
        "incomplete_groups":groups.values().filter(|g|!g.complete()).count(),"explicitly_linked_groups":groups.keys().filter(|p|derived.get(**p).is_some_and(|v|!v.is_empty())).count(),
        "families":family_items,"family_count":families.len(),"family_table_complete":families.len()<=32,"omitted_family_groups":family_rows.iter().skip(32).map(|(_,f)|f.0).sum::<usize>(),
        "largest_groups":ranked.iter().take(5).map(|(n,g)|group_value(n,g)).collect::<Vec<_>>()});
    let items: Vec<_> = if view == "summary" {
        vec![]
    } else if view == "groups" {
        ranked
            .iter()
            .skip(r.offset)
            .take(limit)
            .map(|(n, g)| group_value(n, g))
            .collect()
    } else {
        rows.iter().skip(r.offset).take(limit).map(|(id,s)|{
        let targets=derived.get(&s.paradigm).cloned().unwrap_or_default();
        json!({"entity_id":id,"paradigm":s.paradigm,"shard_index":s.shard_index,"total_shards":s.total_shards,"bytes":s.content.len(),"bytes_scope":"one_shard","source_label":s.source_label,"created_at":s.created_at,"created_at_utc":utc(s.created_at),"split_strategy":"unknown_not_recorded",
        "ambiguous_index":indices[&(s.paradigm.as_str(),s.shard_index)]>1,
        "retention":policies.get(&s.paradigm).map(|(p,t,a)|json!({"pinned":p,"retention_secs":t,"last_accessed_at":a,"last_accessed_at_utc":utc(*a),"eligible_after_utc":utc(a.saturating_add(*t)),"deletion_guaranteed":false})),
        "derived_entity_ids":targets.iter().take(8).collect::<Vec<_>>(),"derived_entity_count":targets.len()})
    }).collect()
    };
    let total_items = match view {
        "groups" => groups.len(),
        "shards" => rows.len(),
        _ => 0,
    };
    let next = r.offset.saturating_add(items.len()).min(total_items);
    let mut response = json!({"summary":summary,"snapshot":snapshot,"measured_at_utc":utc(big_head_ecs::execution::now_secs()),"view":view,"items":items,"matching_shards":rows.len(),"matching_groups":groups.len(),"next_offset":next,"has_more":next<total_items,
        "coverage":{"aggregate_complete":true,"returned_items":items.len(),"matching_items":total_items,"offset":r.offset,"all_items_returned":view!="summary" && r.offset==0 && items.len()==total_items,"body_content_read":false,"page_limited_by_bytes":false},
        "evidence_scope":{"references":"explicit_ShardSources_only","other_component_references_checked":false,"historical_usage_checked":false,"orphan_status":"unknown","split_strategy":"unknown_not_recorded","family_semantics":"name_prefix_only","retention":"New context artifacts of any context subtype may be managed. Unmanaged legacy groups remain retained. TTL is eligibility, not guaranteed deletion; access, pins, references and completeness matter."},
        "reference_scope":"Only explicit ShardSources links; absent links do not prove no references. Entity IDs are valid only in the current runtime."});
    while response.to_string().len() > 14000 {
        let items = response["items"].as_array_mut().unwrap();
        if items.pop().is_none() {
            return Err(
                "aggregate metadata exceeds response budget; narrow paradigm_contains".into(),
            );
        }
        let count = items.len();
        if count == 0 && r.offset < total_items {
            return Err(
                "item metadata exceeds page budget; use view=summary and a narrower filter".into(),
            );
        }
        let next = r.offset.saturating_add(count).min(total_items);
        response["next_offset"] = json!(next);
        response["has_more"] = json!(next < total_items);
        response["coverage"]["returned_items"] = json!(count);
        response["coverage"]["all_items_returned"] = json!(false);
        response["coverage"]["page_limited_by_bytes"] = json!(true);
    }
    Ok(response)
}

/// Give the final narrator bounded raw aggregates, not just the operator's claims.
pub fn reply_evidence(log: &[(String, String, String, bool)]) -> Vec<Value> {
    let mut evidence = vec![];
    let mut seen = BTreeSet::new();
    for (name, _, output, error) in log.iter().rev() {
        if name != "inspect_shards" || *error {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(output) else {
            continue;
        };
        let Some(summary) = value.get("summary") else {
            continue;
        };
        let key = summary["paradigm_contains"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if !seen.insert(key) {
            continue;
        }
        let item = json!({"summary":summary,"coverage":value["coverage"],"snapshot":value["snapshot"],"measured_at_utc":value["measured_at_utc"],"evidence_scope":value["evidence_scope"]});
        if item.to_string().len() <= 10000 {
            evidence.push(item);
        }
    }
    evidence.sort_by_key(|v| v["summary"]["scope"] != "entire_world");
    evidence.truncate(3);
    while serde_json::to_vec(&evidence).map_or(usize::MAX, |v| v.len()) > 10000 {
        evidence.pop();
    }
    evidence
}

pub fn link(world: &mut World, input: &Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        entity_id: u64,
        paradigms: Vec<String>,
    }
    let r: Request = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    let entity = big_head_ecs::resolve_entity(world, r.entity_id)?;
    if r.paradigms.is_empty() || r.paradigms.len() > 16 {
        return Err("provide 1..16 source paradigms".into());
    }
    let available: BTreeSet<_> = world
        .query::<&ShardData>()
        .iter(world)
        .map(|s| s.paradigm.clone())
        .collect();
    if r.paradigms.iter().any(|p| !available.contains(p)) {
        return Err("source paradigm missing; links unchanged".into());
    }
    let mut sources: BTreeSet<_> = world
        .get::<ShardSources>(entity)
        .map(|s| s.paradigms.iter().cloned().collect())
        .unwrap_or_default();
    sources.extend(r.paradigms);
    if sources.len() > 16 {
        return Err("merged source count exceeds 16; links unchanged".into());
    }
    let sources = ShardSources {
        paradigms: sources.into_iter().collect(),
    };
    let response = serde_json::to_value(&sources).map_err(|e| e.to_string())?;
    world.entity_mut(entity).insert(sources);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shard(rt: &mut crate::AgentRuntime, name: &str, index: u32, total: u32, bytes: usize) {
        rt.world.spawn(ShardData {
            paradigm: name.into(),
            shard_index: index,
            total_shards: total,
            content: "x".repeat(bytes),
            source_label: "test".into(),
            created_at: 1789554134,
        });
    }
    #[test]
    fn full_aggregates_do_not_depend_on_first_page_or_retention_subset() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for i in 0..60 {
            shard(&mut rt, &format!("context:message_text:{i:03}"), 0, 1, 10);
        }
        shard(&mut rt, "context:tool_result:large", 0, 2, 9000);
        shard(&mut rt, "context:tool_result:large", 1, 2, 8000);
        shard(&mut rt, "z:single", 0, 1, 10000);
        crate::shard_retention::manage_new(&mut rt.world, "context:message_text:000");
        let first = inspect(&mut rt.world, &json!({"view":"shards","limit":1})).unwrap();
        assert_eq!(first["coverage"]["returned_items"], 1);
        assert_eq!(first["coverage"]["all_items_returned"], false);
        assert_eq!(first["summary"]["groups"], 62);
        assert_eq!(first["summary"]["shards"], 63);
        assert_eq!(first["summary"]["managed_groups"], 1);
        assert_eq!(first["summary"]["unmanaged_groups"], 61);
        assert_eq!(first["summary"]["logical_content_bytes"], 27600);
        assert_eq!(
            first["summary"]["largest_groups"][0]["paradigm"],
            "context:tool_result:large"
        );
        assert_eq!(
            first["summary"]["largest_groups"][0]["group_content_bytes"],
            17000
        );
        let families = first["summary"]["families"].as_array().unwrap();
        let tools = families
            .iter()
            .find(|f| f["family"] == "context:tool_result")
            .unwrap();
        assert_eq!(tools["groups"], 1);
        assert_eq!(tools["unmanaged_groups"], 1);
        let summary = inspect(&mut rt.world, &json!({})).unwrap();
        assert_eq!(summary["summary"], first["summary"]);
        assert_eq!(summary["items"], json!([]));
        assert_eq!(summary["coverage"]["body_content_read"], false);
        let sorted = inspect(&mut rt.world, &json!({"view":"groups","limit":1})).unwrap();
        assert_eq!(sorted["items"][0]["group_content_bytes"], 17000);
        let filtered = inspect(&mut rt.world, &json!({"paradigm_contains":"z:"})).unwrap();
        assert_eq!(filtered["summary"]["scope"], "all_matching_groups");
        assert_eq!(filtered["summary"]["groups"], 1);
    }

    #[test]
    fn timestamps_unknowns_and_metadata_snapshot_are_explicit() {
        let mut rt = crate::AgentRuntime::new_for_test();
        shard(&mut rt, "legacy", 0, 1, 16384);
        let first = inspect(&mut rt.world, &json!({"view":"shards"})).unwrap();
        assert_eq!(
            first["items"][0]["created_at_utc"],
            "2026-09-16T10:22:14+00:00"
        );
        assert_eq!(first["items"][0]["split_strategy"], "unknown_not_recorded");
        assert_eq!(first["evidence_scope"]["orphan_status"], "unknown");
        assert_eq!(first["evidence_scope"]["historical_usage_checked"], false);
        let input = json!({"view":"shards","expected_snapshot":first["snapshot"],"offset":1});
        assert!(inspect(&mut rt.world, &input).is_ok());
        shard(&mut rt, "new", 0, 1, 1);
        assert!(
            inspect(&mut rt.world, &input)
                .unwrap_err()
                .contains("changed")
        );
        assert_eq!(utc(0).unwrap(), "1970-01-01T00:00:00+00:00");
        assert!(utc(u64::MAX).is_none());
    }

    #[test]
    fn family_and_item_limits_never_claim_full_display() {
        let mut rt = crate::AgentRuntime::new_for_test();
        for i in 0..40 {
            shard(&mut rt, &format!("family{i}:group"), 0, 1, i + 1);
        }
        let summary = inspect(&mut rt.world, &json!({})).unwrap();
        assert_eq!(summary["summary"]["family_count"], 40);
        assert_eq!(summary["summary"]["family_table_complete"], false);
        assert_eq!(summary["summary"]["omitted_family_groups"], 8);
        let page = inspect(&mut rt.world, &json!({"view":"shards","limit":50})).unwrap();
        assert!(page.to_string().len() <= 14000);
        assert_eq!(page["coverage"]["page_limited_by_bytes"], true);
        assert_eq!(page["has_more"], true);
        let next=inspect(&mut rt.world,&json!({"view":"shards","offset":page["next_offset"],"expected_snapshot":page["snapshot"]})).unwrap();
        assert_ne!(page["items"][0]["entity_id"], next["items"][0]["entity_id"]);
    }

    #[test]
    fn narrator_receives_raw_global_evidence_even_after_filtered_queries() {
        let mut rt = crate::AgentRuntime::new_for_test();
        shard(&mut rt, "a:group", 0, 1, 100);
        let global = inspect(&mut rt.world, &json!({})).unwrap();
        let mut logs = vec![(
            "inspect_shards".into(),
            "{}".into(),
            global.to_string(),
            false,
        )];
        for filter in ["a", "group", "a:", "missing"] {
            let filtered = inspect(&mut rt.world, &json!({"paradigm_contains":filter})).unwrap();
            logs.push((
                "inspect_shards".into(),
                "{}".into(),
                filtered.to_string(),
                false,
            ));
        }
        logs.push((
            "inspect_shards".into(),
            "{}".into(),
            "invalid/failed tool result".into(),
            true,
        ));
        let evidence = reply_evidence(&logs);
        assert_eq!(evidence.len(), 3);
        assert_eq!(evidence[0]["summary"], global["summary"]);
        assert_eq!(evidence[0]["evidence_scope"]["orphan_status"], "unknown");
    }
    #[test]
    fn source_links_are_discoverable_and_protect_retention() {
        let mut rt = crate::AgentRuntime::new_for_test();
        let (paradigm, _) = crate::context_artifact::store(
            &mut rt.world,
            &rt.component_registry,
            "context:test",
            "test",
            "payload",
        )
        .unwrap();
        let artifact = rt.world.spawn_empty().id();
        let input = json!({"entity_id":artifact.index().index(),"paradigms":[paradigm]});
        link(&mut rt.world, &input).unwrap();
        link(&mut rt.world, &input).unwrap();
        assert_eq!(
            rt.world
                .get::<ShardSources>(artifact)
                .unwrap()
                .paradigms
                .len(),
            1
        );
        let catalog = inspect(&mut rt.world, &json!({"view":"shards","limit":1})).unwrap();
        assert_eq!(catalog["items"][0]["derived_entity_count"], 1);
        assert!(catalog["items"][0].get("content").is_none());
        let (value, error) = big_head_save::read::execute_get_component(
            &rt.world,
            rt.component_registry.dynamic_store(),
            artifact.index().index() as u64,
            "ShardSources",
        );
        assert!(!error);
        assert!(value.contains(&paradigm), "{value}");
        let path = std::env::temp_dir().join(format!("shard-links-{}.json", uuid::Uuid::new_v4()));
        big_head_save::persistence::checkpoint(
            &path,
            &rt.world,
            &rt.component_registry,
            &rt.system_registry,
        )
        .unwrap();
        let mut restored = crate::AgentRuntime::new_for_test();
        restored.restore(&path).unwrap();
        assert!(
            restored
                .world
                .query::<&ShardSources>()
                .iter(&restored.world)
                .any(|s| s.paradigms.contains(&paradigm))
        );
        std::fs::remove_file(path).unwrap();
        let future = big_head_ecs::execution::now_secs() + 8 * 86400;
        let report = crate::shard_retention::collect(
            &mut rt.world,
            &rt.component_registry,
            &rt.system_registry,
            future,
        )
        .unwrap();
        assert_eq!(report["reclaimed_shards"], 0);
        assert!(
            link(
                &mut rt.world,
                &json!({"entity_id":artifact.index().index(),"paradigms":["missing"]})
            )
            .is_err()
        );
        assert_eq!(
            rt.world
                .get::<ShardSources>(artifact)
                .unwrap()
                .paradigms
                .len(),
            1
        );
    }
}
