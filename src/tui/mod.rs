mod app;
mod entry_rows;
mod events;
mod hit_test;
mod image;
mod render;
mod scroll;
mod state;
mod surface;
#[cfg(test)]
mod test_support;
mod watcher;

use crate::{AppResult, config::Config};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use journal_storage::{JournalStore, SecretString, StoreAccess};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use std::path::PathBuf;
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use zeroize::Zeroize;

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
    // Ensure the store exists before probing for a lock so identity checks
    // reflect on-disk state.
    store.ensure()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal_guard = TerminalRestoreGuard::new();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_after_unlock(&mut terminal, config_path, config, store);

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
            .any(|recipient| recipient.key == request.recipient.key)
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
        KeyCode::Char(ch) => UnlockAction::Insert(ch),
        _ => UnlockAction::Ignore,
    }
}

/// Draw the fullscreen unlock screen and collect the passphrase until it unlocks
/// the store. Returns `Ok(true)` once unlocked, `Ok(false)` if the user quits
/// (Esc / Ctrl-C) first. The typed passphrase is zeroized as soon as it's been
/// handed to `store.unlock`, so it doesn't linger in the heap.
fn run_unlock_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store: &mut JournalStore,
) -> AppResult<bool> {
    let mut input = String::new();
    let mut error: Option<String> = None;
    // Reuse the search caret's blink cadence: a keystroke holds it solid, idle
    // toggles it on the blink half-period.
    let mut caret_visible = true;
    let mut last_blink = Instant::now();

    loop {
        terminal
            .draw(|frame| render::draw_unlock(frame, &input, error.as_deref(), caret_visible))?;

        if event::poll(CURSOR_BLINK)? {
            if let Event::Key(key) = event::read()? {
                caret_visible = true;
                last_blink = Instant::now();
                match unlock_key_action(key) {
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
                    UnlockAction::Insert(ch) => input.push(ch),
                    UnlockAction::Delete => {
                        input.pop();
                    }
                    UnlockAction::Ignore => {}
                }
            }
        } else if last_blink.elapsed() >= CURSOR_BLINK {
            last_blink = Instant::now();
            caret_visible = !caret_visible;
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

        // Repaint each tick while the viewer's image builds so the loading
        // ellipsis keeps animating.
        let animate_loading = app.image_viewer_state().is_some() && app.image.runtime.has_pending();

        // Drive the search caret's blink: keystrokes hold it solid, idle toggles
        // it on the blink half-period, and outside the field it stays visible.
        let mut blink_toggled = false;
        if app.is_search_input_active() {
            if is_key_event {
                last_blink = Instant::now();
                if !app.search.cursor_visible {
                    app.search.cursor_visible = true;
                    blink_toggled = true;
                }
            } else if last_blink.elapsed() >= CURSOR_BLINK {
                last_blink = Instant::now();
                app.search.cursor_visible = !app.search.cursor_visible;
                blink_toggled = true;
            }
        } else if !app.search.cursor_visible {
            app.search.cursor_visible = true;
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
        let mut input = String::new();
        for &k in keys {
            match unlock_key_action(k) {
                UnlockAction::Submit => return (input, Some(true)),
                UnlockAction::Cancel => return (input, Some(false)),
                UnlockAction::Insert(ch) => input.push(ch),
                UnlockAction::Delete => {
                    input.pop();
                }
                UnlockAction::Ignore => {}
            }
        }
        (input, None)
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
            unlock_key_action(key(KeyCode::Left)),
            UnlockAction::Ignore
        ));
        assert!(matches!(
            unlock_key_action(key(KeyCode::Tab)),
            UnlockAction::Ignore
        ));
    }
}
