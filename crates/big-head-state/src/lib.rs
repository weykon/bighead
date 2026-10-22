//! Agent activity/state tracking crate.
//!
//! Provides:
//! - [`AgentActivity`] — enum of observable agent states
//! - [`StateHook`] — cloneable broadcast-based hook (follows `LogEmitter` pattern)
//! - [`ActivityGuard`] — RAII guard for scoped activity emissions
//! - [`TerminalRenderer`] — ANSI-colored terminal output subscriber

mod activity;
mod hook;
mod render;

pub use activity::{AgentActivity, WaveContentKind};
pub use hook::{ActivityGuard, StateHook};
pub use render::TerminalRenderer;
