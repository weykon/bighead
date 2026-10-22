//! Durable metadata for an OS process started by an agent.
//!
//! The ECS stores facts that survive a checkpoint. It deliberately does not
//! store a child handle; after restore the runtime must probe the PID again.

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedProcess {
    pub process_id: String,
    pub name: String,
    pub pid: u32,
    pub command_display: String,
    pub working_dir: String,
    pub purpose: String,
    pub owner_spark: String,
    pub owner_conversation: String,
    pub started_at: u64,
    pub last_seen: u64,
    pub state: String,
    pub exit_code: Option<i32>,
    pub log_path: String,
    pub health_url: String,
    pub port: Option<u16>,
}

impl ManagedProcess {
    pub fn new(
        process_id: String,
        name: String,
        pid: u32,
        command_display: String,
        working_dir: String,
        purpose: String,
        log_path: String,
        health_url: String,
        port: Option<u16>,
        now: u64,
    ) -> Self {
        Self {
            process_id,
            name,
            pid,
            command_display,
            working_dir,
            purpose,
            owner_spark: String::new(),
            owner_conversation: String::new(),
            started_at: now,
            last_seen: now,
            state: "Starting".into(),
            exit_code: None,
            log_path,
            health_url,
            port,
        }
    }
}
