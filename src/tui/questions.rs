use super::dirty::DirtyAreas;
use super::render::{LineStyle, RenderLine};
use super::{InputEvent, KeyCode, KeyEvent, KeyModifiers, TuiApp};
use big_head_loop::UserQuestion;

#[derive(Default)]
pub(super) struct QuestionState {
    pub(super) items: Vec<UserQuestion>,
    pub(super) selected: usize,
    pub(super) submitted: Option<String>,
}

impl QuestionState {
    pub(super) fn replace(&mut self, items: Vec<UserQuestion>) -> bool {
        if self.items == items {
            return false;
        }
        if self.items.first().map(|q| &q.id) != items.first().map(|q| &q.id) {
            self.selected = 0;
            self.submitted = None;
        }
        self.items = items;
        true
    }

    pub(super) fn lines(&self) -> Vec<RenderLine> {
        let Some(q) = self.items.first() else {
            return vec![RenderLine {
                text: "暫時沒有待選問題".into(),
                style: LineStyle::Meta,
            }];
        };
        let mut lines = vec![RenderLine {
            text: q.question.clone(),
            style: LineStyle::Normal,
        }];
        for (i, option) in q.options.iter().enumerate() {
            lines.push(RenderLine {
                text: format!(
                    "{} {}. {option}",
                    if i == self.selected { ">" } else { " " },
                    i + 1
                ),
                style: if i == self.selected {
                    LineStyle::Success
                } else {
                    LineStyle::Normal
                },
            });
        }
        lines.push(RenderLine {
            text: if self.submitted.is_some() {
                "答案已提交，等待接收。"
            } else {
                "↑↓ 選擇 · Enter 確認 · 亦可在輸入欄寫自己的答案 · Ctrl+D 取消此問題"
            }
            .into(),
            style: LineStyle::Meta,
        });
        lines
            .into_iter()
            .flat_map(|line| {
                super::render::sanitized_lines(&line.text)
                    .into_iter()
                    .map(move |text| RenderLine {
                        text,
                        style: line.style,
                    })
            })
            .collect()
    }
}

impl TuiApp {
    pub(super) fn handle_question_key(&mut self, key: KeyEvent) -> Option<DirtyAreas> {
        let q = self.questions.items.first()?;
        if self.questions.submitted.is_some() && matches!(key.code, KeyCode::Enter) {
            return Some(DirtyAreas::default());
        }
        let answer = match key.code {
            KeyCode::Up if self.input.as_str().is_empty() => {
                self.questions.selected = self.questions.selected.saturating_sub(1);
                return Some(DirtyAreas::runtime());
            }
            KeyCode::Down if self.input.as_str().is_empty() => {
                self.questions.selected =
                    (self.questions.selected + 1).min(q.options.len().saturating_sub(1));
                return Some(DirtyAreas::runtime());
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => None,
            KeyCode::Enter
                if !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) =>
            {
                Some(if self.input.as_str().trim().is_empty() {
                    q.options.get(self.questions.selected)?.clone()
                } else {
                    self.input.as_str().trim().to_string()
                })
            }
            _ => return None,
        };
        if answer.as_ref().is_some_and(|s| s.chars().count() > 4000) {
            self.status = "答案過長，請縮短至 4000 字以內".into();
            return Some(DirtyAreas::all());
        }
        if self
            .input_tx
            .send(InputEvent::QuestionAnswer {
                id: q.id.clone(),
                answer,
            })
            .is_ok()
        {
            self.questions.submitted = Some(q.id.clone());
            self.input.clear();
        }
        Some(DirtyAreas::all())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{UiEvent, workspace::WorkspaceTab};
    use std::sync::mpsc;

    #[test]
    fn keyboard_choice_free_text_and_cancel_are_correlated_not_ordinary_sparks() {
        let (ui_tx, ui_rx) = mpsc::channel();
        let (input_tx, input_rx) = mpsc::channel();
        let mut app = TuiApp::new(
            ui_rx,
            input_tx,
            crate::app_input::new_clipboard_image_store(),
            vec![],
        );
        let question = UserQuestion {
            id: "q1".into(),
            tx_id: "tx1".into(),
            question: "Choose".into(),
            options: vec!["One".into(), "Two".into()],
            context: "test".into(),
            status: "pending".into(),
            answer: None,
            created_at: 0,
        };
        ui_tx
            .send(UiEvent::Questions(vec![question.clone()]))
            .unwrap();
        app.drain_ui_events(10);
        assert_eq!(app.workspace.active(), WorkspaceTab::Questions);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(input_rx.try_recv().unwrap(), InputEvent::QuestionAnswer { id, answer: Some(text) } if id == "q1" && text == "Two")
        );
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(input_rx.try_recv().is_err());
        app.questions.replace(vec![]);
        app.questions.replace(vec![UserQuestion {
            id: "q2".into(),
            ..question.clone()
        }]);
        app.input.replace("My own answer");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(input_rx.try_recv().unwrap(), InputEvent::QuestionAnswer { answer: Some(text), .. } if text == "My own answer")
        );
        app.questions.replace(vec![UserQuestion {
            id: "q3".into(),
            ..question
        }]);
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(
            matches!(input_rx.try_recv().unwrap(), InputEvent::QuestionAnswer { id, answer: None } if id == "q3")
        );
    }
}
