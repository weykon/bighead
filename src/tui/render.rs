use std::io;

use crossterm::queue;
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineStyle {
    Normal,
    Meta,
    Heading,
    Agent,
    AgentText,
    Api,
    Token,
    ToolCall,
    ToolResult,
    Action,
    Search,
    Code,
    Quote,
    Spark,
    Tick,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderLine {
    pub(super) text: String,
    pub(super) style: LineStyle,
}

pub(super) fn queue_style(stdout: &mut io::Stdout, style: LineStyle) -> io::Result<()> {
    match style {
        LineStyle::Normal => queue!(stdout, SetAttribute(Attribute::Reset), ResetColor),
        LineStyle::Meta => queue!(
            stdout,
            SetAttribute(Attribute::Dim),
            SetForegroundColor(Color::DarkGrey)
        ),
        LineStyle::Heading => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Cyan)
        ),
        LineStyle::Agent => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::White)
        ),
        LineStyle::AgentText => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Grey)
        ),
        LineStyle::Api => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkCyan)
        ),
        LineStyle::Token => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkYellow)
        ),
        LineStyle::ToolCall => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Magenta)
        ),
        LineStyle::ToolResult => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Green)
        ),
        LineStyle::Action => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Cyan)
        ),
        LineStyle::Search => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkMagenta)
        ),
        LineStyle::Code => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Yellow)
        ),
        LineStyle::Quote => queue!(
            stdout,
            SetAttribute(Attribute::Italic),
            SetForegroundColor(Color::Rgb {
                r: 74,
                g: 85,
                b: 104
            })
        ),
        LineStyle::Spark => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Blue)
        ),
        LineStyle::Tick => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Magenta)
        ),
        LineStyle::Success => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Green)
        ),
        LineStyle::Warning => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Yellow)
        ),
        LineStyle::Error => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Red)
        ),
    }
}

pub(super) fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let char_width = c.width().unwrap_or(0);
        if used + char_width > width {
            break;
        }
        out.push(c);
        used += char_width;
    }
    out
}

pub(super) fn fit_padded(text: &str, width: usize) -> String {
    let mut out = fit(text, width);
    let used = display_len(&out);
    if used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out
}

pub(super) fn hline(width: u16) -> String {
    "─".repeat(width as usize)
}

pub(super) fn join_lr(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if right.is_empty() {
        return fit_padded(left, width);
    }

    let right = fit(right, width / 2);
    let right_width = display_len(&right);
    let left_width = width.saturating_sub(right_width.saturating_add(1));
    format!("{} {}", fit_padded(left, left_width), right)
}

pub(super) fn wrapped_lines(lines: &[RenderLine], width: usize) -> Vec<RenderLine> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in lines {
        let mut current = String::new();
        let mut current_width = 0usize;
        for c in line.text.chars() {
            let char_width = c.width().unwrap_or(0);
            if current_width > 0 && current_width + char_width > width {
                wrapped.push(RenderLine {
                    text: current,
                    style: line.style,
                });
                current = String::new();
                current_width = 0;
            }
            current.push(c);
            current_width += char_width;
        }
        wrapped.push(RenderLine {
            text: current,
            style: line.style,
        });
    }
    wrapped
}

pub(super) fn visible_range(
    total_lines: usize,
    height: usize,
    scroll_offset: usize,
) -> (usize, usize) {
    if height == 0 || total_lines == 0 {
        return (0, 0);
    }
    let max_scroll = total_lines.saturating_sub(height);
    let scroll = scroll_offset.min(max_scroll);
    let end = total_lines.saturating_sub(scroll);
    let start = end.saturating_sub(height);
    (start, end)
}

pub(super) fn display_len(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(super) fn sanitized_lines(text: &str) -> Vec<String> {
    let normalized = text.replace('\t', "    ").replace('\r', "");
    let mut lines = Vec::new();
    for line in normalized.lines() {
        let clean = line.chars().filter(|c| !c.is_control()).collect::<String>();
        lines.push(clean);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(super) fn render_agent_markdown(agent: &str, response: &str) -> Vec<RenderLine> {
    let mut lines = Vec::new();
    lines.push(RenderLine {
        text: String::new(),
        style: LineStyle::Normal,
    });
    lines.push(RenderLine {
        text: format!("[{agent}]"),
        style: LineStyle::Agent,
    });

    let mut in_code = false;
    for raw in sanitized_lines(response) {
        let trimmed = raw.trim();

        if trimmed.starts_with("```") {
            in_code = !in_code;
            let label = trimmed.trim_start_matches("```").trim();
            lines.push(RenderLine {
                text: if in_code {
                    if label.is_empty() {
                        "----- code -----".into()
                    } else {
                        format!("----- code: {label} -----")
                    }
                } else {
                    "----- end code -----".into()
                },
                style: LineStyle::Code,
            });
            continue;
        }

        if in_code {
            lines.push(RenderLine {
                text: format!("  {raw}"),
                style: LineStyle::Code,
            });
            continue;
        }

        let (text, style) = markdown_line_to_tui(&raw);
        lines.push(RenderLine { text, style });
    }

    lines
}

fn markdown_line_to_tui(raw: &str) -> (String, LineStyle) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), LineStyle::AgentText);
    }

    if let Some(stripped) = trimmed.strip_prefix('#') {
        let heading = stripped.trim_start_matches('#').trim();
        return (strip_inline_markers(heading), LineStyle::Heading);
    }

    if let Some(quote) = trimmed.strip_prefix('>') {
        return (
            format!("| {}", strip_inline_markers(quote.trim())),
            LineStyle::Quote,
        );
    }

    if is_markdown_table_separator(trimmed) {
        return (String::new(), LineStyle::Meta);
    }

    if trimmed.starts_with('|') && trimmed.ends_with('|') {
        return (strip_inline_markers(trimmed), LineStyle::AgentText);
    }

    let normalized = normalize_list_marker(trimmed);
    (strip_inline_markers(&normalized), LineStyle::AgentText)
}

fn is_markdown_table_separator(line: &str) -> bool {
    line.contains('|')
        && line
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn normalize_list_marker(line: &str) -> String {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return format!("  - {rest}");
        }
    }

    let Some((number, rest)) = line.split_once(". ") else {
        return line.to_string();
    };
    if number.chars().all(|c| c.is_ascii_digit()) {
        format!("  {number}. {rest}")
    } else {
        line.to_string()
    }
}

fn strip_inline_markers(text: &str) -> String {
    text.replace("***", "")
        .replace("**", "")
        .replace("__", "")
        .replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_preserves_utf8_boundaries() {
        assert_eq!(fit("abc", 2), "ab");
        assert_eq!(fit("图片abc", 4), "图片");
    }

    #[test]
    fn fit_padded_overwrites_remaining_cells() {
        assert_eq!(display_len(&fit_padded("abc", 5)), 5);
        assert_eq!(display_len(&fit_padded("图片", 5)), 5);
        assert_eq!(fit_padded("abcdef", 3), "abc");
    }

    #[test]
    fn join_lr_keeps_both_edges_within_the_requested_width() {
        let joined = join_lr("BIG HEAD", "Wave 12: reasoning", 32);
        assert_eq!(display_len(&joined), 32);
        assert!(joined.starts_with("BIG HEAD"));
        assert!(joined.contains("Wave 12"));
    }

    #[test]
    fn sanitized_lines_split_embedded_newlines() {
        assert_eq!(
            sanitized_lines("history: 第一行\n第二行\tX\r\n第三行"),
            vec![
                "history: 第一行".to_string(),
                "第二行    X".to_string(),
                "第三行".to_string()
            ]
        );
    }

    #[test]
    fn wrapped_lines_never_emit_newline_controls() {
        let lines = sanitized_lines("a\nb")
            .into_iter()
            .map(|text| RenderLine {
                text,
                style: LineStyle::Normal,
            })
            .collect::<Vec<_>>();
        let wrapped = wrapped_lines(&lines, 10);
        assert_eq!(
            wrapped,
            vec![
                RenderLine {
                    text: "a".to_string(),
                    style: LineStyle::Normal
                },
                RenderLine {
                    text: "b".to_string(),
                    style: LineStyle::Normal
                }
            ]
        );
    }

    #[test]
    fn visible_range_tracks_bottom_by_default() {
        assert_eq!(visible_range(100, 20, 0), (80, 100));
    }

    #[test]
    fn visible_range_scrolls_up_and_clamps() {
        assert_eq!(visible_range(100, 20, 10), (70, 90));
        assert_eq!(visible_range(100, 20, 500), (0, 20));
    }

    #[test]
    fn render_agent_markdown_marks_headings_and_code() {
        let lines = render_agent_markdown("coordinator", "# Title\n\n```rust\nlet x = 1;\n```");
        assert!(
            lines
                .iter()
                .any(|line| line.text == "[coordinator]" && line.style == LineStyle::Agent)
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text == "Title" && line.style == LineStyle::Heading)
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("let x = 1;") && line.style == LineStyle::Code)
        );
    }

    #[test]
    fn markdown_line_strips_inline_markers() {
        let (text, style) = markdown_line_to_tui("- **done** with `code`");
        assert_eq!(style, LineStyle::AgentText);
        assert_eq!(text, "  - done with code");
    }
}
