mod effects;
mod redraw;
mod scheduler;
mod terminal;
mod watcher;
pub(crate) mod worker;

use super::{events, image, render, state, theme};
use crate::{AppResult, config::Config};
use crossterm::{
    event::{
        self, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
};
use notema_encryption::SecretString;
use notema_storage::{CachePolicy, CachedLibrary, JournalStore, LibraryDiscovery, StoreAccess};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::{
    io,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

/// Quiet period after the last search keystroke before the (whole-corpus) hit
/// recompute runs, so fast typing doesn't re-scan every entry per key.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);

/// Quiet period after the last watched file change before reloading the store,
/// so a burst of writes (e.g. a Day One import) collapses into one reload.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(400);

use super::app::AppModel;
use super::text_input::PassphraseInput;

pub(crate) fn run(
    config_path: PathBuf,
    config: Config,
    store: JournalStore,
    discovery: Option<LibraryDiscovery>,
) -> AppResult<()> {
    // Ensure the store exists before probing for a lock so identity checks
    // reflect on-disk state.
    store.ensure()?;
    // Before raw mode / the alternate screen: auto dark/light detection talks
    // OSC to the normal screen, and load warnings should print readably.
    let startup = theme::load_startup(&config_path, &config.ui);
    terminal::with_terminal(|terminal| {
        run_after_unlock(terminal, config_path, config, store, discovery, &startup)
    })
}

/// Launch straight into a fullscreen new-entry editor and quit once the entry is
/// saved or discarded — the `notema log` no-body path. Never prompts for a
/// passphrase: a passphrase-less identity is unlocked silently so the metadata
/// dialogs can suggest recently-used people/tags from existing entries, but an
/// identity that *needs* a passphrase is left locked (writing a new entry needs
/// only the recipients roster, so it works either way; the store's other entries
/// stay locked placeholders behind the editor).
pub(crate) fn run_compose(
    config_path: PathBuf,
    config: Config,
    mut store: JournalStore,
    journal: String,
    metadata: notema_domain::Metadata,
) -> AppResult<()> {
    store.ensure()?;
    let startup = theme::load_startup(&config_path, &config.ui);
    if store.unlock_available() && !store.identity_needs_passphrase()? {
        store.unlock(None)?;
    }
    terminal::with_terminal(|terminal| {
        let validate_library = !store.encryption_enabled() || store.is_unlocked();
        let (mut app, cached) =
            AppModel::new_cached(config_path, config, store, startup.detected_mode)?;
        app.begin_compose(journal, metadata);
        let initial_validation = validate_library.then(|| InitialLibraryValidation {
            cached,
            discovery: None,
            generation: app.library_generation(),
        });
        run_loop(terminal, app, initial_validation)
    })
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
    mut discovery: Option<LibraryDiscovery>,
    startup: &theme::StartupTheme,
) -> AppResult<()> {
    // Pick up an encryption *disable* performed on another device before probing
    // for a lock: if this device just fell back to plaintext (its key and pins
    // retired), tell the user, since the change is silent and consequential.
    if store.reconcile_disabled_encryption()? {
        discovery = None;
        run_disable_notice(terminal, &startup.theme)?;
    }

    if store.unlock_available() {
        if store.identity_needs_passphrase()? {
            if !run_unlock_screen(terminal, &mut store, &startup.theme)? {
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
                &startup.theme,
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
                &startup.theme,
            );
        }
    }

    let had_pending_requests = !store.pending_requests()?.is_empty();
    if !approve_pending_requests(terminal, &mut store, &startup.theme)? {
        // User quit at a pending-request modal; exit cleanly.
        return Ok(());
    }
    if had_pending_requests {
        discovery = None;
    }

    let (mut app, cached) =
        AppModel::new_cached(config_path, config, store, startup.detected_mode)?;
    // Must run after raw mode: the detection query reads control-sequence
    // replies from stdin.
    app.image.runtime = image::ImageRuntime::detect(&app.services.store);
    let generation = app.library_generation();
    run_loop(
        terminal,
        app,
        Some(InitialLibraryValidation {
            cached,
            discovery,
            generation,
        }),
    )
}

/// Surface any pending device-access requests as a modal before the app loads,
/// approving/denying each in turn. Runs only on a device that can decrypt, since
/// approval re-encrypts the store with the unlocked identity. Returns `Ok(false)`
/// if the user quit with Ctrl-C; `Esc` defers the rest (they reappear next launch)
/// and returns `Ok(true)`.
fn approve_pending_requests(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store: &mut JournalStore,
    theme: &theme::Theme,
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
            .any(|recipient| recipient.encryption_key == request.recipient.encryption_key)
        {
            store.deny_pending(&request)?;
            continue;
        }
        loop {
            terminal.draw(|frame| render::draw_pending_request(theme, frame, &request, None))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            // With the keyboard-enhancement protocol on, keys also report release
            // and repeat; only act on the initial press.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(false);
            }
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    store.approve_pending(&request, |done, total| {
                        let _ = terminal.draw(|frame| {
                            render::draw_pending_request(
                                theme,
                                frame,
                                &request,
                                Some((done, total)),
                            )
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
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
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
    theme: &theme::Theme,
) -> AppResult<()> {
    wait_for_dismiss(terminal, |frame| {
        render::draw_pending_notice(theme, frame, device_name, &notice)
    })
}

/// Notify that encryption was disabled on another device, so this device retired
/// its key and trust pins and now opens the journal as plaintext. Dismissed on
/// any key, after which the app loads normally.
fn run_disable_notice(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    theme: &theme::Theme,
) -> AppResult<()> {
    wait_for_dismiss(terminal, |frame| render::draw_disable_notice(theme, frame))
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
    theme: &theme::Theme,
) -> AppResult<bool> {
    let mut input = PassphraseInput::default();
    let mut error: Option<String> = None;

    loop {
        let mut field_rect = None;
        terminal.draw(|frame| {
            field_rect = render::draw_unlock(theme, frame, &input, error.as_deref())
        })?;

        match event::read()? {
            // Only the initial press edits the field; skip release/repeat reports
            // the enhancement protocol adds.
            Event::Key(key) if key.kind == KeyEventKind::Press => match unlock_key_action(key) {
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
            // Bracketed paste is enabled terminal-wide, so a pasted passphrase
            // (e.g. from a password manager) arrives here, not as key presses.
            Event::Paste(text) => {
                for ch in text.chars().filter(|c| !c.is_control()) {
                    input.insert(ch);
                }
            }
            _ => {}
        }
    }
}

struct InitialLibraryValidation {
    cached: Option<CachedLibrary>,
    discovery: Option<LibraryDiscovery>,
    generation: u64,
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: AppModel,
    initial_validation: Option<InitialLibraryValidation>,
) -> AppResult<()> {
    let mut view = super::ui::ViewState::default();
    let is_ish = crate::platform::ish::is_ish();
    let watcher = if is_ish {
        None
    } else {
        match watcher::FileWatcher::start(&app.services.config.journal.path) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                app.toast(
                    state::ToastVariant::Warning,
                    format!("Live journal reload unavailable: {error}"),
                );
                None
            }
        }
    };
    let validation_generation = initial_validation
        .as_ref()
        .map(|validation| validation.generation);
    let validation_rx = initial_validation.map(|validation| {
        let store = app.services.store.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = match validation.discovery {
                Some(discovery) => store.validate_discovered_library(
                    validation.cached,
                    CachePolicy::Normal,
                    discovery,
                ),
                None => store.validate_library(validation.cached, CachePolicy::Normal),
            }
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
        });
        rx
    });
    // Watch the themes directory too: edits to the active theme's file repaint
    // live, no restart needed. (The directory exists — startup materialized it.)
    let theme_watcher = if is_ish {
        None
    } else {
        match watcher::FileWatcher::start(&theme::themes_dir(&app.services.config_path)) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                app.toast(
                    state::ToastVariant::Warning,
                    format!("Live theme reload unavailable: {error}"),
                );
                None
            }
        }
    };
    let mut pending_theme_reload_at: Option<Instant> = None;

    redraw::draw(terminal, &mut app, &mut view)?;
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
    let mut validation_dirty = false;
    let mut validation_finished = validation_rx.is_none();

    loop {
        // Consume source changes before accepting the startup snapshot. If any
        // landed while validation was running, rebuild once from the current
        // tree instead of installing a result that may predate the change.
        let changed = watcher
            .as_ref()
            .map_or_else(Vec::new, watcher::FileWatcher::poll_changes);
        if !changed.is_empty() {
            if !validation_finished {
                validation_dirty = true;
            }
            pending_paths.extend(changed);
            pending_refresh_at = Some(Instant::now() + REFRESH_DEBOUNCE);
        }
        let validation_result = validation_rx
            .as_ref()
            .and_then(|rx| poll_library_validation(rx, validation_finished));
        let library_updated = match validation_result {
            Some(Ok(_snapshot))
                if initial_library_result_is_stale(
                    validation_generation,
                    app.library_generation(),
                    validation_dirty,
                ) =>
            {
                validation_finished = true;
                let _ = events::dispatch_action(
                    terminal,
                    &mut app,
                    events::Action::Background(events::BackgroundAction::LibraryValidationStale),
                );
                true
            }
            Some(Ok(snapshot)) => {
                validation_finished = true;
                let _ = events::dispatch_action(
                    terminal,
                    &mut app,
                    events::Action::Background(events::BackgroundAction::LibraryValidated(
                        Box::new(snapshot),
                    )),
                );
                true
            }
            Some(Err(error)) => {
                validation_finished = true;
                let _ = events::dispatch_action(
                    terminal,
                    &mut app,
                    events::Action::Background(events::BackgroundAction::LibraryValidationFailed(
                        error,
                    )),
                );
                true
            }
            None => false,
        };
        if reassert_mouse_capture_at.is_some_and(|at| Instant::now() >= at) {
            reassert_mouse_capture_at = None;
            execute!(io::stdout(), EnableMouseCapture)?;
        }
        // A newly finished image build makes the frame stale; repaint below.
        let images_ready = events::dispatch_action(
            terminal,
            &mut app,
            events::Action::Background(events::BackgroundAction::PollImages),
        )?
        .redraw;
        // A finished geocode lookup updates the open location dialog or writes back
        // a backfilled address; either repaints, and the outcome may dispatch the
        // next paced reverse lookup, so execute its effects.
        let geocode_outcome = events::dispatch_action(
            terminal,
            &mut app,
            events::Action::Background(events::BackgroundAction::PollGeocode),
        )?;
        let geocode_ready = effects::execute(terminal, &mut app, geocode_outcome)?.redraw;
        // Route any finished weather/air fetches: attach to the editor draft or
        // write back to the entry file. Then pace out the next backfill job.
        let context_outcome = events::dispatch_action(
            terminal,
            &mut app,
            events::Action::Background(events::BackgroundAction::PollEnvironment),
        )?;
        let context_ready = effects::execute(terminal, &mut app, context_outcome)?.redraw;
        let poll_timeout = scheduler::poll_timeout(
            &app,
            terminal.size()?.width,
            pending_refresh_at,
            pending_theme_reload_at,
        );

        let event = if let Some(ev) = pending_events.pop_front() {
            Some(ev)
        } else if event::poll(poll_timeout)? {
            Some(event::read()?)
        } else {
            None
        };

        let redraw = match event {
            // Skip release reports the enhancement protocol adds; keep repeats so
            // held keys still autorepeat (typing, scrolling).
            Some(Event::Key(key)) if key.kind == KeyEventKind::Release => false,
            Some(Event::Key(key)) => {
                // Back to keyboard mode: a parked cursor must not keep its
                // hover glow while the user arrows around.
                events::dispatch_action(
                    terminal,
                    &mut app,
                    events::Action::SetHover(crate::tui::state::HoverTarget::None),
                )?;
                // No global Ctrl+C quit: `q` quits the app, and the editor forwards
                // Ctrl+C to the textarea as copy.
                let outcome = events::handle_key(terminal, &mut app, key)?;
                let outcome = effects::execute(terminal, &mut app, outcome)?;
                if outcome.should_quit() {
                    break;
                }
                outcome.redraw
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
                // They were just polled, so they're newer than anything already
                // queued — append them after, don't jump them ahead.
                for ev in batch.split_off(consumed) {
                    pending_events.push_back(ev);
                }
                let last = match batch.last() {
                    Some(Event::Mouse(m)) => *m,
                    _ => mouse,
                };
                let area = events::terminal_area(terminal)?;
                events::handle_scroll(terminal, &mut app, last, area, net, &view)?;
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
                events::update_hover(terminal, &mut app, last.column, last.row, area, &view)?
            }
            Some(Event::Mouse(mouse)) => {
                let outcome = events::handle_mouse(terminal, &mut app, mouse, &view)?;
                let outcome = effects::execute(terminal, &mut app, outcome)?;
                if outcome.should_quit() {
                    break;
                }
                outcome.redraw
            }
            Some(Event::Paste(text)) => {
                let outcome = events::handle_paste(terminal, &mut app, text)?;
                let outcome = effects::execute(terminal, &mut app, outcome)?;
                if outcome.should_quit() {
                    break;
                }
                outcome.redraw
            }
            Some(Event::Resize(_, _)) => true,
            Some(_) => false,
            None => false,
        };

        // Timer completions re-enter through dispatch even under a continuous
        // event stream, so toasts and heading flashes cannot linger behind input.
        let timers_changed = events::dispatch_action(
            terminal,
            &mut app,
            events::Action::Background(events::BackgroundAction::PollTimers),
        )?
        .redraw;
        let redraw = redraw || timers_changed;

        // Debounce watcher-driven reloads: each change pushes the deadline out and
        // accumulates the changed paths; the reload runs only once no change has
        // arrived for the quiet period, then re-reads just those entries.
        let refreshed = if pending_refresh_at.is_some_and(|at| Instant::now() >= at) {
            pending_refresh_at = None;
            let paths = std::mem::take(&mut pending_paths);
            events::dispatch_action(
                terminal,
                &mut app,
                events::Action::Background(events::BackgroundAction::LibraryPathsChanged(paths)),
            )?;
            true
        } else {
            false
        };

        // Live theme reload, debounced the same way: only changes to the
        // active theme's file count (edits to other themes wait until they're
        // selected). A broken edit keeps the current theme and says so.
        let active_theme = app.effective_theme_name();
        let active_theme_changed = theme_watcher
            .as_ref()
            .map_or_else(Vec::new, watcher::FileWatcher::poll_changes)
            .iter()
            .any(|path| {
                path.extension().is_some_and(|ext| ext == "toml")
                    && path
                        .file_stem()
                        .is_some_and(|stem| stem == active_theme.as_str())
            });
        if active_theme_changed {
            pending_theme_reload_at = Some(Instant::now() + REFRESH_DEBOUNCE);
        }
        let theme_reloaded = if pending_theme_reload_at.is_some_and(|at| Instant::now() >= at) {
            pending_theme_reload_at = None;
            let name = app.effective_theme_name();
            events::dispatch_action(
                terminal,
                &mut app,
                events::Action::Background(events::BackgroundAction::ReloadTheme(name)),
            )?;
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
            events::dispatch_action(
                terminal,
                &mut app,
                events::Action::Background(events::BackgroundAction::CommitSearch),
            )?;
            true
        } else {
            false
        };

        // Warm the viewer's images once it's open and drop them when the entry
        // closes; cheap when nothing changed, rebuilds on change.
        let image_outcome = events::dispatch_action(
            terminal,
            &mut app,
            events::Action::SyncImages(terminal.size()?),
        )?;
        let _ = effects::execute(terminal, &mut app, image_outcome)?;

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
        // Repaint each tick while toasts are visible so the countdown line
        // shrinks continuously; the loop already wakes on the toast deadline.
        let animate_toasts = !app.toasts.items().is_empty();

        if redraw
            || library_updated
            || refreshed
            || theme_reloaded
            || search_recomputed
            || images_ready
            || geocode_ready
            || context_ready
            || timers_changed
            || animate_loading
            || animate_toasts
        {
            if overlay_closed && app.image.runtime.uses_graphics() {
                terminal.clear()?;
            }
            redraw::draw(terminal, &mut app, &mut view)?;
        }
    }

    // Remember the selected journal (by stable id) for the next session. All break
    // paths fall through here, so this covers every exit (including Ctrl-C).
    let selected = app
        .selected_journal()
        .map(|journal| journal.id.clone())
        .filter(|id| !id.is_empty());
    if app.state.last_journal_id != selected {
        app.state.last_journal_id = selected;
        // Best-effort: a failed preference write shouldn't turn a clean quit into
        // a printed error, and a toast can't render once we're tearing down.
        let _ = crate::config::save_state(&app.services.config_path, &app.state);
    }

    Ok(())
}

fn poll_library_validation<T>(
    receiver: &std::sync::mpsc::Receiver<Result<T, String>>,
    finished: bool,
) -> Option<Result<T, String>> {
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
        Err(std::sync::mpsc::TryRecvError::Disconnected) if !finished => Some(Err(
            "journal validation worker stopped unexpectedly".to_string(),
        )),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
    }
}

fn initial_library_result_is_stale(
    started_at: Option<u64>,
    current: u64,
    watcher_dirty: bool,
) -> bool {
    watcher_dirty || started_at.is_some_and(|started_at| started_at != current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn disconnected_library_validator_is_reported_once() {
        let (sender, receiver) = std::sync::mpsc::channel::<Result<(), String>>();
        drop(sender);

        let result = poll_library_validation(&receiver, false).unwrap();

        assert_eq!(
            result.unwrap_err(),
            "journal validation worker stopped unexpectedly"
        );
        assert!(poll_library_validation(&receiver, true).is_none());
    }

    #[test]
    fn changed_library_rejects_an_older_validation_result() {
        assert!(!initial_library_result_is_stale(Some(4), 4, false));
        assert!(initial_library_result_is_stale(Some(4), 5, false));
        assert!(initial_library_result_is_stale(Some(4), 4, true));
    }

    /// Drive a fresh passphrase buffer through a sequence of keys the same way
    /// `run_unlock_screen` does, returning the resulting buffer and whether it
    /// submitted (Enter) or cancelled (Esc / Ctrl-C).
    fn drive(keys: &[KeyEvent]) -> (String, Option<bool>) {
        let mut input = PassphraseInput::default();
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
