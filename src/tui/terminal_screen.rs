use std::io::{self, Stdout};

use crossterm::cursor::{MoveTo, SetCursorStyle, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

pub(super) struct TerminalSession {
    stdout: Stdout,
    restored: bool,
}

impl TerminalSession {
    pub(super) fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(
            stdout,
            EnterAlternateScreen,
            // Distinguish Ctrl+? from Backspace in terminals supporting CSI-u.
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            EnableMouseCapture,
            EnableBracketedPaste,
            SetCursorStyle::SteadyBar,
            Show,
            Clear(ClearType::Purge),
            Clear(ClearType::All)
        )?;

        Ok(Self {
            stdout,
            restored: false,
        })
    }

    pub(super) fn stdout_mut(&mut self) -> &mut Stdout {
        &mut self.stdout
    }

    pub(super) fn restore(&mut self) {
        if !self.restored {
            restore_terminal();
            self.restored = true;
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(super) fn restore_terminal() {
    let _ = execute!(
        io::stdout(),
        Show,
        SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste,
        DisableMouseCapture,
        PopKeyboardEnhancementFlags,
        // Clear only the alternate buffer before leaving it. Clearing after
        // LeaveAlternateScreen wipes the user's normal terminal scrollback and
        // can look like a blank/frozen terminal on quit.
        Clear(ClearType::All),
        MoveTo(0, 0),
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}
