//! `big-head-cli` — thin CLI execution for AI agents.
//!
//! Provides:
//! - **command**: [`CliCommand`], [`CliResult`] types + lightweight [`SecurityValidator`]
//! - **executor**: [`CliExecutor`] — sync execution with smart output truncation
//!
//! Inspired by Claude Code's Bash tool: separates execution from governance,
//! keeps the executor thin, security as a composable layer.

pub mod command;
pub mod executor;

pub use command::{CliCommand, CliResult, SecurityResult, SecurityValidator};
pub use executor::CliExecutor;
