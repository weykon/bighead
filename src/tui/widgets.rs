use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};

use super::TuiApp;
use super::dirty::DirtyAreas;
use super::input::input_visual_lines;
use super::layout::{MIN_HEIGHT, MIN_WIDTH, Rect, TuiLayout};
use super::render::{
    display_len, fit, fit_padded, hline, join_lr, queue_style, visible_range, wrapped_lines,
};
use super::theme::queue_activity_style;
use super::workspace::{LineBuffer, WorkspaceTab};

const HEADER_BG: Color = Color::Rgb {
    r: 16,
    g: 24,
    b: 38,
};

impl TuiApp {
    pub(super) fn draw(&self, stdout: &mut io::Stdout, dirty: DirtyAreas) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        let Some(layout) = TuiLayout::new(width, height) else {
            queue!(
                stdout,
                MoveTo(0, 0),
                Clear(ClearType::All),
                Print(format!(
                    "Big Head TUI needs at least {MIN_WIDTH}x{MIN_HEIGHT} terminal cells."
                ))
            )?;
            return stdout.flush();
        };

        queue!(stdout, BeginSynchronizedUpdate, Hide)?;
        let draw_result = (|| -> io::Result<()> {
            if dirty.chrome {
                self.draw_header(stdout, layout.header)?;
            }
            if dirty.body {
                self.draw_workspace(stdout, layout.body, layout.wide)?;
            }
            if dirty.input {
                self.draw_input(stdout, layout.input)?;
            } else {
                self.queue_input_cursor(stdout, layout.input)?;
            }
            Ok(())
        })();
        queue!(stdout, EndSynchronizedUpdate)?;
        draw_result?;
        stdout.flush()
    }

    fn draw_header(&self, stdout: &mut io::Stdout, area: Rect) -> io::Result<()> {
        let status = if self.status.is_empty() {
            self.activity.status()
        } else {
            &self.status
        };
        let title = join_lr(
            " BIG HEAD / AGENT RUNTIME",
            &format!("● {status} "),
            area.width as usize,
        );
        queue!(
            stdout,
            MoveTo(area.x, area.y),
            SetBackgroundColor(HEADER_BG),
            SetForegroundColor(Color::Cyan),
            SetAttribute(Attribute::Bold),
            Print(title),
            SetAttribute(Attribute::Reset),
            MoveTo(area.x, area.y + 1),
            Print(" ")
        )?;
        self.draw_tabs(
            stdout,
            Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width.saturating_sub(2),
                height: 1,
            },
        )?;
        queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            ResetColor,
            MoveTo(area.x, area.y + 2),
            SetForegroundColor(Color::DarkGrey),
            Print(hline(area.width)),
            ResetColor
        )
    }

    fn draw_tabs(&self, stdout: &mut io::Stdout, area: Rect) -> io::Result<()> {
        let mut x = area.x;
        let end_x = area.x.saturating_add(area.width);
        let compact = area.width < 64;
        for tab in WorkspaceTab::ALL {
            if x >= end_x {
                break;
            }
            let unread = self.workspace.unread(tab);
            let badge = if unread == 0 {
                String::new()
            } else {
                format!(" +{unread}")
            };
            let title = if compact {
                tab.compact_title()
            } else {
                tab.title()
            };
            let label = format!(" {} {}{} ", tab.shortcut(), title, badge);
            let remaining = end_x.saturating_sub(x) as usize;
            let label = fit(&label, remaining);
            queue!(stdout, MoveTo(x, area.y), SetBackgroundColor(HEADER_BG))?;
            if tab == self.workspace.active() {
                queue!(
                    stdout,
                    SetAttribute(Attribute::Reverse),
                    SetAttribute(Attribute::Bold),
                    SetForegroundColor(Color::Cyan)
                )?;
            } else {
                queue!(
                    stdout,
                    SetAttribute(Attribute::Reset),
                    SetForegroundColor(Color::Grey)
                )?;
            }
            queue!(stdout, Print(&label), SetAttribute(Attribute::Reset))?;
            x = x.saturating_add(display_len(&label) as u16);
        }
        if x < end_x {
            queue!(
                stdout,
                MoveTo(x, area.y),
                SetBackgroundColor(HEADER_BG),
                Print(" ".repeat(end_x.saturating_sub(x) as usize))
            )?;
        }
        Ok(())
    }

    fn draw_workspace(&self, stdout: &mut io::Stdout, area: Rect, wide: bool) -> io::Result<()> {
        self.clear_area(stdout, area)?;
        let tab = self.workspace.active();
        let feed = self.workspace.active_feed();
        let mode = if feed.is_following() {
            "FOLLOW".to_string()
        } else {
            format!("PAUSED +{}", feed.scroll_offset())
        };
        let heading = join_lr(
            &format!(" {}", tab.title().to_uppercase()),
            &format!("{} lines · {mode} ", feed.len()),
            area.width as usize,
        );
        queue!(
            stdout,
            MoveTo(area.x, area.y),
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::White),
            Print(heading),
            SetAttribute(Attribute::Reset),
            MoveTo(area.x, area.y + 1),
            SetForegroundColor(Color::DarkGrey),
            Print(hline(area.width)),
            ResetColor
        )?;

        let content = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(2),
        };
        if tab == WorkspaceTab::Questions {
            let area = content.inset(1, 0);
            let lines = wrapped_lines(&self.questions.lines(), area.width as usize);
            let selected = lines.iter().position(|line| line.style == super::render::LineStyle::Success).unwrap_or(0);
            let start = if selected >= area.height as usize { selected.saturating_sub(area.height as usize / 2) } else { 0 };
            for (row, line) in lines.iter().skip(start).take(area.height as usize).enumerate() {
                queue!(stdout, MoveTo(area.x, area.y + row as u16))?;
                queue_style(stdout, line.style)?;
                queue!(stdout, Print(fit_padded(&line.text, area.width as usize)))?;
            }
            queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)
        } else if tab == WorkspaceTab::Processes {
            self.draw_processes(stdout, content.inset(1, 0))
        } else if tab == WorkspaceTab::Activity && wide {
            self.draw_activity_workspace(stdout, content, feed)
        } else {
            self.draw_feed(stdout, content.inset(1, 0), feed)
        }
    }

    fn draw_processes(&self, stdout: &mut io::Stdout, area: Rect) -> io::Result<()> {
        if area.width == 0 || area.height == 0 {
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let header = "STATE       NAME                 PID   PORT  AGE     PURPOSE";
        queue!(
            stdout,
            MoveTo(area.x, area.y),
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Cyan),
            Print(fit(header, area.width as usize)),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        for (row, process) in (1..area.height).zip(self.processes.rows(now)) {
            let y = area.y + row;
            queue!(
                stdout,
                MoveTo(area.x, y),
                SetAttribute(Attribute::Reset),
                ResetColor
            )?;
            {
                let line = format!(
                    "{:<11} {:<20} {:>5} {:>5} {:<7} {}",
                    process.state,
                    process.name,
                    process.pid,
                    process.port,
                    process.age,
                    process.purpose
                );
                let color = match process.state.as_str() {
                    "Running" | "Healthy" => Color::Green,
                    "Starting" => Color::Yellow,
                    "Exited" | "Failed" | "Stopped" => Color::Red,
                    _ => Color::DarkGrey,
                };
                queue!(
                    stdout,
                    SetForegroundColor(color),
                    Print(fit(&line, area.width as usize))
                )?;
            }
        }
        if self.processes.is_empty() && area.height > 1 {
            let y = area.y + 1;
            queue!(
                stdout,
                MoveTo(area.x, y),
                SetForegroundColor(Color::DarkGrey)
            )?;
            queue!(
                stdout,
                Print(fit("No managed processes", area.width as usize))
            )?;
        }
        queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)
    }

    fn draw_activity_workspace(
        &self,
        stdout: &mut io::Stdout,
        area: Rect,
        feed: &LineBuffer,
    ) -> io::Result<()> {
        const SUMMARY_HEIGHT: u16 = 5;
        let summary = Rect {
            height: SUMMARY_HEIGHT.min(area.height),
            ..area
        }
        .inset(1, 0);
        let Some((agent_area, runtime_area)) = summary.split_columns(52, 3) else {
            return self.draw_feed(stdout, area.inset(1, 0), feed);
        };

        self.draw_activity_summary(stdout, agent_area, runtime_area)?;
        if area.height <= SUMMARY_HEIGHT {
            return Ok(());
        }
        queue!(
            stdout,
            MoveTo(area.x, area.y + SUMMARY_HEIGHT - 1),
            SetForegroundColor(Color::DarkGrey),
            Print(hline(area.width)),
            ResetColor
        )?;
        self.draw_feed(
            stdout,
            Rect {
                x: area.x + 1,
                y: area.y + SUMMARY_HEIGHT,
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(SUMMARY_HEIGHT),
            },
            feed,
        )
    }

    fn draw_activity_summary(
        &self,
        stdout: &mut io::Stdout,
        agent_area: Rect,
        runtime_area: Rect,
    ) -> io::Result<()> {
        queue!(
            stdout,
            MoveTo(agent_area.x, agent_area.y),
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Cyan),
            Print("AGENT"),
            MoveTo(runtime_area.x, runtime_area.y),
            Print("RUNTIME"),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;

        let agent_lines = self.activity.detail_lines();
        let runtime_lines = self.runtime.detail_lines();
        let rows = agent_area.height.saturating_sub(1) as usize;
        for row in 0..rows {
            let y = agent_area.y + 1 + row as u16;
            if let Some(line) = agent_lines.get(row) {
                queue!(stdout, MoveTo(agent_area.x, y))?;
                queue_activity_style(stdout, line.kind)?;
                queue!(stdout, Print(fit(&line.text, agent_area.width as usize)))?;
            }
            if let Some(line) = runtime_lines.get(row) {
                queue!(stdout, MoveTo(runtime_area.x, y))?;
                queue_style(stdout, line.style)?;
                queue!(stdout, Print(fit(&line.text, runtime_area.width as usize)))?;
            }
        }
        queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)
    }

    fn draw_feed(&self, stdout: &mut io::Stdout, area: Rect, feed: &LineBuffer) -> io::Result<()> {
        if area.width == 0 || area.height == 0 {
            return Ok(());
        }
        let lines = wrapped_lines(feed.lines(), area.width as usize);
        let height = area.height as usize;
        let scroll_offset = feed.scroll_offset().min(lines.len().saturating_sub(height));
        let (start, end) = visible_range(lines.len(), height, scroll_offset);
        for row in 0..height {
            let y = area.y + row as u16;
            queue!(
                stdout,
                MoveTo(area.x, y),
                SetAttribute(Attribute::Reset),
                ResetColor
            )?;
            if let Some(line) = lines[start..end].get(row) {
                queue_style(stdout, line.style)?;
                queue!(stdout, Print(fit_padded(&line.text, area.width as usize)))?;
            } else if feed.lines().is_empty() && row == 0 {
                queue!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(fit_padded("No events yet", area.width as usize))
                )?;
            } else {
                queue!(stdout, Print(" ".repeat(area.width as usize)))?;
            }
        }
        queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)
    }

    fn draw_input(&self, stdout: &mut io::Stdout, area: Rect) -> io::Result<()> {
        let border = section_rule(
            "SPARK",
            &format!("{} chars", self.input.as_str().chars().count()),
            area.width as usize,
        );
        queue!(
            stdout,
            MoveTo(area.x, area.y),
            SetForegroundColor(Color::DarkGrey),
            Print(border),
            ResetColor
        )?;

        let input_rows = area.height.saturating_sub(2).max(1) as usize;
        let prompt_width = 3usize;
        let edit_width = area.width.saturating_sub(prompt_width as u16).max(1) as usize;
        let visual = input_visual_lines(self.input.as_str(), self.input.cursor(), edit_width);
        let cursor_line = visual
            .iter()
            .position(|line| line.cursor_x.is_some())
            .unwrap_or(0);
        let start = cursor_line.saturating_sub(input_rows.saturating_sub(1));
        let end = (start + input_rows).min(visual.len());
        let visible = &visual[start..end];
        let cursor = visual[cursor_line].cursor_x.map(|x| {
            (
                area.x + (prompt_width + x).min(area.width.saturating_sub(1) as usize) as u16,
                area.y + 1 + cursor_line.saturating_sub(start) as u16,
            )
        });

        for row in 0..input_rows {
            let y = area.y + 1 + row as u16;
            let prefix = if start + row == 0 { "›  " } else { "   " };
            let text = visible.get(row).map_or("", |line| line.text.as_str());
            queue!(
                stdout,
                MoveTo(area.x, y),
                SetForegroundColor(if row == 0 {
                    Color::Cyan
                } else {
                    Color::DarkGrey
                }),
                Print(prefix),
                ResetColor,
                Print(fit_padded(text, edit_width))
            )?;
        }

        let (left_hint, right_hint) = if area.width < 64 {
            (" Enter send", "Ctrl+[]\\ views ")
        } else {
            (
                " Enter send · Ctrl+J newline",
                "Ctrl+[]\\ views · PgUp/PgDn ",
            )
        };
        let hint = join_lr(left_hint, right_hint, area.width as usize);
        queue!(
            stdout,
            MoveTo(area.x, area.y + area.height - 1),
            SetForegroundColor(Color::DarkGrey),
            Print(hint),
            ResetColor
        )?;

        if let Some((x, y)) = cursor {
            queue!(stdout, MoveTo(x, y), Show)
        } else {
            queue!(
                stdout,
                MoveTo(area.x + prompt_width as u16, area.y + 1),
                Show
            )
        }
    }

    fn queue_input_cursor(&self, stdout: &mut io::Stdout, area: Rect) -> io::Result<()> {
        let input_rows = area.height.saturating_sub(2).max(1) as usize;
        let prompt_width = 3usize;
        let edit_width = area.width.saturating_sub(prompt_width as u16).max(1) as usize;
        let visual = input_visual_lines(self.input.as_str(), self.input.cursor(), edit_width);
        let cursor_line = visual
            .iter()
            .position(|line| line.cursor_x.is_some())
            .unwrap_or(0);
        let start = cursor_line.saturating_sub(input_rows.saturating_sub(1));
        let cursor = visual[cursor_line].cursor_x.map(|x| {
            (
                area.x + (prompt_width + x).min(area.width.saturating_sub(1) as usize) as u16,
                area.y + 1 + cursor_line.saturating_sub(start) as u16,
            )
        });
        if let Some((x, y)) = cursor {
            queue!(stdout, MoveTo(x, y), Show)
        } else {
            queue!(
                stdout,
                MoveTo(area.x + prompt_width as u16, area.y + 1),
                Show
            )
        }
    }

    fn clear_area(&self, stdout: &mut io::Stdout, area: Rect) -> io::Result<()> {
        for row in 0..area.height {
            queue!(
                stdout,
                MoveTo(area.x, area.y + row),
                SetAttribute(Attribute::Reset),
                ResetColor,
                Print(" ".repeat(area.width as usize))
            )?;
        }
        Ok(())
    }
}

fn section_rule(label: &str, right: &str, width: usize) -> String {
    let left = format!("─ {label} ");
    let right = format!(" {right} ─");
    let occupied = display_len(&left).saturating_add(display_len(&right));
    if occupied >= width {
        return fit(&format!("{left}{right}"), width);
    }
    format!("{left}{}{right}", "─".repeat(width - occupied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_rule_has_stable_width() {
        let rule = section_rule("SPARK", "18 chars", 48);
        assert_eq!(display_len(&rule), 48);
        assert!(rule.contains("SPARK"));
        assert!(rule.contains("18 chars"));
    }
}
