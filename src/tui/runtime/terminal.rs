use crate::{AppResult, tui::runtime::CrosstermBackend};
use crossterm::{
    cursor::{SetCursorStyle, Show},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::Terminal;
use std::io::{self, Write};

pub(super) fn with_terminal(
    inner: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<()>,
) -> AppResult<()> {
    enable_raw_mode()?;
    let mut terminal_guard = TerminalRestoreGuard::new();
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        // Deliver a paste as one `Event::Paste` block instead of a keystroke storm,
        // so the editor inserts it at once and per-keystroke hooks don't fire.
        EnableBracketedPaste,
        SetCursorStyle::BlinkingBar
    )?;
    // Opt into the kitty keyboard protocol where the terminal supports it: without
    // it crossterm can't report the Super (Cmd) modifier at all, so Cmd+C/X/V would
    // be invisible. Disambiguation also makes modified keys explicit. Terminals that
    // don't support it (e.g. macOS Terminal.app) simply keep legacy reporting.
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = inner(&mut terminal);
    let restore_result = restore_terminal(terminal.backend_mut());
    if restore_result.is_ok() {
        terminal_guard.disarm();
    }

    match result {
        Ok(()) => restore_result,
        Err(error) => Err(error),
    }
}

struct TerminalRestoreGuard {
    active: bool,
}

impl TerminalRestoreGuard {
    fn new() -> Self {
        Self { active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = restore_terminal(&mut io::stdout());
        }
    }
}

fn restore_terminal(output: &mut impl Write) -> AppResult<()> {
    disable_raw_mode()?;
    // Pop the enhancement flags first; the terminal ignores the pop if nothing was
    // pushed, so it's safe to send unconditionally from the drop-guard path too.
    let _ = execute!(output, PopKeyboardEnhancementFlags);
    execute!(
        output,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        SetCursorStyle::DefaultUserShape,
        Show
    )?;
    Ok(())
}
