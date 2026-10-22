//! Rhai 脚本引擎与 ECS 世界绑定。

pub mod scripting;

pub use scripting::{build_ecs_engine, check_syntax, eval_with_world, eval_with_world_captured};
