use big_head_loop::ManagedProcess;

/// TUI-facing process model. It owns ordering and presentation-friendly
/// values so widgets do not need to know ECS serialization details.
#[derive(Debug, Default, Clone)]
pub(super) struct ProcessTable {
    entries: Vec<ManagedProcess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProcessRow {
    pub(super) state: String,
    pub(super) name: String,
    pub(super) pid: String,
    pub(super) port: String,
    pub(super) age: String,
    pub(super) purpose: String,
}

impl ProcessTable {
    pub(super) fn replace(&mut self, mut entries: Vec<ManagedProcess>) -> bool {
        entries.sort_by_key(|process| (std::cmp::Reverse(process.started_at), process.pid));
        if self.entries == entries {
            return false;
        }
        self.entries = entries;
        true
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn rows(&self, now: u64) -> impl Iterator<Item = ProcessRow> + '_ {
        self.entries.iter().map(move |process| ProcessRow {
            state: process.state.clone(),
            name: process.name.clone(),
            pid: process.pid.to_string(),
            port: process
                .port
                .map(|port| port.to_string())
                .unwrap_or_else(|| "-".into()),
            age: format_age(now.saturating_sub(process.started_at)),
            purpose: process.purpose.clone(),
        })
    }
}

fn format_age(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        _ => format!("{}h", seconds / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(name: &str, started_at: u64) -> ManagedProcess {
        ManagedProcess::new(
            name.into(),
            name.into(),
            42,
            "dev-server".into(),
            "/tmp".into(),
            "test".into(),
            String::new(),
            String::new(),
            None,
            started_at,
        )
    }

    #[test]
    fn replace_sorts_newest_first_and_skips_identical_snapshots() {
        let mut table = ProcessTable::default();
        assert!(table.replace(vec![process("old", 1), process("new", 2)]));
        assert!(!table.replace(vec![process("old", 1), process("new", 2)]));
        let rows: Vec<_> = table.rows(62).collect();
        assert_eq!(rows[0].name, "new");
        assert_eq!(rows[0].age, "1m");
    }
}
