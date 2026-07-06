mod app;
mod entry_rows;
mod events;
mod hit_test;
mod image;
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
    time::{Duration, Instant},
};

/// Blink half-period for the search caret.
const CURSOR_BLINK: Duration = Duration::from_millis(530);

/// Quiet period after the last search keystroke before the (whole-corpus) hit
/// recompute runs, so fast typing doesn't re-scan every entry per key.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);

/// Quiet period after the last watched file change before reloading the store,
/// so a burst of writes (e.g. a Day One import) collapses into one reload.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(400);

use app::App;

pub fn run(config_path: PathBuf, config: Config, store: JournalStore) -> AppResult<()> {
    let mut app = App::new(config_path, config, store)?;
    enable_raw_mode()?;
    // Must run after raw mode: the detection query reads control-sequence
    // replies from stdin.
    app.image.runtime = image::ImageRuntime::detect(&app.store);
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
    let mut overlay_was_visible = app.has_overlay();
    let mut last_blink = Instant::now();
    // When set, a watched file change is pending; reload once the filesystem has
    // been quiet until this deadline (coalesces import-time write storms). The
    // accumulated changed paths let the reload touch only affected entries.
    let mut pending_refresh_at: Option<Instant> = None;
    let mut pending_paths: Vec<PathBuf> = Vec::new();

    loop {
        // A newly finished image build makes the frame stale; repaint below.
        let images_ready = app.image.runtime.poll_results();

        let mut poll_timeout = app
            .status_timeout()
            .map(|t| t.min(Duration::from_millis(200)))
            .unwrap_or(Duration::from_millis(200));
        // Poll briefly while builds are pending so results paint promptly.
        if app.image.runtime.has_pending() {
            poll_timeout = poll_timeout.min(Duration::from_millis(30));
        }
        // Wake often enough to blink the search caret while typing in the field.
        if app.is_search_input_active() {
            poll_timeout = poll_timeout.min(CURSOR_BLINK);
        }
        // Wake to run a debounced search recompute once typing pauses.
        if app.search_dirty {
            let remaining = app
                .search_last_edit
                .map(|edited| SEARCH_DEBOUNCE.saturating_sub(edited.elapsed()))
                .unwrap_or_default();
            poll_timeout = poll_timeout.min(remaining);
        }
        // Wake to run a debounced store reload once file changes settle.
        if let Some(at) = pending_refresh_at {
            poll_timeout = poll_timeout.min(at.saturating_duration_since(Instant::now()));
        }

        let event = if event::poll(poll_timeout)? {
            Some(event::read()?)
        } else {
            None
        };
        let is_key_event = matches!(&event, Some(Event::Key(_)));

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

        // Debounce watcher-driven reloads: each change pushes the deadline out and
        // accumulates the changed paths; the reload runs only once no change has
        // arrived for the quiet period, then re-reads just those entries.
        let changed = watcher.poll_changes();
        if !changed.is_empty() {
            pending_paths.extend(changed);
            pending_refresh_at = Some(Instant::now() + REFRESH_DEBOUNCE);
        }
        let refreshed = if pending_refresh_at.is_some_and(|at| Instant::now() >= at) {
            pending_refresh_at = None;
            let paths = std::mem::take(&mut pending_paths);
            app.refresh_paths(&paths)?;
            true
        } else {
            false
        };

        // Run the debounced search recompute once typing has paused.
        let search_recomputed = if app.search_dirty
            && app
                .search_last_edit
                .is_none_or(|edited| edited.elapsed() >= SEARCH_DEBOUNCE)
        {
            app.update_search_results();
            true
        } else {
            false
        };

        // Warm the viewer's images once it's open and drop them when the entry
        // closes; cheap when nothing changed, rebuilds on change.
        app.sync_image_warm(terminal.size()?);

        // An overlay marks the underlying image cells as `skip`, so ratatui's
        // diff won't re-emit them; force a full repaint when it closes to redraw
        // the image and wipe the overlay residue.
        let overlay_visible = app.has_overlay();
        let overlay_closed = overlay_was_visible && !overlay_visible;
        overlay_was_visible = overlay_visible;

        // Repaint each tick while the viewer's image builds so the loading
        // ellipsis keeps animating.
        let animate_loading = app.image_viewer_state().is_some() && app.image.runtime.has_pending();

        // Drive the search caret's blink: keystrokes hold it solid, idle toggles
        // it on the blink half-period, and outside the field it stays visible.
        let mut blink_toggled = false;
        if app.is_search_input_active() {
            if is_key_event {
                last_blink = Instant::now();
                if !app.search_cursor_visible {
                    app.search_cursor_visible = true;
                    blink_toggled = true;
                }
            } else if last_blink.elapsed() >= CURSOR_BLINK {
                last_blink = Instant::now();
                app.search_cursor_visible = !app.search_cursor_visible;
                blink_toggled = true;
            }
        } else if !app.search_cursor_visible {
            app.search_cursor_visible = true;
        }

        if redraw
            || refreshed
            || search_recomputed
            || images_ready
            || animate_loading
            || blink_toggled
        {
            if overlay_closed && app.image.runtime.uses_graphics() {
                terminal.clear()?;
            }
            terminal.draw(|frame| render::draw(frame, &mut app))?;
        }
    }

    // Remember the selected journal for the next session. All break paths fall
    // through here, so this covers every exit (including Ctrl-C).
    let selected = app.selected_journal().map(|journal| journal.name.clone());
    if app.config.last_journal != selected {
        app.config.last_journal = selected;
        crate::config::save_config(&app.config_path, &app.config)?;
    }

    Ok(())
}
