use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone)]
pub(super) struct HistoryRing {
    entries: Vec<String>,
    cursor: Option<usize>,
    draft: String,
}

impl HistoryRing {
    pub(super) fn new(entries: Vec<String>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .filter(|entry| !entry.trim().is_empty())
                .collect(),
            cursor: None,
            draft: String::new(),
        }
    }

    pub(super) fn remember(&mut self, entry: &str) {
        let entry = entry.trim();
        if entry.is_empty() {
            self.reset();
            return;
        }
        if self.entries.last().is_none_or(|last| last != entry) {
            self.entries.push(entry.to_string());
        }
        self.reset();
    }

    pub(super) fn previous(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.cursor {
            Some(0) => 0,
            Some(index) => index.saturating_sub(1),
            None => {
                self.draft = current.to_string();
                self.entries.len().saturating_sub(1)
            }
        };
        self.cursor = Some(next);
        self.entries.get(next).cloned()
    }

    pub(super) fn next(&mut self) -> Option<String> {
        let cursor = self.cursor?;
        if cursor + 1 >= self.entries.len() {
            let draft = self.draft.clone();
            self.reset();
            return Some(draft);
        }
        let next = cursor + 1;
        self.cursor = Some(next);
        self.entries.get(next).cloned()
    }

    pub(super) fn reset(&mut self) {
        self.cursor = None;
        self.draft.clear();
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct InputEditor {
    text: String,
    cursor: usize,
    desired_col: Option<usize>,
}

impl InputEditor {
    pub(super) fn as_str(&self) -> &str {
        &self.text
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.desired_col = None;
    }

    pub(super) fn replace(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.char_len();
        self.desired_col = None;
    }

    pub(super) fn logical_line_count(&self) -> usize {
        logical_lines(self.as_str()).len()
    }

    pub(super) fn is_at_first_line(&self) -> bool {
        logical_cursor_position(self.as_str(), self.cursor).0 == 0
    }

    pub(super) fn is_at_last_line(&self) -> bool {
        let lines = logical_lines(self.as_str());
        logical_cursor_position(self.as_str(), self.cursor).0 + 1 >= lines.len()
    }

    pub(super) fn insert_char(&mut self, c: char) {
        if self.cursor >= self.char_len() {
            self.text.push(c);
        } else {
            let byte = self.byte_index(self.cursor);
            self.text.insert(byte, c);
        }
        self.cursor += 1;
        self.desired_col = None;
    }

    pub(super) fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.cursor >= self.char_len() {
            self.text.push_str(text);
            self.cursor += text.chars().count();
            self.desired_col = None;
            return;
        }
        for c in text.chars() {
            self.insert_char(c);
        }
    }

    pub(super) fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let start = self.byte_index(self.cursor - 1);
        let end = self.byte_index(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
        self.desired_col = None;
        true
    }

    pub(super) fn delete(&mut self) -> bool {
        if self.cursor >= self.char_len() {
            return false;
        }
        let start = self.byte_index(self.cursor);
        let end = self.byte_index(self.cursor + 1);
        self.text.replace_range(start..end, "");
        self.desired_col = None;
        true
    }

    pub(super) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.desired_col = None;
    }

    pub(super) fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_len());
        self.desired_col = None;
    }

    pub(super) fn move_to_start(&mut self) {
        self.cursor = 0;
        self.desired_col = None;
    }

    pub(super) fn move_to_end(&mut self) {
        self.cursor = self.char_len();
        self.desired_col = None;
    }

    pub(super) fn move_to_line_start(&mut self) {
        let (start, _) = self.current_line_bounds();
        self.cursor = start;
        self.desired_col = None;
    }

    pub(super) fn move_to_line_end(&mut self) {
        let (_, end) = self.current_line_bounds();
        self.cursor = end;
        self.desired_col = None;
    }

    pub(super) fn delete_before_cursor(&mut self) {
        let byte = self.byte_index(self.cursor);
        self.text.replace_range(..byte, "");
        self.cursor = 0;
        self.desired_col = None;
    }

    pub(super) fn delete_after_cursor(&mut self) {
        let byte = self.byte_index(self.cursor);
        self.text.replace_range(byte.., "");
        self.desired_col = None;
    }

    pub(super) fn move_up(&mut self) {
        self.move_vertical(-1);
    }

    pub(super) fn move_down(&mut self) {
        self.move_vertical(1);
    }

    fn move_vertical(&mut self, direction: isize) {
        let lines = logical_lines(self.as_str());
        let (line_idx, col) = logical_cursor_position(self.as_str(), self.cursor);
        let target_line = if direction < 0 {
            line_idx.saturating_sub(1)
        } else {
            (line_idx + 1).min(lines.len().saturating_sub(1))
        };
        let desired = self.desired_col.unwrap_or(col);
        self.cursor = logical_index_for_position(&lines, target_line, desired, self.char_len());
        self.desired_col = Some(desired);
    }

    fn current_line_bounds(&self) -> (usize, usize) {
        let chars = self.text.chars().collect::<Vec<_>>();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1] != '\n' {
            start -= 1;
        }
        let mut end = self.cursor;
        while end < chars.len() && chars[end] != '\n' {
            end += 1;
        }
        (start, end)
    }

    fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_index(&self, char_index: usize) -> usize {
        if char_index == 0 {
            return 0;
        }
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(idx, _)| idx)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InputVisualLine {
    pub(super) text: String,
    pub(super) cursor_x: Option<usize>,
}

pub(super) fn input_visual_lines(text: &str, cursor: usize, width: usize) -> Vec<InputVisualLine> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut cursor_x = None;
    let mut cursor_seen = false;
    let mut char_index = 0usize;

    for c in text.chars() {
        if cursor == char_index && !cursor_seen {
            cursor_x = Some(current_width);
            cursor_seen = true;
        }

        if c == '\n' {
            lines.push(InputVisualLine {
                text: current,
                cursor_x,
            });
            current = String::new();
            current_width = 0;
            cursor_x = None;
            char_index += 1;
            continue;
        }

        let char_width = c.width().unwrap_or(0);
        if current_width + char_width > width && !current.is_empty() {
            lines.push(InputVisualLine {
                text: current,
                cursor_x,
            });
            current = String::new();
            current_width = 0;
            cursor_x = if cursor == char_index && !cursor_seen {
                cursor_seen = true;
                Some(0)
            } else {
                None
            };
        }
        current.push(c);
        current_width += char_width;
        char_index += 1;
    }

    if cursor == char_index && !cursor_seen {
        cursor_x = Some(current_width);
    }

    lines.push(InputVisualLine {
        text: current,
        cursor_x,
    });
    lines
}

fn logical_lines(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, c) in text.chars().enumerate() {
        if c == '\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn logical_cursor_position(text: &str, cursor: usize) -> (usize, usize) {
    let lines = logical_lines(text);
    let line_idx = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, start)| **start <= cursor)
        .map_or(0, |(idx, _)| idx);
    (line_idx, cursor.saturating_sub(lines[line_idx]))
}

fn logical_index_for_position(
    lines: &[usize],
    line_idx: usize,
    desired_col: usize,
    char_len: usize,
) -> usize {
    let start = lines.get(line_idx).copied().unwrap_or(0);
    let next_start = lines.get(line_idx + 1).copied();
    let end = next_start.map_or(char_len, |next| next.saturating_sub(1));
    start.saturating_add(desired_col).min(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_editor_inserts_and_deletes_at_cursor_without_breaking_utf8() {
        let mut input = InputEditor::default();
        input.insert_str("你好ab");
        input.move_left();
        input.move_left();
        input.insert_char('，');

        assert_eq!(input.as_str(), "你好，ab");
        assert_eq!(input.cursor(), 3);

        assert!(input.backspace());
        assert_eq!(input.as_str(), "你好ab");
        assert_eq!(input.cursor(), 2);

        assert!(input.delete());
        assert_eq!(input.as_str(), "你好b");
    }

    #[test]
    fn input_editor_moves_within_multiline_text() {
        let mut input = InputEditor::default();
        input.insert_str("abc\nxy\n12345");
        input.move_to_start();
        input.move_right();
        input.move_right();
        input.move_down();
        assert_eq!(input.cursor(), 6);
        input.move_down();
        assert_eq!(input.cursor(), 9);
        input.move_up();
        assert_eq!(input.cursor(), 6);
    }

    #[test]
    fn input_visual_lines_wrap_and_track_wide_cursor() {
        let lines = input_visual_lines("图片abc", 2, 4);

        assert_eq!(
            lines,
            vec![
                InputVisualLine {
                    text: "图片".to_string(),
                    cursor_x: Some(4)
                },
                InputVisualLine {
                    text: "abc".to_string(),
                    cursor_x: None
                }
            ]
        );
    }

    #[test]
    fn input_visual_lines_preserve_explicit_newline_cursor() {
        let lines = input_visual_lines("hello\n世界", 6, 20);

        assert_eq!(
            lines,
            vec![
                InputVisualLine {
                    text: "hello".to_string(),
                    cursor_x: None
                },
                InputVisualLine {
                    text: "世界".to_string(),
                    cursor_x: Some(0)
                }
            ]
        );
    }

    #[test]
    fn history_ring_walks_entries_and_restores_draft() {
        let mut history = HistoryRing::new(vec!["one".into(), "two".into()]);

        assert_eq!(history.previous("draft"), Some("two".into()));
        assert_eq!(history.previous("two"), Some("one".into()));
        assert_eq!(history.previous("one"), Some("one".into()));
        assert_eq!(history.next(), Some("two".into()));
        assert_eq!(history.next(), Some("draft".into()));
        assert_eq!(history.next(), None);
    }

    #[test]
    fn history_ring_deduplicates_adjacent_entries() {
        let mut history = HistoryRing::new(Vec::new());
        history.remember("same");
        history.remember("same");

        assert_eq!(history.previous(""), Some("same".into()));
        assert_eq!(history.next(), Some(String::new()));
    }
}
