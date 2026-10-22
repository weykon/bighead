//! MVP 文件持久化 —— checkpoint/restore。
//!
//! 保存和恢复三层状态：
//! 1. 动态组件 schema
//! 2. Rhai 脚本系统定义
//! 3. 实体/组件数据（世界快照）
//!
//! 存储格式：JSON 主文件、内容寻址 blobs 和增量 journal。

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use bevy_ecs::world::World;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Serialize};

use big_head_ecs::dynamic_component;
use big_head_ecs::registry::ComponentActionRegistry;
use big_head_ecs::schema::DynamicSchema;
use big_head_ecs::{SystemDefinition, SystemKind, SystemRegistry};

/// 顶层持久化状态。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedState {
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub journal_generation: String,
    pub schemas: BTreeMap<String, DynamicSchema>,
    pub systems: Vec<SystemDefinition>,
    pub world: PersistenceSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<serde_json::Value>,
}

/// 所有实体及其组件值的快照。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistenceSnapshot {
    pub entities: Vec<PersistenceEntity>,
}

/// 单个实体的组件集合。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceEntity {
    pub index: u64,
    pub components: Vec<PersistenceComponent>,
}

/// 单个组件的序列化值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceComponent {
    pub name: String,
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_blob: Option<ContentBlob>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlob {
    pub sha256: String,
    pub bytes: u64,
}

fn blob_directory(path: &Path) -> std::path::PathBuf {
    path.with_extension("blobs")
}

pub fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn store_blob(directory: &Path, content: &str) -> Result<ContentBlob, String> {
    let blob = ContentBlob {
        sha256: digest(content.as_bytes()),
        bytes: content.len() as u64,
    };
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let target = directory.join(&blob.sha256);
    if let Ok(metadata) = fs::metadata(&target) {
        if metadata.len() != blob.bytes {
            return Err(format!(
                "existing blob has wrong size: {}",
                target.display()
            ));
        }
        let mut file = fs::File::open(&target).map_err(|e| e.to_string())?;
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = [0u8; 65536];
        loop {
            let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let actual: String = hash
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != blob.sha256 {
            return Err(format!(
                "existing blob integrity mismatch: {}",
                target.display()
            ));
        }
        return Ok(blob);
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".tmp-{}-{nonce}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &target)?;
        fs::File::open(directory)?.sync_all()
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(temporary);
        return Err(format!("write shard blob: {e}"));
    }
    Ok(blob)
}

fn read_blob(checkpoint: &Path, blob: &ContentBlob) -> Result<String, String> {
    if blob.sha256.len() != 64
        || !blob
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid shard blob hash".into());
    }
    let path = blob_directory(checkpoint).join(&blob.sha256);
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("read shard blob {}: {e}", path.display()))?;
    if content.len() as u64 != blob.bytes || digest(content.as_bytes()) != blob.sha256 {
        return Err(format!("shard blob integrity mismatch: {}", path.display()));
    }
    Ok(content)
}

pub(crate) struct StreamingWorld<'a> {
    pub world: &'a World,
    pub registry: &'a ComponentActionRegistry,
    pub blob_dir: &'a Path,
}

impl Serialize for StreamingWorld<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Entities<'a>(&'a StreamingWorld<'a>);
        impl Serialize for Entities<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let source = self.0;
                let mut sequence = serializer.serialize_seq(None)?;
                for archetype in source.world.archetypes().iter() {
                    for entry in archetype.entities() {
                        let entity = entry.id();
                        let mut components = Vec::new();
                        for &cid in archetype.components() {
                            let Some(info) = source.world.components().get_info(cid) else {
                                continue;
                            };
                            let full = info.name().to_string();
                            let name = full.rsplit("::").next().unwrap_or(&full).to_string();
                            let value = if source.registry.dynamic_store().is_dynamic(cid) {
                                dynamic_component::read(source.world, entity, cid)
                            } else {
                                source.registry.serialize_rust_component(
                                    source.world,
                                    entity,
                                    &name,
                                )
                            };
                            if let Some(mut value) = value {
                                let content_blob = if name == "ShardData" {
                                    if let Some(content) = value
                                        .get("content")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| s.len() > 4096)
                                    {
                                        let blob = store_blob(source.blob_dir, content)
                                            .map_err(serde::ser::Error::custom)?;
                                        value.as_object_mut().unwrap().remove("content");
                                        Some(blob)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };
                                components.push(PersistenceComponent {
                                    name,
                                    value,
                                    content_blob,
                                });
                            }
                        }
                        if !components.is_empty() {
                            sequence.serialize_element(&PersistenceEntity {
                                index: entity.index().index() as u64,
                                components,
                            })?;
                        }
                    }
                }
                sequence.end()
            }
        }
        let mut state = serializer.serialize_struct("PersistenceSnapshot", 1)?;
        state.serialize_field("entities", &Entities(self))?;
        state.end()
    }
}

/// 构建当前世界状态的快照，用于持久化。
pub fn build_snapshot(world: &World, registry: &ComponentActionRegistry) -> PersistenceSnapshot {
    let mut entities = Vec::new();

    for archetype in world.archetypes().iter() {
        for arch_entity in archetype.entities() {
            let entity = arch_entity.id();
            let idx = entity.index().index() as u64;
            let mut components = Vec::new();

            for &cid in archetype.components().iter() {
                let Some(info) = world.components().get_info(cid) else {
                    continue;
                };
                let full_name = info.name().to_string();
                let short_name = full_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&full_name)
                    .to_string();

                if registry.dynamic_store().is_dynamic(cid) {
                    if let Some(val) = dynamic_component::read(world, entity, cid) {
                        components.push(PersistenceComponent {
                            name: short_name,
                            value: val,
                            content_blob: None,
                        });
                        continue;
                    }
                }

                if let Some(val) = registry.serialize_rust_component(world, entity, &short_name) {
                    components.push(PersistenceComponent {
                        name: short_name,
                        value: val,
                        content_blob: None,
                    });
                }
            }

            if !components.is_empty() {
                entities.push(PersistenceEntity {
                    index: idx,
                    components,
                });
            }
        }
    }

    PersistenceSnapshot { entities }
}

/// 保存 checkpoint 到 JSON 文件。
pub fn checkpoint(
    path: &Path,
    world: &World,
    registry: &ComponentActionRegistry,
    system_registry: &SystemRegistry,
) -> Result<(), String> {
    checkpoint_with_metadata(path, world, registry, system_registry, None)
}

pub fn checkpoint_with_metadata(
    path: &Path,
    world: &World,
    registry: &ComponentActionRegistry,
    system_registry: &SystemRegistry,
    runtime: Option<serde_json::Value>,
) -> Result<(), String> {
    let mut schemas = BTreeMap::new();
    for (name, schema) in registry.dynamic_store().iter() {
        schemas.insert(name.to_string(), schema.clone());
    }

    let systems: Vec<SystemDefinition> = system_registry
        .iter()
        .filter(|sys| matches!(sys.kind, SystemKind::RhaiScript { .. }))
        .cloned()
        .collect();

    #[derive(Serialize)]
    struct StreamingState<'a> {
        format_version: u32,
        journal_generation: String,
        schemas: BTreeMap<String, DynamicSchema>,
        systems: Vec<SystemDefinition>,
        world: StreamingWorld<'a>,
        runtime: Option<serde_json::Value>,
    }
    let blob_dir = blob_directory(path);
    let state = StreamingState {
        format_version: 3,
        journal_generation: digest(
            format!("{}:{:?}", std::process::id(), std::time::SystemTime::now()).as_bytes(),
        ),
        schemas,
        systems,
        world: StreamingWorld {
            world,
            registry,
            blob_dir: &blob_dir,
        },
        runtime,
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &state)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("write {}: {error}", path.display()));
    }

    Ok(())
}

/// 从 JSON 文件恢复状态。
pub fn restore(
    path: &Path,
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    system_registry: &mut SystemRegistry,
) -> Result<(), String> {
    restore_with_metadata(path, world, registry, system_registry).map(|_| ())
}

pub fn restore_with_metadata(
    path: &Path,
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    system_registry: &mut SystemRegistry,
) -> Result<Option<serde_json::Value>, String> {
    let file = fs::File::open(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    let mut state: PersistedState =
        serde_json::from_reader(BufReader::new(file)).map_err(|e| format!("parse state: {}", e))?;
    if state.format_version > 3 {
        return Err(format!(
            "unsupported checkpoint version: {}",
            state.format_version
        ));
    }
    crate::journal::replay(path, &mut state)?;
    // Validate every external reference before inserting anything into the world.
    for entity in &state.world.entities {
        for component in &entity.components {
            if let Some(blob) = &component.content_blob {
                if component.name != "ShardData" || component.value.get("content").is_some() {
                    return Err("invalid external shard representation".into());
                }
                read_blob(path, blob)?;
            }
        }
    }

    for (name, schema) in &state.schemas {
        if registry.dynamic_store().resolve_name(name).is_some() {
            continue;
        }
        registry.define_dynamic_component(world, name, schema.clone())?;
    }

    for system in &state.systems {
        // Builtin systems (e.g. guide_*) are already registered by
        // AgentRuntime::new(). Skip duplicates instead of failing the
        // whole restore — the in-memory definition from new() is the
        // canonical source.
        if system_registry.get(&system.id).is_some() {
            continue;
        }
        system_registry.register(system.clone())?;
    }

    for entity_data in state.world.entities {
        let entity = world.spawn_empty().id();

        for mut comp in entity_data.components {
            if let Some(blob) = &comp.content_blob {
                let content = read_blob(path, blob)?;
                comp.value
                    .as_object_mut()
                    .ok_or("invalid shard metadata")?
                    .insert("content".into(), serde_json::Value::String(content));
            }
            if registry.dynamic_store().resolve_name(&comp.name).is_some() {
                registry.insert_dynamic(world, entity, &comp.name, comp.value)?;
                continue;
            }

            let json_str = serde_json::to_string(&comp.value)
                .map_err(|e| format!("serialize component '{}': {}", comp.name, e))?;
            if let Err(e) = registry.insert_component(world, entity, &comp.name, &json_str) {
                if comp.content_blob.is_some() {
                    return Err(format!("restore external component '{}': {e}", comp.name));
                }
                log::warn!("restore: skip component '{}': {}", comp.name, e);
            }
        }
    }

    Ok(state.runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use big_head_ecs::schema::FieldType;
    use std::collections::BTreeMap;

    #[derive(Component, serde::Serialize, serde::Deserialize)]
    struct TestTag {
        label: String,
    }

    fn test_world_with_registry() -> (World, ComponentActionRegistry, SystemRegistry) {
        let mut world = World::new();
        let mut registry = ComponentActionRegistry::new();
        registry.register_rust_component::<TestTag>("TestTag");
        (world, registry, SystemRegistry::new())
    }

    #[test]
    fn external_shards_roundtrip_reuse_and_integrity_failure_preserves_world() {
        let (mut world, mut registry, systems) = test_world_with_registry();
        registry
            .define_dynamic_component(
                &mut world,
                "ShardData",
                DynamicSchema {
                    fields: [
                        ("content".into(), FieldType::String),
                        ("paradigm".into(), FieldType::String),
                    ]
                    .into(),
                },
            )
            .unwrap();
        let content = "\u{4e2d}\u{1f30d}".repeat(2000);
        for _ in 0..2 {
            let e = world.spawn_empty().id();
            registry
                .insert_dynamic(
                    &mut world,
                    e,
                    "ShardData",
                    serde_json::json!({"content":content,"paradigm":"test"}),
                )
                .unwrap();
        }
        let dir = std::env::temp_dir().join(format!(
            "blob-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("checkpoint.json");
        checkpoint(&path, &world, &registry, &systems).unwrap();
        assert!(fs::metadata(&path).unwrap().len() < 4096);
        assert_eq!(fs::read_dir(blob_directory(&path)).unwrap().count(), 1);
        let blob_path = blob_directory(&path).join(digest(content.as_bytes()));
        let modified = fs::metadata(&blob_path).unwrap().modified().unwrap();
        checkpoint(&path, &world, &registry, &systems).unwrap();
        assert_eq!(
            fs::metadata(&blob_path).unwrap().modified().unwrap(),
            modified
        );
        let (mut restored, mut restored_registry, mut restored_systems) =
            test_world_with_registry();
        restore(
            &path,
            &mut restored,
            &mut restored_registry,
            &mut restored_systems,
        )
        .unwrap();
        let snap = build_snapshot(&restored, &restored_registry);
        assert_eq!(snap.entities.len(), 2);
        assert!(
            snap.entities
                .iter()
                .all(|e| e.components[0].value["content"] == content)
        );
        let old = fs::read(&path).unwrap();
        fs::write(&blob_path, "x".repeat(content.len())).unwrap();
        assert!(checkpoint(&path, &world, &registry, &systems).is_err());
        assert_eq!(fs::read(&path).unwrap(), old);
        let before = restored.entities().len();
        assert!(
            restore(
                &path,
                &mut restored,
                &mut restored_registry,
                &mut restored_systems
            )
            .unwrap_err()
            .contains("integrity")
        );
        assert_eq!(restored.entities().len(), before);
        fs::remove_file(&blob_path).unwrap();
        assert!(
            restore(
                &path,
                &mut restored,
                &mut restored_registry,
                &mut restored_systems
            )
            .unwrap_err()
            .contains("read shard blob")
        );
        assert_eq!(restored.entities().len(), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "manual save measurement; not a timing assertion"]
    fn measure_large_shard_checkpoint() {
        let (mut world, mut registry, systems) = test_world_with_registry();
        registry
            .define_dynamic_component(
                &mut world,
                "ShardData",
                DynamicSchema {
                    fields: [("content".into(), FieldType::String)].into(),
                },
            )
            .unwrap();
        let e = world.spawn_empty().id();
        registry
            .insert_dynamic(
                &mut world,
                e,
                "ShardData",
                serde_json::json!({"content":"a".repeat(20 * 1024 * 1024)}),
            )
            .unwrap();
        let dir = std::env::temp_dir().join(format!("checkpoint-measure-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("checkpoint.json");
        let start = std::time::Instant::now();
        let old = serde_json::to_vec_pretty(&build_snapshot(&world, &registry)).unwrap();
        let old_ms = start.elapsed().as_millis();
        let old_bytes = old.len();
        drop(old);
        let start = std::time::Instant::now();
        checkpoint(&path, &world, &registry, &systems).unwrap();
        let first_ms = start.elapsed().as_millis();
        let start = std::time::Instant::now();
        checkpoint(&path, &world, &registry, &systems).unwrap();
        eprintln!(
            "20 MiB shard: old snapshot+JSON CPU={old_ms}ms bytes={old_bytes}; first external save={first_ms}ms; repeated external save={}ms manifest_bytes={}",
            start.elapsed().as_millis(),
            fs::metadata(&path).unwrap().len()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_inline_checkpoint_still_restores() {
        let (world, registry, systems) = test_world_with_registry();
        let path = std::env::temp_dir().join(format!("legacy-inline-{}.json", std::process::id()));
        let state = serde_json::json!({"schemas":{},"systems":[],"world":{"entities":[{"index":99,"components":[{"name":"TestTag","value":{"label":"legacy"}}]}]}});
        fs::write(&path, state.to_string()).unwrap();
        let (mut target, mut reg, mut sys) = test_world_with_registry();
        restore(&path, &mut target, &mut reg, &mut sys).unwrap();
        assert_eq!(
            target.query::<&TestTag>().single(&target).unwrap().label,
            "legacy"
        );
        checkpoint(&path, &world, &registry, &systems).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_checkpoint_restore_roundtrip() {
        let (mut world, mut registry, mut sys_reg) = test_world_with_registry();

        // 定义动态组件
        let mut fields = BTreeMap::new();
        fields.insert("label".to_string(), FieldType::String);
        registry
            .define_dynamic_component(&mut world, "Tag", DynamicSchema { fields })
            .unwrap();

        // 创建带动态组件的实体
        let entity = world.spawn_empty().id();
        registry
            .insert_dynamic(
                &mut world,
                entity,
                "Tag",
                serde_json::json!({"label": "test"}),
            )
            .unwrap();

        // 注册 Rhai 系统
        sys_reg
            .register(SystemDefinition {
                id: "my_script".into(),
                description: "test".into(),
                system_prompt: std::borrow::Cow::Borrowed(""),
                kind: SystemKind::RhaiScript {
                    source: "log(\"hello\")".into(),
                },
                tool_names: vec![],
                max_iterations: 1,
                is_tool: false,
            })
            .unwrap();

        // 保存
        let path = std::env::temp_dir().join("big_head_test_checkpoint.json");
        checkpoint(&path, &world, &registry, &sys_reg).unwrap();

        // 恢复到全新状态
        let (mut world2, mut registry2, mut sys_reg2) = test_world_with_registry();
        restore(&path, &mut world2, &mut registry2, &mut sys_reg2).unwrap();

        // 验证：动态组件 schema 已恢复
        assert!(registry2.dynamic_store().get_id("Tag").is_some());

        // 验证：Rhai 系统已恢复
        assert!(sys_reg2.get("my_script").is_some());
        match &sys_reg2.get("my_script").unwrap().kind {
            SystemKind::RhaiScript { source } => assert_eq!(source, "log(\"hello\")"),
            _ => panic!("expected RhaiScript"),
        }

        let _ = fs::remove_file(&path);
    }
}
