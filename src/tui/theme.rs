use std::io;

use crossterm::queue;
use crossterm::style::{Attribute, Color, SetAttribute, SetForegroundColor};

use super::activity::ActivityLineKind;

pub(super) fn queue_activity_style(
    stdout: &mut io::Stdout,
    kind: ActivityLineKind,
) -> io::Result<()> {
    match kind {
        ActivityLineKind::Normal => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkGrey)
        ),
        ActivityLineKind::Info => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Cyan)
        ),
        ActivityLineKind::Api => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkCyan)
        ),
        ActivityLineKind::Token => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkYellow)
        ),
        ActivityLineKind::ToolCall => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Magenta)
        ),
        ActivityLineKind::ToolResult => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Green)
        ),
        ActivityLineKind::Action => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::DarkBlue)
        ),
        ActivityLineKind::Search => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::DarkMagenta)
        ),
        ActivityLineKind::Success => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Green)
        ),
        ActivityLineKind::Warning => queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Yellow)
        ),
        ActivityLineKind::Error => queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Red)
        ),
    }
}
