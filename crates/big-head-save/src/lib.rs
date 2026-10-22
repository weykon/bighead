//! 世界快照和文件持久化。

pub mod journal;
#[cfg(test)]
mod journal_tests;
pub mod persistence;
pub mod read;
pub mod reclamation;
pub mod snapshot;

pub use persistence::{PersistedState, build_snapshot, checkpoint, restore};
pub use read::{execute_get_component, execute_get_entity};
pub use snapshot::{
    ComponentRegistryEntry, ComponentSnapshot, EntitySnapshot, WorldSnapshot,
    snapshot_markdown_with_dynamic_reader, snapshot_with_dynamic_reader,
};
