use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use big_head_loop::{ManagedProcess, TickActionReport};
use crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal;

use crate::InputEvent;
use crate::app_input::{
    ClipboardImageStore, expand_image_placeholders, paste_clipboard_image_placeholder,
};
use big_head_state::{AgentActivity, StateHook};
use tokio::sync::broadcast;

mod activity;
mod dirty;
mod events;
mod input;
mod layout;
mod processes;
mod questions;
mod render;
mod runtime_events;
mod terminal_screen;
mod theme;
mod widgets;
mod workspace;

use activity::ActivityState;
use dirty::{DirtyAreas, dirty_if, should_render};
use input::{HistoryRing, InputEditor};
use layout::TuiLayout;
use processes::ProcessTable;
use render::LineStyle;
use runtime_events::RuntimeEventState;
use terminal_screen::{TerminalSession, restore_terminal};
use workspace::{WorkspaceState, WorkspaceTab};

const MAX_LOG_LINES: usize = 1_000;
const MAX_UI_EVENTS_PER_TICK: usize = 64;
const IDLE_EVENT_POLL: Duration = Duration::from_millis(16);
const DIRTY_EVENT_POLL: Duration = Duration::from_millis(0);

pub(crate) struct TuiHandle {
    tx: mpsc::Sender<UiEvent>,
    join: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone)]
enum UiEvent {
    Log(String),
    AgentResponse { agent: String, response: String },
    Activity(AgentActivity),
    Spark(String),
    Runtime(String),
    Error(String),
    Checkpoint { path: String, ok: bool },
    TickActions(Vec<TickActionReport>),
    Processes(Vec<ManagedProcess>),
    Questions(Vec<big_head_loop::UserQuestion>),
    Status(String),
    Shutdown,
}

pub(crate) fn spawn(
    hook: StateHook,
    input_tx: mpsc::Sender<InputEvent>,
    attachments: ClipboardImageStore,
    history: Vec<String>,
) -> TuiHandle {
    let (ui_tx, ui_rx) = mpsc::channel();
    tokio::spawn({
        let ui_tx = ui_tx.clone();
        async move {
            let mut rx = hook.subscribe();
            loop {
                match rx.recv().await {
                    Ok(activity) => {
                        if ui_tx.send(UiEvent::Activity(activity)).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let event = UiEvent::Log(format!("[state] skipped {n} activities"));
                        if ui_tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    });

    let join = thread::spawn(move || {
        let mut app = TuiApp::new(ui_rx, input_tx, attachments, history);
        if let Err(e) = app.run() {
            eprintln!("tui error: {e}");
        }
    });

    TuiHandle {
        tx: ui_tx.clone(),
        join: Some(join),
    }
}

impl TuiHandle {
    pub(crate) fn questions(&self, questions: Vec<big_head_loop::UserQuestion>) {
        let _ = self.tx.send(UiEvent::Questions(questions));
    }
    pub(crate) fn log(&self, text: impl Into<String>) {
        let _ = self.tx.send(UiEvent::Log(text.into()));
    }

    pub(crate) fn runtime(&self, text: impl Into<String>) {
        let _ = self.tx.send(UiEvent::Runtime(text.into()));
    }

    pub(crate) fn spark(&self, text: impl Into<String>) {
        let _ = self.tx.send(UiEvent::Spark(text.into()));
    }

    pub(crate) fn error(&self, text: impl Into<String>) {
        let _ = self.tx.send(UiEvent::Error(text.into()));
    }

    pub(crate) fn checkpoint(&self, path: impl Into<String>, ok: bool) {
        let _ = self.tx.send(UiEvent::Checkpoint {
            path: path.into(),
            ok,
        });
    }

    pub(crate) fn tick_actions(&self, reports: Vec<TickActionReport>) {
        let _ = self.tx.send(UiEvent::TickActions(reports));
    }

    pub(crate) fn processes(&self, processes: Vec<ManagedProcess>) {
        let _ = self.tx.send(UiEvent::Processes(processes));
    }

    pub(crate) fn status(&self, text: impl Into<String>) {
        let _ = self.tx.send(UiEvent::Status(text.into()));
    }

    pub(crate) fn agent(&self, agent: impl Into<String>, response: impl Into<String>) {
        let _ = self.tx.send(UiEvent::AgentResponse {
            agent: agent.into(),
            response: response.into(),
        });
    }

    pub(crate) fn shutdown(mut self) {
        let _ = self.tx.send(UiEvent::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct TuiApp {
    ui_rx: mpsc::Receiver<UiEvent>,
    input_tx: mpsc::Sender<InputEvent>,
    attachments: ClipboardImageStore,
    workspace: WorkspaceState,
    status: String,
    activity: ActivityState,
    runtime: RuntimeEventState,
    processes: ProcessTable,
    questions: questions::QuestionState,
    input: InputEditor,
    history: HistoryRing,
    should_quit: bool,
    graceful_shutdown_requested: bool,
}

impl TuiApp {
    fn new(
        ui_rx: mpsc::Receiver<UiEvent>,
        input_tx: mpsc::Sender<InputEvent>,
        attachments: ClipboardImageStore,
        history: Vec<String>,
    ) -> Self {
        Self {
            ui_rx,
            input_tx,
            attachments,
            workspace: WorkspaceState::new(MAX_LOG_LINES),
            status: "Idle".into(),
            activity: ActivityState::default(),
            runtime: RuntimeEventState::default(),
            processes: ProcessTable::default(),
            questions: questions::QuestionState::default(),
            input: InputEditor::default(),
            history: HistoryRing::new(history),
            should_quit: false,
            graceful_shutdown_requested: false,
        }
    }

    fn run(&mut self) -> io::Result<()> {
        let mut terminal = TerminalSession::enter()?;
        self.event_loop(terminal.stdout_mut())
    }

    fn event_loop(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        let mut dirty = DirtyAreas::all();
        let mut last_draw: Option<Instant> = None;
        while !self.should_quit {
            dirty.include(self.drain_ui_events(MAX_UI_EVENTS_PER_TICK));

            let poll_timeout = if dirty.input {
                DIRTY_EVENT_POLL
            } else {
                IDLE_EVENT_POLL
            };

            if event::poll(poll_timeout)? {
                match event::read()? {
                    TermEvent::Key(key) => dirty.include(self.handle_key(key)),
                    TermEvent::Paste(text) => {
                        self.input.insert_str(&normalize_paste_text(&text));
                        self.history.reset();
                        dirty.include(DirtyAreas::input());
                    }
                    TermEvent::Mouse(mouse) => dirty.include(self.handle_mouse(mouse)),
                    TermEvent::Resize(_, _) => dirty.include(DirtyAreas::all()),
                    _ => {}
                }
            }

            if should_render(dirty, last_draw) {
                self.draw(stdout, dirty)?;
                last_draw = Some(Instant::now());
                dirty = DirtyAreas::default();
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> DirtyAreas {
        if key.kind == KeyEventKind::Release {
            return DirtyAreas::default();
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.graceful_shutdown_requested {
                restore_terminal();
                eprintln!("^C  Hard-quit immediately; checkpoint will not be saved.");
                std::process::exit(130);
            }
            self.graceful_shutdown_requested = true;
            self.status =
                "Cancellation requested; saving at next safe point. Ctrl+C again hard-quits."
                    .into();
            self.push_styled_log(
                WorkspaceTab::Activity,
                "^C Cancellation requested. Saving checkpoint and exiting at the next safe point. Press Ctrl+C again to hard-quit without saving."
                    .into(),
                LineStyle::Meta,
            );
            let _ = self.input_tx.send(InputEvent::Shutdown);
            return DirtyAreas::runtime();
        }

        if paste_key(key) {
            let placeholder = paste_clipboard_image_placeholder(&self.attachments);
            self.input.insert_str(&placeholder);
            return DirtyAreas::input();
        }

        if let Some(tab) = workspace_shortcut(key) {
            return self.switch_workspace(tab);
        }

        if self.workspace.active() == WorkspaceTab::Questions {
            if let Some(dirty) = self.handle_question_key(key) {
                return dirty;
            }
        }

        match key.code {
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.workspace.cycle_backward();
                DirtyAreas::all()
            }
            KeyCode::Tab => {
                self.workspace.cycle_forward();
                DirtyAreas::all()
            }
            KeyCode::BackTab => {
                self.workspace.cycle_backward();
                DirtyAreas::all()
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                DirtyAreas::all()
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.move_to_start();
                DirtyAreas::input()
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.move_to_end();
                DirtyAreas::input()
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.delete_before_cursor();
                self.history.reset();
                DirtyAreas::input()
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.delete_after_cursor();
                self.history.reset();
                DirtyAreas::input()
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.insert_char('\n');
                self.history.reset();
                DirtyAreas::input()
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.input.insert_char(c);
                self.history.reset();
                DirtyAreas::input()
            }
            KeyCode::Backspace => {
                self.history.reset();
                dirty_if(self.input.backspace(), DirtyAreas::input())
            }
            KeyCode::Delete => {
                self.history.reset();
                dirty_if(self.input.delete(), DirtyAreas::input())
            }
            KeyCode::Left => {
                self.input.move_left();
                DirtyAreas::input()
            }
            KeyCode::Right => {
                self.input.move_right();
                DirtyAreas::input()
            }
            KeyCode::Up => {
                if self.should_use_history_previous() {
                    if let Some(text) = self.history.previous(self.input.as_str()) {
                        self.input.replace(text);
                    }
                } else {
                    self.input.move_up();
                }
                DirtyAreas::input()
            }
            KeyCode::Down => {
                if self.should_use_history_next() {
                    if let Some(text) = self.history.next() {
                        self.input.replace(text);
                    }
                } else {
                    self.input.move_down();
                }
                DirtyAreas::input()
            }
            KeyCode::PageUp => {
                self.scroll_up(10);
                DirtyAreas::runtime()
            }
            KeyCode::PageDown => {
                self.scroll_down(10);
                DirtyAreas::runtime()
            }
            KeyCode::Home => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.scroll_up(MAX_LOG_LINES);
                    DirtyAreas::runtime()
                } else {
                    self.input.move_to_line_start();
                    DirtyAreas::input()
                }
            }
            KeyCode::End => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.scroll_to_bottom();
                    DirtyAreas::runtime()
                } else {
                    self.input.move_to_line_end();
                    DirtyAreas::input()
                }
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    self.input.insert_char('\n');
                    self.history.reset();
                    return DirtyAreas::input();
                }
                let expanded = expand_image_placeholders(self.input.as_str(), &self.attachments);
                if self.input_tx.send(InputEvent::Line(expanded)).is_ok() {
                    self.history.remember(self.input.as_str());
                    self.input.clear();
                    DirtyAreas::input()
                } else {
                    DirtyAreas::default()
                }
            }
            // Most terminals encode Ctrl+[ as the same ESC byte.
            KeyCode::Esc => self.switch_workspace(WorkspaceTab::Conversation),
            _ => DirtyAreas::default(),
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> DirtyAreas {
        if !self.mouse_in_body(mouse.row) {
            return DirtyAreas::default();
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_up(3);
                DirtyAreas::runtime()
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down(3);
                DirtyAreas::runtime()
            }
            _ => DirtyAreas::default(),
        }
    }

    fn mouse_in_body(&self, row: u16) -> bool {
        let Ok((width, height)) = terminal::size() else {
            return false;
        };
        TuiLayout::new(width, height).is_some_and(|layout| layout.body.contains_row(row))
    }

    fn scroll_up(&mut self, lines: usize) {
        self.workspace.scroll_up(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.workspace.scroll_down(lines);
    }

    fn scroll_to_bottom(&mut self) {
        self.workspace.follow();
    }

    fn switch_workspace(&mut self, tab: WorkspaceTab) -> DirtyAreas {
        dirty_if(self.workspace.switch_to(tab), DirtyAreas::all())
    }

    fn should_use_history_previous(&self) -> bool {
        self.input.logical_line_count() <= 1 || self.input.is_at_first_line()
    }

    fn should_use_history_next(&self) -> bool {
        self.input.logical_line_count() <= 1 || self.input.is_at_last_line()
    }
}

fn paste_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V'))
        && (key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT))
}

fn workspace_shortcut(key: KeyEvent) -> Option<WorkspaceTab> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char('[') => Some(WorkspaceTab::Conversation),
        // Legacy terminal control encoding aliases Ctrl+] to Ctrl+5 and
        // Ctrl+\ to Ctrl+4 before Crossterm sees the key event.
        KeyCode::Char(']') | KeyCode::Char('5') => Some(WorkspaceTab::Activity),
        KeyCode::Char('\\') | KeyCode::Char('4') => Some(WorkspaceTab::TickActions),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(WorkspaceTab::Processes),
        KeyCode::Char('?') => Some(WorkspaceTab::Questions),
        KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(WorkspaceTab::Questions)
        }
        _ => None,
    }
}

fn normalize_paste_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_input::new_clipboard_image_store;

    fn test_app() -> (TuiApp, mpsc::Sender<UiEvent>) {
        let (ui_tx, ui_rx) = mpsc::channel();
        let (input_tx, _input_rx) = mpsc::channel();
        (
            TuiApp::new(ui_rx, input_tx, new_clipboard_image_store(), Vec::new()),
            ui_tx,
        )
    }

    #[test]
    fn paste_key_accepts_ctrl_or_alt_v() {
        assert!(paste_key(KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL
        )));
        assert!(paste_key(KeyEvent::new(
            KeyCode::Char('V'),
            KeyModifiers::ALT
        )));
        assert!(!paste_key(KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn normalize_paste_text_keeps_multiline_content_in_input() {
        assert_eq!(
            normalize_paste_text("first\r\nsecond\rother"),
            "first\nsecond\nother"
        );
    }

    #[test]
    fn control_brackets_select_workspaces_without_function_keys() {
        assert_eq!(
            workspace_shortcut(KeyEvent::new(KeyCode::Char('['), KeyModifiers::CONTROL)),
            Some(WorkspaceTab::Conversation)
        );
        assert_eq!(
            workspace_shortcut(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL)),
            Some(WorkspaceTab::Activity)
        );
        assert_eq!(
            workspace_shortcut(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
            Some(WorkspaceTab::TickActions)
        );
        assert_eq!(
            workspace_shortcut(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL)),
            Some(WorkspaceTab::Activity)
        );
        assert_eq!(
            workspace_shortcut(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::CONTROL)),
            Some(WorkspaceTab::TickActions)
        );
        assert_eq!(
            workspace_shortcut(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn unbound_control_keys_are_not_inserted_into_the_spark() {
        let (mut app, _tx) = test_app();

        let dirty = app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));

        assert!(!dirty.any());
        assert_eq!(app.input.as_str(), "");
    }

    #[test]
    fn questions_shortcut_preserves_typing_and_releases_ctrl_q() {
        for key in [
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::CONTROL),
            KeyEvent::new(
                KeyCode::Char('?'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            KeyEvent::new(
                KeyCode::Char('/'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ] {
            assert_eq!(workspace_shortcut(key), Some(WorkspaceTab::Questions));
        }
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            KeyEvent::new(
                KeyCode::Char('Q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        ] {
            assert_eq!(workspace_shortcut(key), None);
        }
    }

    #[test]
    fn empty_tick_batches_do_not_dirty_the_terminal() {
        let (mut app, tx) = test_app();
        tx.send(UiEvent::TickActions(Vec::new())).unwrap();

        assert!(!app.drain_ui_events(1).any());
        assert_eq!(app.workspace.feed(WorkspaceTab::TickActions).len(), 0);
    }

    #[test]
    fn repeated_status_values_do_not_trigger_redraws() {
        let (mut app, tx) = test_app();
        tx.send(UiEvent::Status("Idle".into())).unwrap();

        assert!(!app.drain_ui_events(1).any());
    }
}
