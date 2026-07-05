mod app;
mod entry_rows;
mod events;
mod hit_test;
mod render;
mod scroll;
mod state;
mod surface;
mod watcher;

use crate::{AppResult, config::Config};
use crossterm::{
    cursor::Show,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use journal_storage::JournalStore;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::path::PathBuf;
use std::{
    io::{self, Write},
    time::Duration,
};

use app::App;

pub fn run(config_path: PathBuf, config: Config, store: JournalStore) -> AppResult<()> {
    let app = App::new(config_path, config, store)?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal_guard = TerminalRestoreGuard::new();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(&mut terminal, app);
    let restore_result = restore_terminal(terminal.backend_mut());
    if restore_result.is_ok() {
        terminal_guard.disarm();
    }

    match result {
        Ok(()) => restore_result,
        Err(err) => Err(err),
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
    execute!(output, DisableMouseCapture, LeaveAlternateScreen, Show)?;
    Ok(())
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> AppResult<()> {
    let watcher = watcher::FileWatcher::start(&app.config.journal_root);

    terminal.draw(|frame| render::draw(frame, &mut app))?;

    loop {
        let poll_timeout = app
            .status_timeout()
            .map(|t| t.min(Duration::from_millis(200)))
            .unwrap_or(Duration::from_millis(200));

        let event = if event::poll(poll_timeout)? {
            Some(event::read()?)
        } else {
            None
        };

        let redraw = match event {
            Some(Event::Key(key)) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break;
                }
                if events::handle_key(terminal, &mut app, key)? {
                    break;
                }
                true
            }
            Some(Event::Mouse(mouse)) => {
                if events::handle_mouse(terminal, &mut app, mouse)? {
                    break;
                }
                true
            }
            Some(Event::Resize(_, _)) => true,
            Some(_) => false,
            None => app.expire_status(),
        };

        let watcher_changed = watcher.poll_change();

        if watcher_changed {
            app.refresh()?;
        }

        if redraw || watcher_changed {
            terminal.draw(|frame| render::draw(frame, &mut app))?;
        }
    }

    Ok(())
}
