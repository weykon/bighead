//! Atomic, checksummed delivery deltas. A full checkpoint starts a new generation.
use crate::persistence::{self, PersistedState, StreamingWorld};
use bevy_ecs::world::World;
use big_head_ecs::{ComponentActionRegistry, SystemKind, SystemRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct DeliveryJournal {
    path: PathBuf,
    generation: String,
    sequence: u64,
    previous: String,
    baseline: Option<Capture>,
    manifest_stamp: Option<(u64, std::time::SystemTime)>,
    bytes: u64,
}

struct Capture {
    entities: BTreeMap<u64, Value>,
    schemas: Value,
    systems: Value,
    runtime: Option<Value>,
}

#[derive(Serialize, Deserialize)]
struct Delta {
    generation: String,
    sequence: u64,
    previous: String,
    upserts: Vec<Value>,
    removed: Vec<u64>,
    schemas: Value,
    systems: Value,
    runtime: Option<Value>,
}

#[derive(Serialize, Deserialize)]
struct Frame {
    checksum: String,
    payload: String,
}

fn directory(path: &Path) -> PathBuf {
    path.with_extension("journal")
}
fn stamp(path: &Path) -> Result<(u64, std::time::SystemTime), String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    Ok((meta.len(), meta.modified().map_err(|e| e.to_string())?))
}
fn read_state(path: &Path) -> Result<PersistedState, String> {
    serde_json::from_reader(BufReader::new(
        fs::File::open(path).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| e.to_string())
}

impl DeliveryJournal {
    /// Called only after a successful full save in this process's entity-ID space.
    pub fn reset(&mut self, path: &Path) -> Result<(), String> {
        let state = read_state(path)?;
        let entities = state
            .world
            .entities
            .into_iter()
            .map(|e| Ok((e.index, serde_json::to_value(e).map_err(|e| e.to_string())?)))
            .collect::<Result<_, String>>()?;
        *self = Self {
            path: path.to_owned(),
            generation: state.journal_generation.clone(),
            previous: state.journal_generation,
            sequence: 0,
            bytes: 0,
            manifest_stamp: Some(stamp(path)?),
            baseline: Some(Capture {
                entities,
                schemas: serde_json::to_value(state.schemas).unwrap(),
                systems: serde_json::to_value(state.systems).unwrap(),
                runtime: state.runtime,
            }),
        };
        Ok(())
    }

    pub fn save(
        &mut self,
        path: &Path,
        world: &World,
        registry: &ComponentActionRegistry,
        systems: &SystemRegistry,
        runtime: Option<Value>,
    ) -> Result<(), String> {
        if self.path != path
            || self.baseline.is_none()
            || self.sequence >= 128
            || self.bytes >= 32 * 1024 * 1024
        {
            persistence::checkpoint_with_metadata(path, world, registry, systems, runtime)?;
            return self.reset(path);
        }
        if Some(stamp(path)?) != self.manifest_stamp {
            return Err(
                "checkpoint changed by another writer; restart before appending delivery journal"
                    .into(),
            );
        }
        let blob_dir = path.with_extension("blobs");
        let disk = serde_json::to_value(StreamingWorld {
            world,
            registry,
            blob_dir: &blob_dir,
        })
        .map_err(|e| e.to_string())?;
        let current = Capture {
            entities: disk["entities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| (v["index"].as_u64().unwrap(), v.clone()))
                .collect(),
            schemas: serde_json::to_value(
                registry
                    .dynamic_store()
                    .iter()
                    .map(|(name, schema)| (name.to_owned(), schema.clone()))
                    .collect::<BTreeMap<_, _>>(),
            )
            .unwrap(),
            systems: serde_json::to_value(
                systems
                    .iter()
                    .filter(|s| matches!(s.kind, SystemKind::RhaiScript { .. }))
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
            runtime,
        };
        let old = self.baseline.as_ref().unwrap();
        let upserts: Vec<_> = current
            .entities
            .iter()
            .filter(|(id, e)| old.entities.get(id) != Some(e))
            .map(|(_, e)| e.clone())
            .collect();
        let removed: Vec<_> = old
            .entities
            .keys()
            .filter(|id| !current.entities.contains_key(id))
            .copied()
            .collect();
        if upserts.is_empty()
            && removed.is_empty()
            && current.schemas == old.schemas
            && current.systems == old.systems
            && current.runtime == old.runtime
        {
            return Ok(());
        }
        let delta = Delta {
            generation: self.generation.clone(),
            sequence: self.sequence + 1,
            previous: self.previous.clone(),
            upserts,
            removed,
            schemas: if current.schemas == old.schemas {
                Value::Null
            } else {
                current.schemas.clone()
            },
            systems: if current.systems == old.systems {
                Value::Null
            } else {
                current.systems.clone()
            },
            runtime: current.runtime.clone(),
        };
        let payload = serde_json::to_string(&delta).map_err(|e| e.to_string())?;
        let checksum = persistence::digest(payload.as_bytes());
        let bytes = serde_json::to_vec(&Frame {
            checksum: checksum.clone(),
            payload,
        })
        .map_err(|e| e.to_string())?;
        let dir = directory(path);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let target = dir.join(format!("{}-{:020}.json", self.generation, delta.sequence));
        // A retry may encounter a committed frame whose directory sync failed.
        if target.exists() {
            if fs::read(&target).map_err(|e| e.to_string())? != bytes {
                return Err("conflicting delivery journal writer".into());
            }
        } else {
            let temporary = dir.join(format!(".tmp-{}-{}", std::process::id(), delta.sequence));
            let result = (|| -> std::io::Result<()> {
                let mut f = fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open(&temporary)?;
                f.write_all(&bytes)?;
                f.sync_all()?;
                // Hard linking publishes without overwriting another writer's frame.
                fs::hard_link(&temporary, &target)?;
                Ok(())
            })();
            let _ = fs::remove_file(&temporary);
            result.map_err(|e| format!("write delivery journal: {e}"))?;
        }
        fs::File::open(&dir)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        let head = dir.join(format!("{}.head", self.generation));
        let temporary = dir.join(format!(".tmp-head-{}", std::process::id()));
        let head_result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)?;
            serde_json::to_writer(&mut file, &(delta.sequence, &checksum))?;
            file.sync_all()?;
            fs::rename(&temporary, &head)?;
            fs::File::open(&dir)?.sync_all()
        })();
        let _ = fs::remove_file(&temporary);
        head_result.map_err(|e| format!("commit delivery journal head: {e}"))?;
        fs::File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
        self.sequence = delta.sequence;
        self.previous = checksum;
        self.bytes += bytes.len() as u64;
        self.baseline = Some(current);
        Ok(())
    }
}

pub(crate) fn replay(path: &Path, state: &mut PersistedState) -> Result<(), String> {
    if state.journal_generation.is_empty() {
        return if state.format_version >= 3 {
            Err("missing delivery journal generation".into())
        } else {
            Ok(())
        };
    }
    let dir = directory(path);
    if !dir.exists() {
        return Ok(());
    }
    let prefix = format!("{}-", state.journal_generation);
    let head = dir.join(format!("{}.head", state.journal_generation));
    // Frames beyond the durable head were never acknowledged and are ignored.
    if !head.exists() {
        return Ok(());
    }
    let (committed, head_hash): (u64, String) = serde_json::from_reader(BufReader::new(
        fs::File::open(head).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| e.to_string())?;
    let mut paths = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    paths.retain(|p| {
        p.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&prefix)
            && p.extension().is_some_and(|e| e == "json")
            && p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix(&prefix))
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|seq| seq <= committed)
    });
    paths.sort();
    if paths.len() as u64 != committed {
        return Err("delivery journal sequence gap before committed head".into());
    }
    let mut entities: BTreeMap<_, _> = std::mem::take(&mut state.world.entities)
        .into_iter()
        .map(|e| (e.index, e))
        .collect();
    let mut previous = state.journal_generation.clone();
    for (i, path) in paths.iter().enumerate() {
        let frame: Frame = serde_json::from_reader(BufReader::new(
            fs::File::open(path).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| format!("invalid journal frame: {e}"))?;
        if persistence::digest(frame.payload.as_bytes()) != frame.checksum {
            return Err("delivery journal checksum mismatch".into());
        }
        let d: Delta = serde_json::from_str(&frame.payload).map_err(|e| e.to_string())?;
        if d.sequence != i as u64 + 1
            || d.previous != previous
            || d.generation != state.journal_generation
        {
            return Err("delivery journal sequence/generation mismatch".into());
        }
        for id in d.removed {
            entities.remove(&id);
        }
        for value in d.upserts {
            let e: crate::persistence::PersistenceEntity =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            entities.insert(e.index, e);
        }
        if !d.schemas.is_null() {
            state.schemas = serde_json::from_value(d.schemas).map_err(|e| e.to_string())?;
        }
        if !d.systems.is_null() {
            state.systems = serde_json::from_value(d.systems).map_err(|e| e.to_string())?;
        }
        state.runtime = d.runtime;
        previous = frame.checksum;
    }
    if previous != head_hash {
        return Err("delivery journal head hash mismatch".into());
    }
    state.world.entities = entities.into_values().collect();
    Ok(())
}
