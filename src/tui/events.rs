//! UI event reduction: convert runtime notifications into workspace state.
//!
//! Keeping this reducer separate from terminal input and drawing makes the
//! TUI easier to extend: new runtime events only need one state transition.

use super::activity::{ActivityLine, ActivityLineKind, format_activity};
use super::dirty::{DirtyAreas, dirty_if};
use super::render::{LineStyle, RenderLine, render_agent_markdown, sanitized_lines};
use super::runtime_events::RuntimeLine;
use super::workspace::WorkspaceTab;
use super::{TuiApp, UiEvent};

impl TuiApp {
    pub(super) fn drain_ui_events(&mut self, max_events: usize) -> DirtyAreas {
        let mut dirty = DirtyAreas::default();
        for _ in 0..max_events {
            let Ok(event) = self.ui_rx.try_recv() else {
                break;
            };
            match event {
                UiEvent::Log(line) => {
                    self.push_log(line);
                    dirty.include(DirtyAreas::runtime());
                }
                UiEvent::AgentResponse { agent, response } => {
                    self.push_agent_response(agent, response);
                    dirty.include(DirtyAreas::runtime());
                }
                UiEvent::Activity(activity) => {
                    let next_status = format_activity(&activity);
                    let status_changed = self.status != next_status;
                    self.status = next_status;
                    let line = self.activity.apply(activity);
                    let has_line = line.is_some();
                    if let Some(line) = line {
                        self.push_activity_line(line);
                    }
                    dirty.include(dirty_if(status_changed || has_line, DirtyAreas::runtime()));
                }
                UiEvent::Spark(text) => {
                    let line = self.runtime.record_spark(&text);
                    self.push_conversation_line(line.clone());
                    self.push_runtime_line(line);
                    dirty.include(DirtyAreas::runtime());
                }
                UiEvent::Runtime(text) => {
                    let line = self.runtime.record_runtime(&text);
                    self.push_runtime_line(line);
                    dirty.include(DirtyAreas::runtime());
                }
                UiEvent::Error(text) => {
                    let line = self.runtime.record_error(&text);
                    self.status = line.text.clone();
                    self.push_runtime_line(line);
                    dirty.include(DirtyAreas::runtime());
                }
                UiEvent::Checkpoint { path, ok } => {
                    let line = self.runtime.record_checkpoint(&path, ok);
                    self.status = line.text.clone();
                    self.push_runtime_line(line);
                    dirty.include(DirtyAreas::runtime());
                }
                UiEvent::TickActions(reports) => {
                    let lines = self.runtime.record_tick_reports(&reports);
                    if !lines.is_empty() {
                        self.workspace.push(
                            WorkspaceTab::TickActions,
                            lines.iter().map(|line| RenderLine {
                                text: line.text.clone(),
                                style: line.style,
                            }),
                        );
                        if let Some(summary) = lines.first() {
                            self.push_runtime_line(RuntimeLine {
                                text: summary.text.clone(),
                                style: summary.style,
                            });
                        }
                        dirty.include(DirtyAreas::runtime());
                    }
                }
                UiEvent::Processes(processes) => {
                    dirty.include(dirty_if(
                        self.processes.replace(processes),
                        DirtyAreas::runtime(),
                    ));
                }
                UiEvent::Questions(questions) => {
                    if self.questions.replace(questions) {
                        if !self.questions.items.is_empty() {
                            self.workspace.push(WorkspaceTab::Questions, [RenderLine { text: "有待回答的問題".into(), style: LineStyle::Meta }]);
                        }
                        if !self.questions.items.is_empty() && self.input.as_str().is_empty() {
                            self.workspace.switch_to(WorkspaceTab::Questions);
                        }
                        dirty.include(DirtyAreas::all());
                    }
                }
                UiEvent::Status(status) => {
                    if self.status != status {
                        self.status = status;
                        dirty.include(DirtyAreas::chrome());
                    }
                }
                UiEvent::Shutdown => self.should_quit = true,
            }
        }
        dirty
    }

    fn push_log(&mut self, text: String) {
        self.push_styled_log(WorkspaceTab::Conversation, text, LineStyle::Normal);
    }

    fn push_activity_line(&mut self, line: ActivityLine) {
        let style = match line.kind {
            ActivityLineKind::Normal | ActivityLineKind::Info => LineStyle::Meta,
            ActivityLineKind::Api => LineStyle::Api,
            ActivityLineKind::Token => LineStyle::Token,
            ActivityLineKind::ToolCall => LineStyle::ToolCall,
            ActivityLineKind::ToolResult => LineStyle::ToolResult,
            ActivityLineKind::Action => LineStyle::Action,
            ActivityLineKind::Search => LineStyle::Search,
            ActivityLineKind::Success => LineStyle::Success,
            ActivityLineKind::Warning => LineStyle::Warning,
            ActivityLineKind::Error => LineStyle::Error,
        };
        self.push_styled_log(WorkspaceTab::Activity, line.text, style);
    }

    fn push_runtime_line(&mut self, line: RuntimeLine) {
        self.push_styled_log(WorkspaceTab::Activity, line.text, line.style);
    }

    fn push_conversation_line(&mut self, line: RuntimeLine) {
        self.push_styled_log(WorkspaceTab::Conversation, line.text, line.style);
    }

    pub(super) fn push_styled_log(&mut self, tab: WorkspaceTab, text: String, style: LineStyle) {
        let lines = sanitized_lines(&text)
            .into_iter()
            .map(|text| RenderLine { text, style });
        self.workspace.push(tab, lines);
    }

    fn push_agent_response(&mut self, agent: String, response: String) {
        let lines = render_agent_markdown(&agent, &response);
        self.workspace.push(WorkspaceTab::Conversation, lines);
    }
}
