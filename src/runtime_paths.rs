use std::fs;
use std::path::{Path, PathBuf};

use big_head_world::WorldManifest;

const LEGACY_CHECKPOINT_PATH: &str = "big_head_checkpoint.json";
const LEGACY_VIEW_SNAPSHOT_PATH: &str = "big_head_view_snapshot.json";

#[derive(Debug, Clone)]
pub(crate) struct RuntimePaths {
    pub(crate) world: WorldManifest,
    pub(crate) checkpoint: PathBuf,
    pub(crate) view_snapshot: PathBuf,
}

impl RuntimePaths {
    pub(crate) fn from_args(args: &[String]) -> Result<Self, String> {
        let world_id = arg_value(args, "--world").unwrap_or_else(|| "local:default".into());
        let world = WorldManifest::local(world_id)?;
        world.ensure_dirs()?;
        world.save()?;
        migrate_legacy_default_world_files(&world);

        Ok(Self {
            checkpoint: world.paths.checkpoint_path.clone(),
            view_snapshot: world.paths.view_snapshot_path.clone(),
            world,
        })
    }

    pub(crate) fn world_id(&self) -> &str {
        &self.world.identity.world_id
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn migrate_legacy_default_world_files(manifest: &WorldManifest) {
    if manifest.identity.world_id != "local:default" {
        return;
    }
    copy_legacy_file_if_missing(
        LEGACY_CHECKPOINT_PATH,
        &manifest.paths.checkpoint_path,
        "checkpoint",
    );
    copy_legacy_file_if_missing(
        LEGACY_VIEW_SNAPSHOT_PATH,
        &manifest.paths.view_snapshot_path,
        "view snapshot",
    );
}

fn copy_legacy_file_if_missing(legacy_path: &str, target_path: &Path, label: &str) {
    if target_path.exists() {
        return;
    }
    let legacy = Path::new(legacy_path);
    if !legacy.exists() {
        return;
    }
    // Copy all recovery dependencies before publishing the migrated checkpoint.
    if label == "checkpoint" {
        let result = (|| -> std::io::Result<()> {
            for extension in ["blobs", "journal"] {
                let source = legacy.with_extension(extension);
                if !source.exists() {
                    continue;
                }
                let destination = target_path.with_extension(extension);
                fs::create_dir_all(&destination)?;
                for entry in fs::read_dir(source)? {
                    let entry = entry?;
                    if entry.file_type()?.is_file()
                        && !entry.file_name().to_string_lossy().starts_with('.')
                    {
                        fs::copy(entry.path(), destination.join(entry.file_name()))?;
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Legacy checkpoint sidecar migration failed: {error}");
            return;
        }
    }
    if let Err(e) = fs::copy(legacy, target_path) {
        eprintln!(
            "Legacy {label} migration failed: {} -> {}: {}",
            legacy.display(),
            target_path.display(),
            e
        );
    }
}
