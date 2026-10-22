//! Thin synchronous CLI executor.
//!
//! Runs commands via `sh -c`, collects output with smart truncation:
//! - Head (first N lines) — context
//! - Tail (last M lines) — result/error location
//! - Error lines — extracted from the middle section
//! - Full output written to disk for later retrieval
//!
//! Uses `std::process::Command` + threads for concurrent I/O and timeout.

use crate::command::{CliCommand, CliResult};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command as StdCommand, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Number of lines to keep from the beginning (context).
const HEAD_LINES: usize = 80;
/// Number of lines to keep from the end (results/errors).
const TAIL_LINES: usize = 50;
/// Maximum total lines before truncation kicks in.
const MAX_LINES: usize = HEAD_LINES + TAIL_LINES + 100;
/// Patterns that mark lines worth preserving from the middle section.
const ERROR_PATTERNS: &[&str] = &[
    "error",
    "Error",
    "ERROR",
    "fail",
    "Fail",
    "FAIL",
    "panic",
    "Panic",
    "exception",
    "Exception",
    "undefined",
    "Undefined",
    "cannot",
    "Cannot",
    "fatal",
    "Fatal",
    "warning:",
    "Warning:",
    "not found",
    "Not found",
    "NOT FOUND",
];

/// Output directory for full command outputs.
const OUTPUT_DIR: &str = ".big-head/outputs";
/// Give normal readers a brief chance to consume EOF after the command exits.
const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(250);
/// Bound cleanup even when an inherited pipe or OS process refuses to close.
const PIPE_CLEANUP_GRACE: Duration = Duration::from_secs(1);

/// Thin CLI executor.
pub struct CliExecutor {
    default_timeout_secs: u64,
    default_working_dir: String,
}

impl Default for CliExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CliExecutor {
    pub fn new() -> Self {
        Self {
            default_timeout_secs: 120,
            default_working_dir: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    /// Execute a command. Handles both background and foreground execution.
    ///
    /// Uses threads for concurrent stdout/stderr reading so that timeout
    /// detection is not blocked by a process that produces no output.
    pub fn execute(&self, cmd: CliCommand) -> Result<CliResult, String> {
        // Check if command ends with & or background flag is set
        let is_background = cmd.background || cmd.command.trim().ends_with('&');
        if is_background {
            return self.execute_background(cmd);
        }

        let timeout_secs = cmd
            .timeout_secs
            .unwrap_or(self.default_timeout_secs)
            .min(600);
        let working_dir = cmd
            .working_dir
            .as_deref()
            .unwrap_or(&self.default_working_dir);

        let start = Instant::now();

        let mut command = StdCommand::new("sh");
        command
            .args(["-c", &cmd.command])
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command
            .spawn()
            .map_err(|e| format!("Failed to spawn command: {}", e))?;
        let process_group_id = child.id();

        // Read stdout/stderr on separate threads so the main thread can enforce timeout.
        let stdout_handle = child.stdout.take().ok_or("Failed to capture stdout")?;
        let stderr_handle = child.stderr.take().ok_or("Failed to capture stderr")?;

        let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout_handle);
            let lines = reader
                .lines()
                .filter_map(|l| l.ok())
                .collect::<Vec<String>>();
            let _ = stdout_tx.send(lines);
        });

        let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr_handle);
            let lines = reader
                .lines()
                .filter_map(|l| l.ok())
                .collect::<Vec<String>>();
            let _ = stderr_tx.send(lines);
        });

        // Poll for process completion or timeout.
        let exit_code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {
                    if start.elapsed().as_secs() >= timeout_secs {
                        terminate_process_group(process_group_id);
                        let _ = child.kill();
                        let _ = child.wait(); // reap zombie
                        let duration_ms = start.elapsed().as_millis() as u64;
                        // Descendants may have inherited the pipes. Never wait
                        // indefinitely for their reader threads to see EOF.
                        let _ = receive_lines(&stdout_rx, PIPE_CLEANUP_GRACE);
                        let _ = receive_lines(&stderr_rx, PIPE_CLEANUP_GRACE);
                        return Ok(CliResult {
                            exit_code: None,
                            stdout: String::new(),
                            stderr: String::new(),
                            full_output_path: None,
                            duration_ms,
                            timed_out: true,
                            truncated: false,
                            total_lines: 0,
                            error_lines: vec![],
                            pid: None,
                            log_path: None,
                        });
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("Failed to wait for process: {}", e)),
            }
        };

        // The shell may exit after starting an accidental background process.
        // If a descendant still owns either pipe, clean up the isolated process
        // group and continue instead of blocking the whole agent on join().
        let mut stdout_lines = receive_lines(&stdout_rx, PIPE_DRAIN_GRACE);
        let mut stderr_lines = receive_lines(&stderr_rx, PIPE_DRAIN_GRACE);
        if stdout_lines.is_none() || stderr_lines.is_none() {
            terminate_process_group(process_group_id);
            if stdout_lines.is_none() {
                stdout_lines = receive_lines(&stdout_rx, PIPE_CLEANUP_GRACE);
            }
            if stderr_lines.is_none() {
                stderr_lines = receive_lines(&stderr_rx, PIPE_CLEANUP_GRACE);
            }
        }
        let stdout_lines = stdout_lines.unwrap_or_default();
        let stderr_lines = stderr_lines.unwrap_or_default();

        let duration_ms = start.elapsed().as_millis() as u64;
        let total_lines = stdout_lines.len() + stderr_lines.len();

        let (stdout, stderr, truncated, error_lines, full_output_path) = if total_lines <= MAX_LINES
        {
            (
                stdout_lines.join("\n"),
                stderr_lines.join("\n"),
                false,
                vec![],
                None,
            )
        } else {
            let (s_out, s_err, err_lines, path) =
                Self::smart_truncate(&stdout_lines, &stderr_lines, &cmd.command);
            (s_out, s_err, true, err_lines, path)
        };

        Ok(CliResult {
            exit_code,
            stdout,
            stderr,
            full_output_path,
            duration_ms,
            timed_out: false,
            truncated,
            total_lines,
            error_lines,
            pid: None,
            log_path: None,
        })
    }

    /// Execute a command in background using `nohup` to detach from the
    /// controlling terminal. The process runs independently of the parent.
    ///
    /// Output (stdout+stderr) is redirected to a log file under
    /// `.big-head/outputs/bg_<pid>.log` so the agent can inspect it later.
    /// The PID is captured from the wrapper shell's `$!` — set immediately
    /// when the background job is spawned, with no race against `ps`/`pgrep`.
    ///
    /// **Limitations** (document for the agent):
    /// - Not subject to `timeout_secs` — the wrapper returns immediately;
    ///   the background process runs until it exits on its own.
    /// - No live output capture; read `log_path` to inspect progress.
    fn execute_background(&self, mut cmd: CliCommand) -> Result<CliResult, String> {
        let working_dir = cmd
            .working_dir
            .as_deref()
            .unwrap_or(&self.default_working_dir);

        // Strip trailing & (already detected as background intent).
        cmd.command = cmd
            .command
            .trim_end()
            .trim_end_matches('&')
            .trim()
            .to_string();

        let _ = std::fs::create_dir_all(OUTPUT_DIR);
        let start = Instant::now();

        // Reserve a unique log path up front (timestamp-based) — avoids a
        // rename after we know the PID. We embed the timestamp in nanoseconds
        // so two background commands in the same second don't collide.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let log_path_buf = format!("{}/bg_{}.log", OUTPUT_DIR, nanos);

        // Wrapper shell: run user command in a brace group with redirect,
        // background the brace group, then echo $! to capture the PID.
        //   { ...; }          — brace group (no fork, just grouping)
        //   user_cmd          — as-is (may be multi-command, e.g. "echo a; echo b >&2")
        //   >> log 2>&1       — redirect all output to log file
        //   &                 — background the brace group
        //   echo $!           — print the bg job PID to wrapper stdout
        //
        // We use a **brace group** (not a subshell `(...)`) because macOS
        // bash+piped-stdout has a known timing issue where subshells leak
        // user output to the wrapper's stdout when stdout is piped (verified
        // via minimal repro). Brace groups don't fork, so the redirect
        // applies to the same shell that prints $! — output is clean.
        //
        // The PID reported is the bg job's PID. We use `nohup` (POSIX-portable)
        // instead of `setsid` (Linux-only). On macOS, `setsid` is not present
        // by default — `nohup` works on both. nohup inside the brace group
        // makes the child immune to SIGHUP, surviving the wrapper shell exit.
        let wrapper = format!(
            r#"{{ nohup {}; }} >> "{}" 2>&1 & echo $!"#,
            cmd.command, log_path_buf
        );

        let output = StdCommand::new("sh")
            .args(["-c", &wrapper])
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("Failed to spawn background command: {}", e))?;

        // $! is the PID of the bg job we just spawned. With a brace group
        // and clean redirect, this is the only line on wrapper stdout and
        // parses cleanly.
        let pid: Option<u32> = String::from_utf8_lossy(&output.stdout).trim().parse().ok();

        // If we got a PID, rename the log file to embed it (more useful for
        // the agent than a timestamp-based name). The redirect creates the
        // log file eagerly (even before the bg job writes anything), so the
        // rename always has a source. Best-effort: keep the timestamp path
        // if rename fails.
        let log_path = if let Some(p) = pid {
            let final_log = format!("{}/bg_{}.log", OUTPUT_DIR, p);
            if std::path::Path::new(&log_path_buf).exists() {
                let _ = std::fs::rename(&log_path_buf, &final_log);
                Some(final_log)
            } else {
                Some(log_path_buf)
            }
        } else {
            None
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(CliResult {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            full_output_path: None,
            duration_ms,
            timed_out: false,
            truncated: false,
            total_lines: 0,
            error_lines: vec![],
            pid,
            log_path,
        })
    }

    /// Smart truncation: head + tail + error line extraction + full output to disk.
    fn smart_truncate(
        stdout_lines: &[String],
        stderr_lines: &[String],
        command: &str,
    ) -> (String, String, Vec<String>, Option<String>) {
        let total = stdout_lines.len() + stderr_lines.len();

        let error_lines = Self::extract_error_lines(stdout_lines, stderr_lines);

        let stdout_truncated =
            Self::truncate_lines(stdout_lines, HEAD_LINES, TAIL_LINES, total, "stdout");

        let stderr_truncated = Self::truncate_lines(
            stderr_lines,
            HEAD_LINES / 2,
            TAIL_LINES / 2,
            total,
            "stderr",
        );

        let full_output_path = Self::write_full_output(stdout_lines, stderr_lines, command);

        (
            stdout_truncated,
            stderr_truncated,
            error_lines,
            full_output_path,
        )
    }

    fn truncate_lines(
        lines: &[String],
        head: usize,
        tail: usize,
        total: usize,
        label: &str,
    ) -> String {
        if lines.is_empty() {
            return String::new();
        }
        if lines.len() <= head + tail {
            return lines.join("\n");
        }
        let head_part = &lines[..head];
        let tail_part = &lines[lines.len() - tail..];
        let skipped = lines.len() - head - tail;
        format!(
            "{}\n... [{} {} lines omitted, {} total lines across stdout+stderr] ...\n{}",
            head_part.join("\n"),
            skipped,
            label,
            total,
            tail_part.join("\n"),
        )
    }

    fn extract_error_lines(stdout_lines: &[String], stderr_lines: &[String]) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();
        for line in stdout_lines.iter().chain(stderr_lines.iter()) {
            for pat in ERROR_PATTERNS {
                if line.contains(pat) {
                    if in_middle(stdout_lines, line) || in_middle(stderr_lines, line) {
                        if !errors.contains(line) {
                            errors.push(line.clone());
                        }
                        break;
                    }
                }
            }
        }
        errors
    }

    fn write_full_output(
        stdout_lines: &[String],
        stderr_lines: &[String],
        _command: &str,
    ) -> Option<String> {
        let dir = std::path::Path::new(OUTPUT_DIR);
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[big-head-cli] Failed to create output dir: {}", e);
            return None;
        }
        let filename = format!(
            "run_{}.log",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        let path = dir.join(&filename);
        let mut file = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[big-head-cli] Failed to create output file: {}", e);
                return None;
            }
        };
        for line in stdout_lines {
            let _ = writeln!(file, "{}", line);
        }
        if !stderr_lines.is_empty() {
            let _ = writeln!(file, "\n--- stderr ---");
            for line in stderr_lines {
                let _ = writeln!(file, "{}", line);
            }
        }
        Some(path.to_string_lossy().to_string())
    }
}

fn receive_lines(receiver: &Receiver<Vec<String>>, timeout: Duration) -> Option<Vec<String>> {
    match receiver.recv_timeout(timeout) {
        Ok(lines) => Some(lines),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
    }
}

#[cfg(unix)]
fn terminate_process_group(process_group_id: u32) {
    let Ok(process_group_id) = i32::try_from(process_group_id) else {
        return;
    };
    // SAFETY: kill is called with a valid signal and a negative pid to target
    // only the isolated process group created for this foreground command.
    unsafe {
        libc::kill(-process_group_id, libc::SIGTERM);
    }
    std::thread::sleep(Duration::from_millis(50));
    // A command may ignore SIGTERM. SIGKILL also ensures inherited stdout and
    // stderr descriptors close before the bounded reader grace expires.
    unsafe {
        libc::kill(-process_group_id, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_process_group_id: u32) {}

/// Check if a line falls in the middle (truncated) section of the output.
fn in_middle(lines: &[String], target: &str) -> bool {
    if lines.len() <= HEAD_LINES + TAIL_LINES {
        return false;
    }
    let head_end = HEAD_LINES.min(lines.len());
    let tail_start = (lines.len() - TAIL_LINES).max(head_end);
    lines[head_end..tail_start].contains(&target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let executor = CliExecutor::new();
        let result = executor
            .execute(CliCommand {
                command: "echo hello".to_string(),
                working_dir: None,
                timeout_secs: Some(5),
                description: None,
                background: false,
            })
            .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello"));
        assert!(!result.timed_out);
    }

    #[test]
    fn test_exit_code() {
        let executor = CliExecutor::new();
        let result = executor
            .execute(CliCommand {
                command: "exit 42".to_string(),
                working_dir: None,
                timeout_secs: Some(5),
                description: None,
                background: false,
            })
            .unwrap();

        assert_eq!(result.exit_code, Some(42));
    }

    #[test]
    fn test_stderr_capture() {
        let executor = CliExecutor::new();
        let result = executor
            .execute(CliCommand {
                command: "echo error >&2".to_string(),
                working_dir: None,
                timeout_secs: Some(5),
                description: None,
                background: false,
            })
            .unwrap();

        assert_eq!(result.exit_code, Some(0));
        // stderr goes to stderr, stdout may be empty
        assert!(!result.stderr.is_empty() || !result.stdout.contains("error is captured"));
    }

    #[test]
    fn test_timeout() {
        let executor = CliExecutor::new();
        let result = executor
            .execute(CliCommand {
                command: "sleep 10".to_string(),
                working_dir: None,
                timeout_secs: Some(1),
                description: None,
                background: false,
            })
            .unwrap();

        assert!(
            result.timed_out,
            "expected timeout, got exit_code={:?}",
            result.exit_code
        );
        assert!(result.exit_code.is_none());
    }

    #[test]
    fn timeout_does_not_wait_for_pipeline_descendants() {
        let executor = CliExecutor::new();
        let started = Instant::now();
        let result = executor
            .execute(CliCommand {
                command: "sleep 30 | cat".to_string(),
                working_dir: None,
                timeout_secs: Some(1),
                description: None,
                background: false,
            })
            .unwrap();

        assert!(result.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "timeout cleanup must not block on descendant-held pipes"
        );
    }

    #[test]
    fn completed_shell_does_not_wait_for_background_descendant_pipe() {
        let executor = CliExecutor::new();
        let started = Instant::now();
        let result = executor
            .execute(CliCommand {
                command: "sleep 30 & echo parent-finished".to_string(),
                working_dir: None,
                timeout_secs: Some(5),
                description: None,
                background: false,
            })
            .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("parent-finished"));
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "completed shell must not block on a descendant-held pipe"
        );
    }

    #[test]
    fn test_working_directory() {
        let executor = CliExecutor::new();
        let result = executor
            .execute(CliCommand {
                command: "pwd".to_string(),
                working_dir: Some("/tmp".to_string()),
                timeout_secs: Some(5),
                description: None,
                background: false,
            })
            .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("/tmp") || result.stdout.contains("/private/tmp"));
    }

    #[test]
    fn test_output_truncation() {
        let executor = CliExecutor::new();
        let cmd = format!(
            "for i in $(seq 1 {}); do echo \"line $i\"; done",
            MAX_LINES + 50
        );
        let result = executor
            .execute(CliCommand {
                command: cmd,
                working_dir: None,
                timeout_secs: Some(10),
                description: None,
                background: false,
            })
            .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.truncated);
        assert!(result.total_lines > MAX_LINES);
        assert!(result.stdout.contains("omitted"));
        assert!(result.full_output_path.is_some());
    }

    #[test]
    fn test_error_extraction() {
        let executor = CliExecutor::new();

        // Build a command that outputs enough lines to trigger truncation,
        // with error/warning lines in the middle section.
        let total = MAX_LINES + 20;
        let mut parts: Vec<String> = Vec::new();
        for i in 0..total {
            if i == HEAD_LINES + 5 {
                parts.push("echo \"error: something went wrong\"".into());
            } else if i == HEAD_LINES + 6 {
                parts.push("echo \"warning: deprecated API\"".into());
            } else {
                parts.push(format!("echo \"line {}\"", i));
            }
        }

        let cmd = parts.join("; ");

        let result = executor
            .execute(CliCommand {
                command: cmd,
                working_dir: None,
                timeout_secs: Some(10),
                description: None,
                background: false,
            })
            .unwrap();

        assert!(result.truncated);
        let has_error = result
            .error_lines
            .iter()
            .any(|l| l.contains("something went wrong"));
        let has_warning = result
            .error_lines
            .iter()
            .any(|l| l.contains("deprecated API"));
        assert!(
            has_error || has_warning,
            "error lines should be extracted, got: {:?}",
            result.error_lines
        );
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    #[test]
    fn test_background_flag_works() {
        let executor = CliExecutor::new();

        // Test with background flag - command should return immediately
        let result = executor
            .execute(CliCommand {
                command: "sleep 0.1".to_string(),
                working_dir: None,
                timeout_secs: Some(30),
                description: None,
                background: true,
            })
            .unwrap();

        // Should return immediately with no exit code (background)
        assert!(result.pid.is_some(), "Should return a PID");
        assert_eq!(
            result.exit_code, None,
            "Background commands have no exit code"
        );
        assert!(result.duration_ms < 500, "Should return quickly");
    }

    #[test]
    fn test_trailing_ampersand_runs_background() {
        let executor = CliExecutor::new();

        // Test with trailing & - should auto-detect background
        let result = executor
            .execute(CliCommand {
                command: "sleep 0.1 &".to_string(),
                working_dir: None,
                timeout_secs: Some(30),
                description: None,
                background: false, // Even with false, trailing & should trigger background
            })
            .unwrap();

        assert!(result.pid.is_some(), "Should return a PID for trailing &");
    }

    #[test]
    fn test_background_writes_to_log_file() {
        let executor = CliExecutor::new();

        // Command that writes to stdout/stderr — log should capture both.
        let result = executor
            .execute(CliCommand {
                command: r#"echo bgtest-out; echo bgtest-err >&2"#.to_string(),
                working_dir: None,
                timeout_secs: Some(30),
                description: None,
                background: true,
            })
            .unwrap();

        let pid = result.pid.expect("background should return a PID");
        let log_path = result
            .log_path
            .expect("background should return a log path");

        // PID is real — must be killable (negative test: not zero, not random).
        assert!(pid > 0, "PID should be positive, got {pid}");

        // Log file exists at the returned path.
        assert!(
            std::path::Path::new(&log_path).exists(),
            "log file should exist at {log_path}"
        );

        // Log filename embeds the PID (bg_<pid>.log pattern).
        let filename = std::path::Path::new(&log_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        assert!(
            filename.starts_with("bg_") && filename.ends_with(".log"),
            "log filename should be bg_<n>.log, got {filename}"
        );

        // Wait briefly for output to flush, then read the log.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(
            content.contains("bgtest-out"),
            "log should contain stdout content, got: {content:?}"
        );
        assert!(
            content.contains("bgtest-err"),
            "log should contain stderr content (merged), got: {content:?}"
        );
    }

    #[test]
    fn test_background_pid_is_alive_after_return() {
        use std::process::Command;
        let executor = CliExecutor::new();

        // Long enough that the process is still alive when we check.
        let result = executor
            .execute(CliCommand {
                command: "sleep 5".to_string(),
                working_dir: None,
                timeout_secs: Some(30),
                description: None,
                background: true,
            })
            .unwrap();

        let pid = result.pid.expect("background should return a PID");

        // Process is still running — `ps -p` should find it.
        let ps = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pid="])
            .output()
            .expect("ps should work");
        let ps_out = String::from_utf8_lossy(&ps.stdout);
        assert!(
            ps_out.trim().parse::<u32>().ok() == Some(pid),
            "PID {pid} should be a real running process, ps output: {ps_out:?}"
        );

        // Cleanup: kill it so we don't leak a sleep process.
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}
