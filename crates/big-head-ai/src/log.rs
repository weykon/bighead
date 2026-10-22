//! 结构化日志基础设施，AI 层和运行时层共享。
//!
//! [`LogEmitter`] 类型是 `Arc<dyn Fn(LogEntry) + Send + Sync>` 回调，
//! 由消费者（如 TUI 的 channel bridge）安装。没有 emitter 时，
//! [`emit`] 辅助函数回退到 `eprintln!`，这样所有示例无需修改即可运行。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

mod timestamp_serde {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let secs = t
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64();
        s.serialize_f64(secs)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let secs = f64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_secs_f64(secs))
    }
}

/// 日志条目的严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Agent 推理过程中产生的单条结构化日志。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// 标识生产者：`"coordinator"`、`"rhai:my-system"`、`"tool"` 等。
    pub source: String,
    pub level: LogLevel,
    pub message: String,
    #[serde(with = "timestamp_serde")]
    pub timestamp: std::time::SystemTime,
}

/// 线程安全、可廉价克隆的日志槽。
pub type LogEmitter = Arc<dyn Fn(LogEntry) + Send + Sync>;

static QUIET: AtomicBool = AtomicBool::new(false);

/// 抑制 `emit` 的 `eprintln!` 回退。当存在外部的终端渲染器
///（如 `StateHook` + `TerminalRenderer`）时，调用此方法避免重复输出。
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// 通过 `emitter` 发送日志条目（若存在），否则回退到 `eprintln!`。
pub fn emit(emitter: &Option<LogEmitter>, source: &str, level: LogLevel, message: String) {
    let entry = LogEntry {
        source: source.to_string(),
        level,
        message,
        timestamp: std::time::SystemTime::now(),
    };
    match emitter {
        Some(e) => e(entry),
        None if QUIET.load(Ordering::Relaxed) => {} // state hook 已接管终端输出
        None => eprintln!("[{}] {}", entry.source, entry.message),
    }
}
