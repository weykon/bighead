use big_head_loop::TickActionReport;

use super::render::LineStyle;

#[derive(Debug, Clone, Default)]
pub(super) struct RuntimeEventState {
    spark_count: usize,
    tick_count: usize,
    tick_error_count: usize,
    error_count: usize,
    checkpoint_count: usize,
    last_event: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeLine {
    pub(super) text: String,
    pub(super) style: LineStyle,
}

impl RuntimeEventState {
    pub(super) fn record_spark(&mut self, text: &str) -> RuntimeLine {
        self.spark_count += 1;
        let line = RuntimeLine {
            text: format!("Spark: {}", one_line(text)),
            style: LineStyle::Spark,
        };
        self.last_event = Some(line.text.clone());
        line
    }

    pub(super) fn record_error(&mut self, message: &str) -> RuntimeLine {
        self.error_count += 1;
        let line = RuntimeLine {
            text: format!("ERROR: {}", one_line(message)),
            style: LineStyle::Error,
        };
        self.last_event = Some(line.text.clone());
        line
    }

    pub(super) fn record_checkpoint(&mut self, path: &str, ok: bool) -> RuntimeLine {
        self.checkpoint_count += 1;
        let line = RuntimeLine {
            text: if ok {
                format!("Checkpoint saved: {path}")
            } else {
                format!("Checkpoint failed: {path}")
            },
            style: if ok {
                LineStyle::Success
            } else {
                LineStyle::Error
            },
        };
        self.last_event = Some(line.text.clone());
        line
    }

    pub(super) fn record_runtime(&mut self, text: &str) -> RuntimeLine {
        let line = RuntimeLine {
            text: one_line(text),
            style: LineStyle::Meta,
        };
        self.last_event = Some(line.text.clone());
        line
    }

    pub(super) fn record_tick_reports(&mut self, reports: &[TickActionReport]) -> Vec<RuntimeLine> {
        if reports.is_empty() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        for report in reports {
            self.tick_count += 1;
            let script = one_line(&report.script);
            lines.push(RuntimeLine {
                text: format!("TickAction {:?}: {}", report.entity, trim_to(&script, 120)),
                style: LineStyle::Tick,
            });
            lines.push(RuntimeLine {
                text: format!(
                    "  correlation={} queued={} sparks={}",
                    report.correlation_id,
                    report.queued_sparks,
                    report.spark_ids.join(",")
                ),
                style: LineStyle::Meta,
            });
            for log in &report.logs {
                lines.push(RuntimeLine {
                    text: format!("  tick > {}", one_line(log)),
                    style: LineStyle::Tick,
                });
            }
            if let Some(state) = &report.state {
                lines.push(RuntimeLine {
                    text: format!(
                        "  tick state={} urgent={} {}",
                        trim_to(&one_line(&state.state_key), 120),
                        state.urgent,
                        trim_to(&one_line(&state.summary), 240)
                    ),
                    style: if state.urgent {
                        LineStyle::Error
                    } else {
                        LineStyle::Meta
                    },
                });
            }
            if let Some(error) = &report.error {
                self.tick_error_count += 1;
                lines.push(RuntimeLine {
                    text: format!("  tick ERROR {}", one_line(error)),
                    style: LineStyle::Error,
                });
            }
        }
        if let Some(last) = lines.last() {
            self.last_event = Some(last.text.clone());
        }
        lines
    }

    pub(super) fn detail_lines(&self) -> Vec<RuntimeLine> {
        let mut lines = Vec::new();
        lines.push(RuntimeLine {
            text: format!(
                "Runtime: {} spark(s), {} tick(s), {} checkpoint(s)",
                self.spark_count, self.tick_count, self.checkpoint_count
            ),
            style: LineStyle::Meta,
        });
        if self.error_count > 0 || self.tick_error_count > 0 {
            lines.push(RuntimeLine {
                text: format!(
                    "Errors: {} runtime, {} tick",
                    self.error_count, self.tick_error_count
                ),
                style: LineStyle::Warning,
            });
        }
        if let Some(event) = &self.last_event {
            lines.push(RuntimeLine {
                text: format!("Last: {event}"),
                style: LineStyle::Meta,
            });
        }
        lines
    }
}

fn one_line(text: &str) -> String {
    text.replace('\n', " ").trim().to_string()
}

fn trim_to(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::entity::Entity;

    #[test]
    fn runtime_state_formats_spark_and_checkpoint_lines() {
        let mut state = RuntimeEventState::default();

        let spark = state.record_spark("hello\nworld");
        assert_eq!(spark.text, "Spark: hello world");
        assert_eq!(spark.style, LineStyle::Spark);

        let checkpoint = state.record_checkpoint("checkpoint.json", true);
        assert_eq!(checkpoint.text, "Checkpoint saved: checkpoint.json");
        assert_eq!(checkpoint.style, LineStyle::Success);

        let details = state.detail_lines();
        assert!(
            details
                .iter()
                .any(|line| line.text.contains("1 spark(s)") && line.text.contains("1 checkpoint"))
        );
    }

    #[test]
    fn runtime_state_formats_tick_reports_with_errors() {
        let mut state = RuntimeEventState::default();
        let lines = state.record_tick_reports(&[TickActionReport {
            entity: Entity::from_raw_u32(7).expect("test entity"),
            source_id: "test-source".into(),
            correlation_id: "test-run".into(),
            spark_ids: Vec::new(),
            script: "log(\"tick\")".into(),
            script_hash: "test-script-hash".into(),
            started_at_epoch: 1_788_488_559,
            duration_ms: 12,
            queued_sparks: 0,
            logs: vec!["ran\nok".into()],
            error: Some("boom".into()),
            state: None,
        }]);

        assert_eq!(lines[0].style, LineStyle::Tick);
        assert!(lines[0].text.contains("TickAction"));
        assert!(lines.iter().any(|line| line.text.contains("tick > ran ok")));
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("tick ERROR boom"))
        );
        assert!(
            state
                .detail_lines()
                .iter()
                .any(|line| line.text.contains("Errors: 0 runtime, 1 tick"))
        );
    }
}
