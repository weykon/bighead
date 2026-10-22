use std::fs;
use std::io::Write;

use big_head_loop::TickActionReport;

/// 会话日志 —— 写入 `outputs/run_<timestamp>.log`，记录所有关键事件。
pub(crate) struct SessionLog {
    file: fs::File,
}

impl SessionLog {
    pub(crate) fn new() -> Self {
        fs::create_dir_all("outputs").ok();
        let ts = now_secs();
        let path = format!("outputs/run_{}.log", ts);
        let mut file = fs::File::create(&path).expect("Failed to create session log");
        writeln!(file, "# Session started at {}", ts).ok();
        Self { file }
    }

    pub(crate) fn spark(&mut self, text: &str) {
        writeln!(self.file, "{} SPARK {}", self.timestamp(), text).ok();
        self.file.flush().ok();
    }

    pub(crate) fn agent(&mut self, agent: &str, response: &str) {
        for line in response.lines() {
            writeln!(self.file, "{} AGENT [{}] {}", self.timestamp(), agent, line).ok();
        }
        self.file.flush().ok();
    }

    pub(crate) fn tick_action(&mut self, reports: &[TickActionReport]) {
        for r in reports {
            writeln!(
                self.file,
                "{} TICK {:?} script={}",
                self.timestamp(),
                r.entity,
                r.script
            )
            .ok();
            for log in &r.logs {
                writeln!(
                    self.file,
                    "{} TICK {:?} > {}",
                    self.timestamp(),
                    r.entity,
                    log
                )
                .ok();
            }
            if let Some(state) = &r.state {
                writeln!(
                    self.file,
                    "{} TICK {:?} STATE {}",
                    self.timestamp(),
                    r.entity,
                    serde_json::to_string(state).unwrap()
                )
                .ok();
            }
            if let Some(err) = &r.error {
                writeln!(
                    self.file,
                    "{} TICK {:?} ERROR {}",
                    self.timestamp(),
                    r.entity,
                    err
                )
                .ok();
            }
        }
        if !reports.is_empty() {
            self.file.flush().ok();
        }
    }

    pub(crate) fn error(&mut self, msg: &str) {
        writeln!(self.file, "{} ERROR {}", self.timestamp(), msg).ok();
        self.file.flush().ok();
    }

    pub(crate) fn checkpoint(&mut self, path: &str, ok: bool) {
        if ok {
            writeln!(
                self.file,
                "{} CHECKPOINT saved to {}",
                self.timestamp(),
                path
            )
            .ok();
        } else {
            writeln!(
                self.file,
                "{} CHECKPOINT failed to {}",
                self.timestamp(),
                path
            )
            .ok();
        }
        self.file.flush().ok();
    }

    pub(crate) fn close(&mut self) {
        writeln!(self.file, "# Session ended").ok();
        self.file.flush().ok();
    }

    fn timestamp(&self) -> String {
        format!("[{}]", now_secs())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
