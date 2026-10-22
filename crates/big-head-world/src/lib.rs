//! World identity and Lens exchange primitives.
//!
//! A checkpoint is one sovereign ECS world. This crate models the layer above
//! checkpoints: local/peer world identity, stable object references, and
//! bucket-based Lens transfer between agent worlds.

use std::fs;
pub mod application;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_WORLD_ROOT: &str = "big_head_worlds";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldTrust {
    SelfWorld,
    KnownPeer,
    Unknown,
}

impl Default for WorldTrust {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldIdentity {
    pub world_id: String,
    pub display_name: String,
    pub owner: Option<String>,
    pub endpoint: Option<String>,
    pub public_key: Option<String>,
    #[serde(default)]
    pub trust: WorldTrust,
}

impl WorldIdentity {
    pub fn local(world_id: impl Into<String>) -> Self {
        let world_id = world_id.into();
        Self {
            display_name: world_id.clone(),
            world_id,
            owner: None,
            endpoint: None,
            public_key: None,
            trust: WorldTrust::SelfWorld,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldPaths {
    pub root_dir: PathBuf,
    pub checkpoint_path: PathBuf,
    pub view_snapshot_path: PathBuf,
    pub inbox_dir: PathBuf,
    pub outbox_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldManifest {
    pub identity: WorldIdentity,
    pub paths: WorldPaths,
    #[serde(default)]
    pub peers: Vec<WorldIdentity>,
}

impl WorldManifest {
    pub fn local(world_id: impl Into<String>) -> Result<Self, String> {
        let identity = WorldIdentity::local(world_id);
        Self::from_identity(identity)
    }

    pub fn from_identity(identity: WorldIdentity) -> Result<Self, String> {
        let root_dir = PathBuf::from(DEFAULT_WORLD_ROOT).join(safe_world_dir(&identity.world_id)?);
        Ok(Self {
            paths: WorldPaths {
                checkpoint_path: root_dir.join("checkpoint.json"),
                view_snapshot_path: root_dir.join("view_snapshot.json"),
                inbox_dir: root_dir.join("lens").join("inbox"),
                outbox_dir: root_dir.join("lens").join("outbox"),
                root_dir,
            },
            identity,
            peers: Vec::new(),
        })
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(&self.paths.root_dir)
            .map_err(|e| format!("create {}: {e}", self.paths.root_dir.display()))?;
        fs::create_dir_all(&self.paths.inbox_dir)
            .map_err(|e| format!("create {}: {e}", self.paths.inbox_dir.display()))?;
        fs::create_dir_all(&self.paths.outbox_dir)
            .map_err(|e| format!("create {}: {e}", self.paths.outbox_dir.display()))?;
        Ok(())
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.paths.root_dir.join("world.json")
    }

    pub fn save(&self) -> Result<(), String> {
        self.ensure_dirs()?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize world manifest: {e}"))?;
        fs::write(self.manifest_path(), json)
            .map_err(|e| format!("write {}: {e}", self.manifest_path().display()))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let json = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&json).map_err(|e| format!("parse {}: {e}", path.display()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldObjectRef {
    pub world_id: String,
    pub object_id: String,
    pub object_type: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensPermission {
    #[serde(default)]
    pub can_store: bool,
    #[serde(default)]
    pub can_derive: bool,
    #[serde(default)]
    pub can_forward: bool,
}

impl Default for LensPermission {
    fn default() -> Self {
        Self {
            can_store: true,
            can_derive: true,
            can_forward: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lens {
    pub lens_id: String,
    pub from_world: String,
    pub to_world: Option<String>,
    pub kind: String,
    pub intent: String,
    pub summary: String,
    #[serde(default)]
    pub refs: Vec<WorldObjectRef>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub permissions: LensPermission,
    pub created_at: u64,
}

impl Lens {
    pub fn new(
        from_world: impl Into<String>,
        to_world: Option<String>,
        kind: impl Into<String>,
        intent: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            lens_id: format!("lens_{}", Uuid::new_v4()),
            from_world: from_world.into(),
            to_world,
            kind: kind.into(),
            intent: intent.into(),
            summary: summary.into(),
            refs: Vec::new(),
            payload: serde_json::Value::Object(Default::default()),
            permissions: LensPermission::default(),
            created_at: now_secs(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LensBucket {
    root: PathBuf,
}

impl LensBucket {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn pending_path(&self) -> PathBuf {
        self.root.join("pending.jsonl")
    }

    pub fn accepted_path(&self) -> PathBuf {
        self.root.join("accepted.jsonl")
    }

    pub fn rejected_path(&self) -> PathBuf {
        self.root.join("rejected.jsonl")
    }

    pub fn derived_path(&self) -> PathBuf {
        self.root.join("derived.jsonl")
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| format!("create {}: {e}", self.root.display()))
    }

    pub fn append_pending(&self, lens: &Lens) -> Result<(), String> {
        self.append_jsonl(&self.pending_path(), lens)
    }

    pub fn append_accepted(&self, lens: &Lens) -> Result<(), String> {
        self.append_jsonl(&self.accepted_path(), lens)
    }

    pub fn append_rejected(&self, lens: &Lens) -> Result<(), String> {
        self.append_jsonl(&self.rejected_path(), lens)
    }

    pub fn append_derived(&self, value: &serde_json::Value) -> Result<(), String> {
        self.append_jsonl(&self.derived_path(), value)
    }

    pub fn read_pending(&self) -> Result<Vec<Lens>, String> {
        self.read_jsonl(&self.pending_path())
    }

    pub fn read_accepted(&self) -> Result<Vec<Lens>, String> {
        self.read_jsonl(&self.accepted_path())
    }

    pub fn read_rejected(&self) -> Result<Vec<Lens>, String> {
        self.read_jsonl(&self.rejected_path())
    }

    pub fn find_pending(&self, lens_id: &str) -> Result<Option<Lens>, String> {
        Ok(self
            .read_pending()?
            .into_iter()
            .find(|lens| lens.lens_id == lens_id))
    }

    pub fn accept_pending(&self, lens_id: &str) -> Result<Lens, String> {
        let lens = self
            .find_pending(lens_id)?
            .ok_or_else(|| format!("pending lens not found: {lens_id}"))?;
        self.append_accepted(&lens)?;
        Ok(lens)
    }

    pub fn reject_pending(&self, lens_id: &str) -> Result<Lens, String> {
        let lens = self
            .find_pending(lens_id)?
            .ok_or_else(|| format!("pending lens not found: {lens_id}"))?;
        self.append_rejected(&lens)?;
        Ok(lens)
    }

    fn append_jsonl<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), String> {
        self.ensure_dirs()?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        let json = serde_json::to_string(value).map_err(|e| format!("serialize jsonl: {e}"))?;
        writeln!(file, "{json}").map_err(|e| format!("write {}: {e}", path.display()))
    }

    fn read_jsonl<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<Vec<T>, String> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut out = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value = serde_json::from_str(line)
                .map_err(|e| format!("parse {} line {}: {e}", path.display(), index + 1))?;
            out.push(value);
        }
        Ok(out)
    }
}

fn safe_world_dir(world_id: &str) -> Result<String, String> {
    let safe: String = world_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe = safe.trim_matches('-').to_string();
    if safe.is_empty() {
        return Err("world id must contain at least one ascii letter, digit, '-' or '_'".into());
    }
    Ok(safe)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_manifest_uses_isolated_paths() {
        let manifest = WorldManifest::local("world:weykon").unwrap();
        assert_eq!(manifest.identity.world_id, "world:weykon");
        assert!(manifest.paths.checkpoint_path.ends_with("checkpoint.json"));
        assert!(manifest.paths.root_dir.ends_with("world-weykon"));
    }

    #[test]
    fn bucket_round_trips_pending_lenses() {
        let root = std::env::temp_dir().join(format!("big-head-lens-test-{}", Uuid::new_v4()));
        let bucket = LensBucket::new(&root);
        let mut lens = Lens::new(
            "world:weykon",
            Some("world:alice".into()),
            "proposal",
            "share_context",
            "Share an ambient dwelling insight",
        );
        lens.refs.push(WorldObjectRef {
            world_id: "world:weykon".into(),
            object_id: "topic-ambient-dwelling-001".into(),
            object_type: "Topic".into(),
            version: Some("sha256:test".into()),
        });

        bucket.append_pending(&lens).unwrap();
        let restored = bucket.read_pending().unwrap();
        assert_eq!(restored, vec![lens]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bucket_accepts_and_rejects_pending_lenses_append_only() {
        let root = std::env::temp_dir().join(format!("big-head-lens-test-{}", Uuid::new_v4()));
        let bucket = LensBucket::new(&root);
        let accept = Lens::new(
            "world:weykon",
            Some("world:alice".into()),
            "request",
            "review",
            "Review one topic",
        );
        let reject = Lens::new(
            "world:alice",
            Some("world:weykon".into()),
            "proposal",
            "store",
            "Store external context",
        );

        bucket.append_pending(&accept).unwrap();
        bucket.append_pending(&reject).unwrap();
        assert_eq!(bucket.accept_pending(&accept.lens_id).unwrap(), accept);
        assert_eq!(bucket.reject_pending(&reject.lens_id).unwrap(), reject);
        assert_eq!(bucket.read_accepted().unwrap().len(), 1);
        assert_eq!(bucket.read_rejected().unwrap().len(), 1);
        assert_eq!(bucket.read_pending().unwrap().len(), 2);
        let _ = fs::remove_dir_all(root);
    }
}
