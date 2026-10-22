//! File GC after compaction. Same-directory checkpoint backups remain roots.
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

fn references(v: &Value, live: &mut BTreeSet<String>) -> Result<(), String> {
    match v {
        Value::Object(map) => {
            if let Some(blob) = map.get("content_blob") {
                if !blob.is_null() {
                    let hash = blob
                        .get("sha256")
                        .and_then(Value::as_str)
                        .ok_or("invalid blob reference; GC blocked")?;
                    if !is_hash(hash) {
                        return Err("invalid blob hash; GC blocked".into());
                    }
                    live.insert(hash.into());
                }
            }
            for value in map.values() {
                references(value, live)?;
            }
        }
        Value::Array(array) => {
            for value in array {
                references(value, live)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn stamp(path: &Path) -> Result<(u64, SystemTime), String> {
    let m = fs::metadata(path).map_err(|e| e.to_string())?;
    Ok((m.len(), m.modified().map_err(|e| e.to_string())?))
}

pub fn collect(path: &Path, grace: Duration) -> Result<Value, String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .ok_or("invalid checkpoint path")?
        .to_string_lossy();
    let mut roots = vec![];
    let mut live = BTreeSet::new();
    let mut generations = BTreeSet::new();
    for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != filename && !name.starts_with(&format!("{filename}.")) {
            continue;
        }
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let p = entry.path();
        let before = stamp(&p)?;
        let v: Value = serde_json::from_reader(std::io::BufReader::new(
            fs::File::open(&p).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| format!("backup cannot be parsed; GC blocked: {e}"))?;
        if let Some(version) = v.get("format_version") {
            let version = version
                .as_u64()
                .ok_or("invalid checkpoint version; GC blocked")?;
            if version > 3 {
                return Err("unsupported checkpoint version; GC blocked".into());
            }
            if version == 3
                && !v
                    .get("journal_generation")
                    .and_then(Value::as_str)
                    .is_some_and(is_hash)
            {
                return Err("invalid journal generation; GC blocked".into());
            }
        }
        references(&v, &mut live)?;
        if let Some(g) = v
            .get("journal_generation")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            generations.insert(g.to_owned());
        }
        if before != stamp(&p)? {
            return Err("checkpoint changed during GC scan".into());
        }
        roots.push((p, before));
    }
    if !roots.iter().any(|(p, _)| p == path) {
        return Err("checkpoint root missing; GC blocked".into());
    }
    let now = SystemTime::now();
    let mut obsolete = vec![];
    let journal = path.with_extension("journal");
    if journal.exists() {
        for entry in fs::read_dir(&journal).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(generation) = name.strip_suffix(".head") {
                if is_hash(generation) && !generations.contains(generation) {
                    obsolete.push(entry.path());
                }
                continue;
            }
            let generation = name.split('-').next().unwrap_or("");
            if generations.contains(generation) {
                let frame: Value = serde_json::from_reader(std::io::BufReader::new(
                    fs::File::open(entry.path()).map_err(|e| e.to_string())?,
                ))
                .map_err(|e| e.to_string())?;
                let payload = frame["payload"]
                    .as_str()
                    .ok_or("invalid journal payload; GC blocked")?;
                if frame["checksum"].as_str()
                    != Some(crate::persistence::digest(payload.as_bytes()).as_str())
                {
                    return Err("journal checksum mismatch; GC blocked".into());
                }
                references(
                    &serde_json::from_str(payload).map_err(|e| e.to_string())?,
                    &mut live,
                )?;
            } else if is_hash(generation) && name.ends_with(".json") || name.starts_with(".tmp-") {
                obsolete.push(entry.path());
            }
        }
    }
    let blobs = path.with_extension("blobs");
    if blobs.exists() {
        for entry in fs::read_dir(&blobs).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if (is_hash(&name) && !live.contains(&name)) || name.starts_with(".tmp-") {
                obsolete.push(entry.path());
            }
        }
    }
    let targets: Vec<PathBuf> = obsolete
        .into_iter()
        .filter(|p| {
            fs::metadata(p)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= grace)
        })
        .collect();
    for (p, before) in &roots {
        if *before != stamp(p)? {
            return Err("checkpoint changed before GC; blocked".into());
        }
    }
    let mut bytes = 0;
    for p in &targets {
        bytes += fs::metadata(p).map_err(|e| e.to_string())?.len();
        fs::remove_file(p).map_err(|e| e.to_string())?;
    }
    Ok(
        serde_json::json!({"deleted_files":targets.len(),"deleted_bytes":bytes,"protected_blobs":live.len(),"backup_roots":roots.len(),"grace_secs":grace.as_secs()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_and_active_journal_protect_blobs() {
        let dir = std::env::temp_dir().join(format!(
            "gc-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("checkpoint.json");
        let blob = path.with_extension("blobs");
        fs::create_dir(&blob).unwrap();
        let live = "a".repeat(64);
        let dead = "b".repeat(64);
        let journal_hash = "c".repeat(64);
        let generation = "d".repeat(64);
        fs::write(
            &path,
            serde_json::json!({"journal_generation":generation}).to_string(),
        )
        .unwrap();
        let backup = dir.join("checkpoint.json.bak");
        fs::write(
            &backup,
            serde_json::json!({"content_blob":{"sha256":live}}).to_string(),
        )
        .unwrap();
        for hash in [&live, &dead, &journal_hash] {
            fs::write(blob.join(hash), "data").unwrap();
        }
        let log = path.with_extension("journal");
        fs::create_dir(&log).unwrap();
        let payload = serde_json::json!({"content_blob":{"sha256":journal_hash}}).to_string();
        fs::write(log.join(format!("{generation}-00000000000000000001.json")),serde_json::json!({"checksum":crate::persistence::digest(payload.as_bytes()),"payload":payload}).to_string()).unwrap();
        assert_eq!(
            collect(&path, Duration::from_secs(86400)).unwrap()["deleted_files"],
            0
        );
        assert_eq!(collect(&path, Duration::ZERO).unwrap()["deleted_files"], 1);
        assert!(blob.join(live).exists());
        assert!(blob.join(journal_hash).exists());
        fs::write(backup, "broken").unwrap();
        assert!(collect(&path, Duration::ZERO).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
