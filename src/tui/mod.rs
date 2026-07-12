mod app;
mod editor_state;
mod entry_rows;
mod environment;
mod events;
mod geocode;
mod hit_test;
mod image;
mod render;
mod scroll;
mod search;
mod state;
mod surface;
mod syntax_highlight;
#[cfg(test)]
mod test_support;
mod text_input;
pub(crate) mod theme;
mod watcher;
mod worker;

use crate::{AppResult, config::Config};
use crossterm::{
    cursor::{SetCursorStyle, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use notema_encryption::SecretString;
use notema_storage::{JournalStore, StoreAccess};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use zeroize::Zeroize;

/// Quiet period after the last search keystroke before the (whole-corpus) hit
/// recompute runs, so fast typing doesn't re-scan every entry per key.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);

/// Quiet period after the last watched file change before reloading the store,
/// so a burst of writes (e.g. a Day One import) collapses into one reload.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(400);

use app::App;
use text_input::PassphraseInput;

pub(crate) fn run(config_path: PathBuf, config: Config, store: JournalStore) -> AppResult<()> {
    // Ensure the store exists before probing for a lock so identity checks
    // reflect on-disk state.
    store.ensure()?;
    // Before raw mode / the alternate screen: auto dark/light detection talks
    // OSC to the normal screen, and load warnings should print readably.
    theme::init_from_config(&config_path, &config.ui);
    with_terminal(|terminal| run_after_unlock(terminal, config_path, config, store))
}

/// Launch straight into a fullscreen new-entry editor and quit once the entry is
/// saved or discarded — the `notema log` no-body path. Never prompts for a
/// passphrase: a passphrase-less identity is unlocked silently so the metadata
/// dialogs can suggest recently-used people/tags from existing entries, but an
/// identity that *needs* a passphrase is left locked (writing a new entry needs
/// only the recipients roster, so it works either way; the store's other entries
/// simply stay locked placeholders behind the editor).
pub(crate) fn run_compose(
    config_path: PathBuf,
    config: Config,
    mut store: JournalStore,
    journal: String,
    metadata: notema_domain::Metadata,
) -> AppResult<()> {
    store.ensure()?;
    theme::init_from_config(&config_path, &config.ui);
    if store.unlock_available() && !store.identity_needs_passphrase()? {
        store.unlock(None)?;
    }
    with_terminal(|terminal| {
        let mut app = App::new(config_path, config, store)?;
        app.begin_compose(journal, metadata);
        run_loop(terminal, app)
    })
}

/// Set up the terminal (raw mode + alternate screen + mouse capture), run `inner`,
/// then restore the terminal — disarming the panic-restore guard only on a clean
/// restore. Shared by [`run`] and [`run_compose`].
fn with_terminal(
    inner: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<()>,
) -> AppResult<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetCursorStyle::BlinkingBar
    )?;
    let mut terminal_guard = TerminalRestoreGuard::new();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = inner(&mut terminal);

    let restore_result = restore_terminal(terminal.backend_mut());
    if restore_result.is_ok() {
        terminal_guard.disarm();
    }

    match result {
        Ok(()) => restore_result,
        Err(err) => Err(err),
    }
}

/// Gate the app behind the unlock screen (when the store is encrypted), then
/// build the app and enter the main loop. Runs with the terminal already in raw
/// mode / alternate screen so the unlock screen and image detection can both
/// query stdin.
fn run_after_unlock(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config_path: PathBuf,
    config: Config,
    mut store: JournalStore,
) -> AppResult<()> {
    // Pick up an encryption *disable* performed on another device before probing
    // for a lock: if this device just fell back to plaintext (its key and pins
    // retired), tell the user, since the change is silent and consequential.
    if store.reconcile_disabled_encryption()? {
        run_disable_notice(terminal)?;
    }

    if store.unlock_available() {
        if store.identity_needs_passphrase()? {
            if !run_unlock_screen(terminal, &mut store)? {
                // User quit at the unlock screen; exit cleanly without loading.
                return Ok(());
            }
        } else {
            // A plaintext identity opens without a passphrase — no unlock screen.
            store.unlock(None)?;
        }
    }

    // A device that can't decrypt this encrypted store — no key yet, awaiting
    // approval, or revoked — can't load history. Explain why and exit instead of
    // failing to load. (A store recipient unlocked above, so it resolves to
    // `Ready` and this passes straight through.)
    match store.resolve_access()? {
        StoreAccess::Ready => {}
        StoreAccess::AwaitingApproval { device_name } => {
            return run_pending_notice(
                terminal,
                &device_name,
                render::AccessNotice::AwaitingApproval,
            );
        }
        StoreAccess::NeedsEnroll {
            device_name,
            retired_key,
        } => {
            return run_pending_notice(
                terminal,
                &device_name,
                render::AccessNotice::NeedsEnroll { retired_key },
            );
        }
    }

    if !approve_pending_requests(terminal, &mut store)? {
        // User quit at a pending-request modal; exit cleanly.
        return Ok(());
    }

    let mut app = App::new(config_path, config, store)?;
    // Must run after raw mode: the detection query reads control-sequence
    // replies from stdin.
    app.image.runtime = image::ImageRuntime::detect(&app.store);
    run_loop(terminal, app)
}

/// Surface any pending device-access requests as a modal before the app loads,
/// approving/denying each in turn. Runs only on a device that can decrypt, since
/// approval re-encrypts the store with the unlocked identity. Returns `Ok(false)`
/// if the user quit with Ctrl-C; `Esc` defers the rest (they reappear next launch)
/// and returns `Ok(true)`.
fn approve_pending_requests(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store: &mut JournalStore,
) -> AppResult<bool> {
    // Only a device that can already read the store may approve others: approval
    // re-encrypts history, which a not-yet-approved device can't decrypt.
    if !store.is_current_recipient()? {
        return Ok(true);
    }

    let recipients = store.recipients()?;
    for request in store.pending_requests()? {
        // A request whose key is already a recipient (e.g. this device's own
        // request that synced back before its deletion) needs no approval — just
        // clear the stale file rather than prompting to re-add it.
        if recipients
            .iter()
            .any(|recipient| recipient.enc_key == request.recipient.enc_key)
        {
            store.deny_pending(&request)?;
            continue;
        }
        loop {
            terminal.draw(|frame| render::draw_pending_request(frame, &request, None))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(false);
            }
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    store.approve_pending(&request, |done, total| {
                        let _ = terminal.draw(|frame| {
                            render::draw_pending_request(frame, &request, Some((done, total)))
                        });
                    })?;
                    break;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    store.deny_pending(&request)?;
                    break;
                }
                // Defer: leave this and any remaining requests for next launch.
                KeyCode::Esc => return Ok(true),
                _ => {}
            }
        }
    }

    Ok(true)
}

/// Draw a full-screen notice and block until the user presses any key to
/// dismiss it.
fn wait_for_dismiss(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut draw: impl FnMut(&mut Frame),
) -> AppResult<()> {
    loop {
        terminal.draw(&mut draw)?;
        if let Event::Key(_) = event::read()? {
            return Ok(());
        }
    }
}

/// Show why a device that can't decrypt this encrypted store can't open the
/// journal, then exit on any key. `notice` picks the message — see
/// [`render::AccessNotice`].
fn run_pending_notice(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    device_name: &str,
    notice: render::AccessNotice,
) -> AppResult<()> {
    wait_for_dismiss(terminal, |frame| {
        render::draw_pending_notice(frame, device_name, &notice)
    })
}

/// Notify that encryption was disabled on another device, so this device retired
/// its key and trust pins and now opens the journal as plaintext. Dismissed on
/// any key, after which the app loads normally.
fn run_disable_notice(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<()> {
    wait_for_dismiss(terminal, render::draw_disable_notice)
}

/// Outcome of a single key press on the unlock screen.
enum UnlockAction {
    Submit,
    Cancel,
    Insert(char),
    Delete,
    MoveLeft,
    MoveRight,
    Ignore,
}

/// Map a key press to an unlock-screen action. Factored out from the loop so the
/// editing and submit/cancel rules stay unit-testable without a terminal.
fn unlock_key_action(key: KeyEvent) -> UnlockAction {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return UnlockAction::Cancel;
    }
    match key.code {
        KeyCode::Enter => UnlockAction::Submit,
        KeyCode::Esc => UnlockAction::Cancel,
        KeyCode::Backspace => UnlockAction::Delete,
        KeyCode::Left => UnlockAction::MoveLeft,
        KeyCode::Right => UnlockAction::MoveRight,
        KeyCode::Char(ch) => UnlockAction::Insert(ch),
        _ => UnlockAction::Ignore,
    }
}

/// Draw the fullscreen unlock screen and collect the passphrase until it unlocks
/// the store. Returns `Ok(true)` once unlocked, `Ok(false)` if the user quits
/// (Esc / Ctrl-C) first. The typed passphrase is zeroized as soon as it's been
/// handed to `store.unlock`, so it doesn't linger in the heap. The native
/// terminal cursor marks the caret, so nothing animates between events.
fn run_unlock_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store: &mut JournalStore,
) -> AppResult<bool> {
    let mut input = PassphraseInput::default();
    let mut error: Option<String> = None;

    loop {
        let mut field_rect = None;
        terminal.draw(|frame| field_rect = render::draw_unlock(frame, &input, error.as_deref()))?;

        match event::read()? {
            Event::Key(key) => match unlock_key_action(key) {
                UnlockAction::Cancel => {
                    input.zeroize();
                    return Ok(false);
                }
                UnlockAction::Submit => {
                    match store.unlock(Some(&SecretString::from(input.as_str()))) {
                        Ok(()) => {
                            input.zeroize();
                            return Ok(true);
                        }
                        Err(_) => {
                            input.zeroize();
                            error = Some("Incorrect passphrase".to_string());
                        }
                    }
                }
                UnlockAction::Insert(ch) => input.insert(ch),
                UnlockAction::Delete => input.backspace(),
                UnlockAction::MoveLeft => input.move_left(),
                UnlockAction::MoveRight => input.move_right(),
                UnlockAction::Ignore => {}
            },
            // Click in the field to place the caret, like every other input.
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
                ) =>
            {
                if let Some(rect) = field_rect
                    && mouse.row == rect.y
                    && mouse.column >= rect.x
                    && mouse.column < rect.x + rect.width
                {
                    input.click_at(mouse.column - rect.x);
                }
            }
            _ => {}
        }
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
    execute!(
        output,
        DisableMouseCapture,
        LeaveAlternateScreen,
        SetCursorStyle::DefaultUserShape,
        Show
    )?;
    Ok(())
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> AppResult<()> {
    let watcher = match watcher::FileWatcher::start(&app.config.journal.path) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            app.toast(
                state::ToastVariant::Warning,
                format!("Live journal reload unavailable: {error}"),
            );
            None
        }
    };
    // Watch the themes directory too: edits to the active theme's file repaint
    // live, no restart needed. (The directory exists — startup materialized it.)
    let theme_watcher = match watcher::FileWatcher::start(&theme::themes_dir(&app.config_path)) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            app.toast(
                state::ToastVariant::Warning,
                format!("Live theme reload unavailable: {error}"),
            );
            None
        }
    };
    let mut pending_theme_reload_at: Option<Instant> = None;

    terminal.draw(|frame| render::draw(frame, &mut app))?;
    let mut overlay_was_visible = app.has_overlay();
    // iTerm2 can miss the mouse-capture enable sent during terminal setup: its
    // motion-tracking area is rebuilt on a main-thread side effect that races
    // session startup, leaving hover dead until a focus change rebuilds it.
    // Re-asserting capture once startup has settled forces that rebuild;
    // redundant DECSETs are harmless on other terminals.
    let mut reassert_mouse_capture_at = Some(Instant::now() + Duration::from_millis(250));
    // When set, a watched file change is pending; reload once the filesystem has
    // been quiet until this deadline (coalesces import-time write storms). The
    // accumulated changed paths let the reload touch only affected entries.
    let mut pending_refresh_at: Option<Instant> = None;
    let mut pending_paths: Vec<PathBuf> = Vec::new();
    // Events drained while coalescing a scroll burst that weren't wheel events;
    // handled on the next iterations before polling for more input.
    let mut pending_events: VecDeque<Event> = VecDeque::new();

    loop {
        if reassert_mouse_capture_at.is_some_and(|at| Instant::now() >= at) {
            reassert_mouse_capture_at = None;
            execute!(io::stdout(), EnableMouseCapture)?;
        }
        // A newly finished image build makes the frame stale; repaint below.
        let images_ready = app.image.runtime.poll_results();
        // A finished geocode lookup updates the open location dialog; repaint too.
        let geocode_ready = app.apply_geocode_results();
        // Route any finished weather/air fetches: attach to the editor draft or
        // write back to the entry file. Then pace out the next backfill job.
        let context_ready = app.apply_environment_results();
        let reader_flash_changed = app.expire_reader_heading_flash();
        app.dispatch_environment_backfill();
        // Close the "Fetching…" modal and finish the deferred save once ready.
        let context_saved = events::poll_fetching_environment(&mut app)?;
        let mut poll_timeout = app
            .toast_deadline()
            .map(|t| t.min(Duration::from_millis(200)))
            .unwrap_or(Duration::from_millis(200));
        if let Some(flash) = app.reader_anchor_flash.as_ref() {
            poll_timeout = poll_timeout.min(flash.until.saturating_duration_since(Instant::now()));
        }
        // Poll briefly while builds are pending so results paint promptly.
        if app.image.runtime.has_pending() {
            poll_timeout = poll_timeout.min(Duration::from_millis(30));
        }
        // Likewise while a geocode lookup is in flight so its result lands
        // quickly.
        if app.geocode.has_pending() {
            poll_timeout = poll_timeout.min(Duration::from_millis(50));
        }
        // And while an environment fetch runs or backfill is pending, so the modal's
        // dots animate, results land, and backfill paces out on schedule.
        if app.environment.has_pending() || app.environment_backfill_active() {
            poll_timeout = poll_timeout.min(Duration::from_millis(100));
        }
        // Wake to run a debounced search recompute once typing pauses.
        if app.search.dirty {
            let remaining = app
                .search
                .last_edit
                .map(|edited| SEARCH_DEBOUNCE.saturating_sub(edited.elapsed()))
                .unwrap_or_default();
            poll_timeout = poll_timeout.min(remaining);
        }
        // Wake to run a debounced store reload once file changes settle.
        if let Some(at) = pending_refresh_at {
            poll_timeout = poll_timeout.min(at.saturating_duration_since(Instant::now()));
        }
        // Likewise for a debounced theme reload.
        if let Some(at) = pending_theme_reload_at {
            poll_timeout = poll_timeout.min(at.saturating_duration_since(Instant::now()));
        }

        let event = if let Some(ev) = pending_events.pop_front() {
            Some(ev)
        } else if event::poll(poll_timeout)? {
            Some(event::read()?)
        } else {
            None
        };

        let redraw = match event {
            Some(Event::Key(key)) => {
                // Back to keyboard mode: a parked cursor must not keep its
                // hover glow while the user arrows around.
                app.clear_hover();
                // No global Ctrl+C quit: `q` quits the app, and the editor forwards
                // Ctrl+C to the textarea as copy.
                if events::handle_key(terminal, &mut app, key)?.should_quit() {
                    break;
                }
                true
            }
            Some(Event::Mouse(mouse))
                if events::is_wheel(mouse.kind) && !app.has_overlay() && app.editor.is_none() =>
            {
                // Coalesce a macOS smooth-scroll burst into one applied step and
                // one repaint: drain everything already queued, sum the net
                // wheel movement, and apply it once. A reverse flick cancels the
                // queued momentum instead of the app crawling back through the
                // whole tail one repaint at a time.
                let mut batch = vec![Event::Mouse(mouse)];
                while event::poll(Duration::ZERO)? {
                    batch.push(event::read()?);
                }
                let (net, consumed) = events::fold_leading_wheel(&batch);
                // Non-wheel events after the run are handled on later iterations.
                for ev in batch.split_off(consumed).into_iter().rev() {
                    pending_events.push_front(ev);
                }
                let last = match batch.last() {
                    Some(Event::Mouse(m)) => *m,
                    _ => mouse,
                };
                let area = events::terminal_area(terminal)?;
                events::handle_scroll(terminal, &mut app, last, area, net)?;
                true
            }
            Some(Event::Mouse(mouse)) if mouse.kind == MouseEventKind::Moved => {
                // Coalesce a motion burst: only the cursor's latest position
                // matters. Repaint only when the hovered target actually
                // changed, so motion inside one row costs nothing.
                let mut last = mouse;
                while event::poll(Duration::ZERO)? {
                    match event::read()? {
                        Event::Mouse(m) if m.kind == MouseEventKind::Moved => last = m,
                        other => {
                            pending_events.push_back(other);
                            break;
                        }
                    }
                }
                let area = events::terminal_area(terminal)?;
                events::update_hover(terminal, &mut app, last.column, last.row, area)?
            }
            Some(Event::Mouse(mouse)) => {
                if events::handle_mouse(terminal, &mut app, mouse)?.should_quit() {
                    break;
                }
                true
            }
            Some(Event::Resize(_, _)) => true,
            Some(_) => false,
            None => app.expire_toasts(),
        };

        // Debounce watcher-driven reloads: each change pushes the deadline out and
        // accumulates the changed paths; the reload runs only once no change has
        // arrived for the quiet period, then re-reads just those entries.
        let changed = watcher
            .as_ref()
            .map_or_else(Vec::new, watcher::FileWatcher::poll_changes);
        if !changed.is_empty() {
            pending_paths.extend(changed);
            pending_refresh_at = Some(Instant::now() + REFRESH_DEBOUNCE);
        }
        let refreshed = if pending_refresh_at.is_some_and(|at| Instant::now() >= at) {
            pending_refresh_at = None;
            let paths = std::mem::take(&mut pending_paths);
            if let Err(error) = app.refresh_paths(&paths) {
                app.toast(
                    state::ToastVariant::Error,
                    format!("Journal changes not reloaded: {error}"),
                );
            }
            true
        } else {
            false
        };

        // Live theme reload, debounced the same way: only changes to the
        // active theme's file count (edits to other themes wait until they're
        // selected). A broken edit keeps the current theme and says so.
        let active_theme_changed = theme_watcher
            .as_ref()
            .map_or_else(Vec::new, watcher::FileWatcher::poll_changes)
            .iter()
            .any(|path| {
                path.extension().is_some_and(|ext| ext == "toml")
                    && path
                        .file_stem()
                        .is_some_and(|stem| stem == app.config.ui.theme.as_str())
            });
        if active_theme_changed {
            pending_theme_reload_at = Some(Instant::now() + REFRESH_DEBOUNCE);
        }
        let theme_reloaded = if pending_theme_reload_at.is_some_and(|at| Instant::now() >= at) {
            pending_theme_reload_at = None;
            let path =
                theme::themes_dir(&app.config_path).join(format!("{}.toml", app.config.ui.theme));
            match theme::load_file(&path, theme::mode()) {
                Ok(reloaded) => theme::install(reloaded),
                Err(err) => app.toast(
                    state::ToastVariant::Error,
                    format!("Theme not reloaded: {err:#}"),
                ),
            }
            true
        } else {
            false
        };

        // Run the debounced search recompute once typing has paused.
        let search_recomputed = if app.search.dirty
            && app
                .search
                .last_edit
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

        // Repaint each tick while the viewer's image builds, or the "Fetching…"
        // modal is up, so their loading ellipsis keeps animating.
        let animate_loading = (app.image_viewer_state().is_some()
            && app.image.runtime.has_pending())
            || matches!(app.overlay, state::Overlay::FetchingEnvironment(_));

        if redraw
            || refreshed
            || theme_reloaded
            || search_recomputed
            || images_ready
            || geocode_ready
            || context_ready
            || context_saved
            || reader_flash_changed
            || animate_loading
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
    if app.state.last_journal != selected {
        app.state.last_journal = selected;
        crate::config::save_state(&app.config_path, &app.state)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Drive a fresh passphrase buffer through a sequence of keys the same way
    /// `run_unlock_screen` does, returning the resulting buffer and whether it
    /// submitted (Enter) or cancelled (Esc / Ctrl-C).
    fn drive(keys: &[KeyEvent]) -> (String, Option<bool>) {
        let mut input = text_input::PassphraseInput::default();
        for &k in keys {
            match unlock_key_action(k) {
                UnlockAction::Submit => return (input.as_str().to_string(), Some(true)),
                UnlockAction::Cancel => return (input.as_str().to_string(), Some(false)),
                UnlockAction::Insert(ch) => input.insert(ch),
                UnlockAction::Delete => {
                    input.backspace();
                }
                UnlockAction::MoveLeft => input.move_left(),
                UnlockAction::MoveRight => input.move_right(),
                UnlockAction::Ignore => {}
            }
        }
        (input.as_str().to_string(), None)
    }

    #[test]
    fn typing_and_backspace_edit_the_passphrase() {
        let (input, done) = drive(&[
            key(KeyCode::Char('h')),
            key(KeyCode::Char('i')),
            key(KeyCode::Char('x')),
            key(KeyCode::Backspace),
        ]);
        assert_eq!(input, "hi");
        assert_eq!(done, None);
    }

    #[test]
    fn enter_submits_the_typed_passphrase() {
        let (input, done) = drive(&[
            key(KeyCode::Char('p')),
            key(KeyCode::Char('w')),
            key(KeyCode::Enter),
        ]);
        assert_eq!(input, "pw");
        assert_eq!(done, Some(true));
    }

    #[test]
    fn esc_cancels() {
        let (_input, done) = drive(&[key(KeyCode::Char('p')), key(KeyCode::Esc)]);
        assert_eq!(done, Some(false));
    }

    #[test]
    fn ctrl_c_cancels() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(unlock_key_action(ctrl_c), UnlockAction::Cancel));
    }

    #[test]
    fn non_editing_keys_are_ignored() {
        assert!(matches!(
            unlock_key_action(key(KeyCode::Up)),
            UnlockAction::Ignore
        ));
        assert!(matches!(
            unlock_key_action(key(KeyCode::Tab)),
            UnlockAction::Ignore
        ));
    }

    #[test]
    fn arrows_move_the_caret_for_mid_passphrase_edits() {
        // "ac", Left, "b" → the caret edit lands between the two chars.
        let (input, done) = drive(&[
            key(KeyCode::Char('a')),
            key(KeyCode::Char('c')),
            key(KeyCode::Left),
            key(KeyCode::Char('b')),
        ]);
        assert_eq!(input, "abc");
        assert_eq!(done, None);
    }
}
