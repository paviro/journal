mod app;
mod entry_rows;
mod events;
mod hit_test;
mod render;
mod scroll;
mod state;

use crate::{AppResult, config::Config, crypto};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

use app::App;

pub fn run(config: Config, encryption_paths: crypto::EncryptionPaths) -> AppResult<()> {
    let app = App::new(config, encryption_paths)?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(&mut terminal, app);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> AppResult<()> {
    terminal.draw(|frame| render::draw(frame, &mut app))?;

    loop {
        let event = match app.status_timeout() {
            Some(timeout) => {
                if event::poll(timeout)? {
                    Some(event::read()?)
                } else {
                    None
                }
            }
            None => Some(event::read()?),
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

        if redraw {
            terminal.draw(|frame| render::draw(frame, &mut app))?;
        }
    }

    Ok(())
}
