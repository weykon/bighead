//! Dispatch for the `execute_cli` tool — shell command execution.

use big_head_cli::{CliCommand, CliExecutor, SecurityValidator};

use bevy_ecs::world::World;
use big_head_ai::{AgentAction, ToolCall};
use big_head_ecs::ManagedProcess;
use uuid::Uuid;

/// Dispatch `execute_cli` tool. Returns `(output, is_error, action)`.
pub(crate) fn dispatch_execute_cli(
    world: &mut World,
    security: &SecurityValidator,
    executor: &CliExecutor,
    call: &ToolCall,
) -> (String, bool, Option<AgentAction>) {
    let command = match call.input.get("command").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                "execute_cli: missing 'command' parameter".into(),
                true,
                None,
            );
        }
    };

    let working_dir = call
        .input
        .get("working_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let timeout_secs = call.input.get("timeout_secs").and_then(|v| v.as_u64());

    let description = call
        .input
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let background = call
        .input
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let name = call
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("background-command")
        .to_string();
    let purpose = call
        .input
        .get("purpose")
        .and_then(|v| v.as_str())
        .or(description.as_deref())
        .unwrap_or("Agent-started background process")
        .to_string();
    let health_url = call
        .input
        .get("health_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let port = call
        .input
        .get("port")
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok());

    let cmd = CliCommand {
        command: command.clone(),
        working_dir: working_dir.clone(),
        timeout_secs,
        description,
        background,
    };

    // Security check
    if let big_head_cli::SecurityResult::Denied(reason) = security.validate(&cmd) {
        return (format!("Command denied: {}", reason), true, None);
    }

    // Execute
    match executor.execute(cmd) {
        Ok(result) => {
            let managed_id = if let Some(pid) = result.pid {
                let now = big_head_ecs::execution::now_secs();
                let mut process = ManagedProcess::new(
                    Uuid::new_v4().to_string(),
                    name,
                    pid,
                    command.clone(),
                    working_dir.clone().unwrap_or_else(|| {
                        std::env::current_dir()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| ".".into())
                    }),
                    purpose,
                    result.log_path.clone().unwrap_or_default(),
                    health_url,
                    port,
                    now,
                );
                process.owner_spark = call
                    .input
                    .get("owner_spark")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let id = process.process_id.clone();
                world.spawn(process);
                Some(id)
            } else {
                None
            };
            let truncated_info = if result.truncated {
                format!(
                    "\n[Output truncated: {} total lines, {} error lines extracted. Full output: {}]",
                    result.total_lines,
                    result.error_lines.len(),
                    result.full_output_path.as_deref().unwrap_or("N/A"),
                )
            } else {
                String::new()
            };

            let error_section = if !result.error_lines.is_empty() {
                format!(
                    "\n--- Error/Warning lines from omitted section ---\n{}\n",
                    result.error_lines.join("\n")
                )
            } else {
                String::new()
            };

            // Add PID + log path for background commands
            let bg_info = match (result.pid, result.log_path) {
                (Some(pid), Some(log)) => format!("\nPID: {} (background)\nLog: {}", pid, log),
                (Some(pid), None) => format!("\nPID: {} (background)", pid),
                _ => String::new(),
            };

            let managed_info = managed_id
                .as_deref()
                .map(|id| format!("\nManaged process: {} (visible in TUI Processes)", id))
                .unwrap_or_default();
            let output = format!(
                "Command: {}\nExit code: {}\nDuration: {}ms{}{}{}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}{}",
                command,
                result.exit_code.map_or("N/A".into(), |c| c.to_string()),
                result.duration_ms,
                bg_info,
                truncated_info,
                managed_info,
                result.stdout,
                result.stderr,
                error_section,
            );

            // Background commands intentionally have no exit code yet. A PID
            // means the launch request succeeded; lifecycle failures are
            // reported later by ManagedProcess reconciliation.
            let is_error = if result.pid.is_some() && !result.timed_out {
                false
            } else {
                result.exit_code != Some(0) || result.timed_out
            };
            let action = AgentAction::ExecuteCli {
                command: command.clone(),
                exit_code: result.exit_code,
                duration_ms: result.duration_ms,
                timed_out: result.timed_out,
            };
            (output, is_error, Some(action))
        }
        Err(e) => (format!("CLI execution error: {}", e), true, None),
    }
}
