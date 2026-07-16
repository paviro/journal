mod action;
mod actions;
mod handlers;
mod keyboard;
mod mouse;

use ratatui::{Terminal, backend::Backend, layout::Rect};

use crate::{
    AppResult,
    tui::{
        app::{AppModel, Focus, reader_is_available},
        editor_state::{EditorPrompt, EditorTarget},
        render,
        state::{ListNav, Overlay, ToastVariant},
    },
};
use ratatui_textarea::CursorMove;

pub(crate) use action::{
    Action, BackgroundAction, BrowserAction, EditorAction, ImageAction, InsightsAction,
    LocationAction, MetadataAction, OverlayAction, ReaderAction, SearchAction, SettingsAction,
};
use actions::{
    delete_selected, delete_selected_journal, open_reader_link, save_internal_editor,
    set_feelings_on_entry, set_location_on_entry, set_metadata_on_entry, set_mood_on_entry,
    submit_new_journal, toggle_archive_selected_journal, toggle_starred_on_entry, view_selected,
};
pub(crate) use keyboard::{handle_key, handle_paste};
pub(crate) use mouse::{fold_leading_wheel, handle_mouse, handle_scroll, is_wheel, update_hover};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControlFlow {
    Continue,
    Quit,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DispatchOutcome {
    pub(crate) control: ControlFlow,
    pub(crate) redraw: bool,
    pub(crate) effects: Vec<Effect>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    Redraw,
    Geocode(crate::tui::geocode::GeocodeRequest),
    Environment(crate::tui::environment::EnvironmentRequest),
    PrepareImages(crate::tui::image::WarmRequest),
    Open {
        target: OpenTarget,
        success_message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OpenTarget {
    Path(std::path::PathBuf),
    Uri(String),
}

impl DispatchOutcome {
    #[allow(non_upper_case_globals)]
    pub(crate) const Continue: Self = Self {
        control: ControlFlow::Continue,
        redraw: true,
        effects: Vec::new(),
    };

    #[allow(non_upper_case_globals)]
    pub(crate) const Quit: Self = Self {
        control: ControlFlow::Quit,
        redraw: false,
        effects: Vec::new(),
    };

    pub(crate) fn should_quit(&self) -> bool {
        matches!(self.control, ControlFlow::Quit)
    }

    fn with_effect(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }
}

/// How long the "Fetching weather and air quality…" modal waits before giving up
/// and saving without the data.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Drive the [`Overlay::FetchingEnvironment`] modal: once the editor's background
/// fetch lands (or the timeout fires) close it and re-run the deferred save.
/// Returns whether it acted, so the event loop knows to repaint. No-op when the
/// modal isn't open.
pub(crate) fn poll_fetching_environment(app: &mut AppModel) -> bool {
    let Overlay::FetchingEnvironment(started) = app.overlay else {
        return false;
    };
    let landed = app
        .editor
        .as_ref()
        .is_none_or(|editor| editor.pending_environment.is_none());
    let timed_out = started.elapsed() >= FETCH_TIMEOUT;
    if !(landed || timed_out) {
        return false;
    }
    // Timed out with nothing yet: give up waiting so the save proceeds bare.
    if timed_out && let Some(editor) = app.editor.as_mut() {
        editor.pending_environment = None;
    }
    app.close_overlay();
    if let Err(error) = save_editor_with_reader_restore(app) {
        report_action_error(app, &error);
    }
    true
}

/// Save the open editor and, for an edit of an existing entry, restore the
/// reader's selection/scroll/fullscreen afterward (the reload can reorder
/// entries). Shared by the direct `EditorSave` action and the deferred re-run
/// after a pending environment fetch, so both restore the reader identically.
fn save_editor_with_reader_restore(app: &mut AppModel) -> AppResult<()> {
    let restore_existing = matches!(
        app.editor.as_ref().map(|editor| &editor.target),
        Some(EditorTarget::Existing { .. })
    );
    let snapshot = restore_existing
        .then(|| ReaderSnapshot::capture(app))
        .flatten();
    save_internal_editor(app)?;
    if restore_existing && app.editor.is_none() {
        restore_reader_or_close(app, snapshot);
    }
    Ok(())
}

pub(crate) fn dispatch_action<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    action: Action,
) -> AppResult<DispatchOutcome> {
    let action_reads_selected = matches!(&action, Action::Browser(BrowserAction::ViewSelected));
    let before_reader_target = (app.nav.focus == Focus::Reader)
        .then(|| app.selected_entry_target().map(|target| target.path))
        .flatten();
    let result = apply_action(terminal, app, action);
    let after_reader_target = (app.nav.focus == Focus::Reader && app.editor.is_none())
        .then(|| app.selected_entry_target().map(|target| target.path))
        .flatten();
    let entered_or_changed_reader = after_reader_target.is_some()
        && !action_reads_selected
        && (before_reader_target.is_none() || before_reader_target != after_reader_target);
    let result = result.and_then(|outcome| {
        if entered_or_changed_reader {
            app.reload_selected_entry_from_disk()?;
        }
        Ok(outcome)
    });
    recover_action_error(app, result)
}

fn recover_action_error(
    app: &mut AppModel,
    result: AppResult<DispatchOutcome>,
) -> AppResult<DispatchOutcome> {
    match result {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            report_action_error(app, &error);
            Ok(DispatchOutcome::Continue)
        }
    }
}

fn report_action_error(app: &mut AppModel, error: &anyhow::Error) {
    let detail = error.to_string();
    let first_line = detail.lines().next().unwrap_or("Unknown error");
    let mut concise: String = first_line.chars().take(120).collect();
    if first_line.chars().count() > 120 {
        concise.push('…');
    }
    app.toast(ToastVariant::Error, format!("Action failed: {concise}"));
}

fn apply_action<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    action: Action,
) -> AppResult<DispatchOutcome> {
    match action {
        Action::Mouse(action) => {
            if let Some(followup) = mouse::apply_mouse_action(app, action)? {
                return apply_action(terminal, app, followup);
            }
        }
        Action::SetHover(target) => app.hover = target,
        Action::ViewRendered {
            reader_scroll,
            insights_scroll,
            journal_offset,
            entry_offset,
        } => {
            if let Some(scroll) = reader_scroll {
                app.nav.scroll.reader = scroll;
            }
            if let Some(scroll) = insights_scroll {
                app.nav.scroll.insights = scroll;
            }
            if let Some(offset) = journal_offset {
                *app.nav.journal_list.offset_mut() = offset;
            }
            if let Some(offset) = entry_offset {
                *app.nav.entry_list.offset_mut() = offset;
            }
        }
        Action::SyncImages(size) => {
            if let Some(request) = app.prepare_image_warm(size) {
                return Ok(DispatchOutcome::Continue.with_effect(Effect::PrepareImages(request)));
            }
        }
        Action::Quit => return Ok(DispatchOutcome::Quit),
        Action::RefreshLibrary => {
            app.begin_manual_refresh();
            let mut view = crate::tui::ui::ViewState::default();
            let active_theme = app.appearance.theme.clone();
            let mut context = crate::tui::ui::RenderContext::new(&active_theme, &mut view);
            if let Err(error) = terminal.draw(|frame| render::draw(frame, app, &mut context)) {
                app.finish_manual_refresh();
                return Err(anyhow::anyhow!(error.to_string()));
            }
            let refresh = app.refresh();
            app.finish_manual_refresh();
            refresh?;
            app.toast(ToastVariant::Success, "Refreshed from disk");
            return Ok(DispatchOutcome::Continue.with_effect(Effect::Redraw));
        }
        Action::Background(action) => {
            return Ok(apply_background_action(app, action));
        }

        Action::Browser(action) => {
            if let Some(outcome) = handlers::browser(terminal, app, action)? {
                return Ok(outcome);
            }
        }
        Action::Search(action) => handlers::search(app, action),
        Action::Editor(action) => handlers::editor(app, action)?,
        Action::Metadata(action) => {
            if let Some(outcome) = handlers::metadata(terminal, app, action)? {
                return Ok(outcome);
            }
        }
        Action::Location(action) => {
            if let Some(outcome) = handlers::location(app, action)? {
                return Ok(outcome);
            }
        }
        Action::Settings(action) => handlers::settings(terminal, app, action)?,
        Action::Images(action) => {
            if let Some(outcome) = handlers::images(terminal, app, action)? {
                return Ok(outcome);
            }
        }
        Action::Overlay(action) => handlers::overlay(app, action)?,
        Action::Reader(action) => handlers::reader(app, action),
        Action::Insights(action) => handlers::insights(app, action),
    }

    // One-shot compose (`notema log` with no body) quits as soon as its editor
    // closes — whether the entry was saved or discarded.
    if app.compose && app.editor.is_none() {
        return Ok(DispatchOutcome::Quit);
    }

    Ok(DispatchOutcome::Continue)
}

fn apply_background_action(app: &mut AppModel, action: BackgroundAction) -> DispatchOutcome {
    let mut outcome = DispatchOutcome {
        control: ControlFlow::Continue,
        redraw: false,
        effects: Vec::new(),
    };
    outcome.redraw = match action {
        BackgroundAction::LibraryValidated(snapshot) => {
            app.install_library_snapshot(*snapshot);
            true
        }
        BackgroundAction::LibraryValidationStale => {
            match app
                .services
                .store
                .load_library(notema_storage::CachePolicy::Normal)
            {
                Ok(snapshot) => app.install_library_snapshot(snapshot),
                Err(error) => {
                    app.finish_initial_library_loading();
                    app.toast(
                        ToastVariant::Error,
                        format!("Journal changes not loaded: {error:#}"),
                    );
                }
            }
            true
        }
        BackgroundAction::LibraryValidationFailed(error) => {
            app.finish_initial_library_loading();
            app.toast(
                ToastVariant::Error,
                format!("Journal changes not loaded: {error}"),
            );
            true
        }
        BackgroundAction::ExternalOpenCompleted(message) => {
            app.toast(ToastVariant::Info, message);
            true
        }
        BackgroundAction::ExternalOpenFailed(error) => {
            app.toast(ToastVariant::Error, format!("Couldn't open link: {error}"));
            true
        }
        BackgroundAction::PollImages => app.image.runtime.poll_results(),
        BackgroundAction::PollGeocode => {
            // Fold dialog results and write back any address-backfill results, then
            // pace out the next reverse lookup over the shared geocode worker.
            let changed = app.apply_geocode_results();
            if let Some(request) = app.prepare_address_backfill() {
                outcome.effects.push(Effect::Geocode(request));
            }
            changed
        }
        BackgroundAction::PollEnvironment => {
            let changed = app.apply_environment_results();
            if let Some(request) = app.prepare_environment_backfill() {
                outcome.effects.push(Effect::Environment(request));
            }
            changed
        }
        BackgroundAction::PollTimers => {
            let toasts_expired = app.expire_toasts();
            let flash_expired = app.expire_reader_heading_flash();
            let environment_saved = poll_fetching_environment(app);
            toasts_expired || flash_expired || environment_saved
        }
        BackgroundAction::LibraryPathsChanged(paths) => {
            if let Err(error) = app.refresh_paths(&paths) {
                app.toast(
                    ToastVariant::Error,
                    format!("Journal changes not reloaded: {error}"),
                );
            }
            true
        }
        BackgroundAction::ReloadTheme(name) => {
            let path = crate::tui::theme::themes_dir(&app.services.config_path)
                .join(format!("{name}.toml"));
            match crate::tui::theme::load_file(&path, app.appearance.mode()) {
                Ok(reloaded) => app.appearance.theme = app.appearance.resolve(reloaded),
                Err(error) => app.toast(
                    ToastVariant::Error,
                    format!(
                        "Theme not reloaded: {name}.toml: {}",
                        crate::tui::concise_error(&error)
                    ),
                ),
            }
            true
        }
        BackgroundAction::CommitSearch => {
            app.update_search_results();
            true
        }
    };
    outcome
}

struct ReaderSnapshot {
    id: String,
    focus: Focus,
    fullscreen: bool,
    reader_scroll: u16,
}

impl ReaderSnapshot {
    fn capture(app: &AppModel) -> Option<Self> {
        let target = app.selected_entry_target()?;
        Some(Self {
            id: target.id,
            focus: app.nav.focus,
            fullscreen: app.nav.reader_fullscreen,
            reader_scroll: app.nav.scroll.reader,
        })
    }

    fn restore(self, app: &mut AppModel) -> bool {
        if !app.select_entry_by_id(&self.id, false) {
            return false;
        }
        app.nav.focus = self.focus;
        app.nav.reader_fullscreen = self.fullscreen;
        app.nav.scroll.reader = self.reader_scroll;
        true
    }
}

fn restore_reader_or_close(app: &mut AppModel, snapshot: Option<ReaderSnapshot>) {
    let Some(snapshot) = snapshot else {
        return;
    };
    let was_in_viewer = snapshot.focus == Focus::Reader;
    if !snapshot.restore(app) && was_in_viewer {
        app.nav.focus = Focus::Entries;
        app.nav.scroll.reset_reader();
    }
}

/// Apply an edit-overlay change to the selected entry, then restore the entry
/// view (the reload reorders entries) and close the overlay.
fn commit_entry_edit(
    app: &mut AppModel,
    edit: impl FnOnce(&mut AppModel) -> AppResult<()>,
) -> AppResult<()> {
    let snapshot = ReaderSnapshot::capture(app);
    edit(app)?;
    restore_reader_or_close(app, snapshot);
    app.close_overlay();
    Ok(())
}

/// Route a metadata-dialog save to the open editor's buffer (closing the dialog),
/// or commit it to the selected entry when no editor is open.
fn edit_or_commit(
    app: &mut AppModel,
    to_editor: impl FnOnce(&mut AppModel),
    to_entry: impl FnOnce(&mut AppModel) -> AppResult<()>,
) -> AppResult<()> {
    if app.editor.is_some() {
        to_editor(app);
        app.close_overlay();
        Ok(())
    } else {
        commit_entry_edit(app, to_entry)
    }
}

fn edit_or_commit_location(
    app: &mut AppModel,
    location: Option<notema_domain::Location>,
    osm_zone: Option<String>,
) -> AppResult<Option<crate::tui::environment::EnvironmentRequest>> {
    if app.editor.is_some() {
        let request = app.set_editor_location(location, osm_zone);
        app.close_overlay();
        Ok(request)
    } else {
        let snapshot = ReaderSnapshot::capture(app);
        let request = set_location_on_entry(app, location)?;
        restore_reader_or_close(app, snapshot);
        app.close_overlay();
        Ok(request)
    }
}

fn set_editor_prompt(app: &mut AppModel, prompt: EditorPrompt) {
    if let Some(editor) = app.editor.as_mut() {
        editor.prompt = prompt;
    }
}

/// Point whichever confirm dialog is open at `yes` (the destructive button).
fn set_confirm_selection(app: &mut AppModel, yes: bool) {
    if let Overlay::ConfirmDelete(_, selected) = &mut app.overlay {
        *selected = yes;
    } else if let Some(EditorPrompt::ConfirmDiscard { discard_selected }) =
        app.editor.as_mut().map(|editor| &mut editor.prompt)
    {
        *discard_selected = yes;
    }
}

fn request_editor_discard(app: &mut AppModel) {
    if app.editor.as_ref().is_some_and(|editor| editor.is_dirty()) {
        // Default the selection to Keep so a stray Enter never discards.
        set_editor_prompt(
            app,
            EditorPrompt::ConfirmDiscard {
                discard_selected: false,
            },
        );
    } else {
        app.cancel_editor();
    }
}

/// Scroll the global help cheatsheet, clamping at the top; the draw clamps the
/// bottom against the rendered height it alone knows.
fn scroll_help(app: &mut AppModel, delta: i16) {
    if let Overlay::Help { scroll } = &mut app.overlay {
        *scroll = scroll.saturating_add_signed(delta);
    }
}

fn scroll_editor_help(app: &mut AppModel, delta: i16) {
    if let Some(EditorPrompt::Help { scroll }) =
        app.editor.as_mut().map(|editor| &mut editor.prompt)
    {
        *scroll = scroll.saturating_add_signed(delta);
    }
}

fn start_editor_selection(app: &mut AppModel, col: u16, row: u16) {
    let Some(editor) = app.editor.as_mut() else {
        return;
    };
    if let Some((row, col)) = editor.text_pos_at(col, row) {
        editor.textarea.cancel_selection();
        editor.textarea.move_cursor(CursorMove::Jump(row, col));
        editor.textarea.start_selection();
        editor.mouse_selecting = true;
    }
}

fn select_editor_word(app: &mut AppModel, col: u16, row: u16) {
    let Some(editor) = app.editor.as_mut() else {
        return;
    };
    let Some((line, column)) = editor.text_pos_at(col, row) else {
        return;
    };
    let Some((start, end)) = crate::tui::text_input::word_bounds(
        &editor.textarea.lines()[line as usize],
        column as usize,
    ) else {
        return;
    };
    // Anchor at the word's end and leave the caret at its start: while selecting,
    // the caret cell is painted with the reversed selection style, so keeping it on
    // a selected char avoids highlighting the boundary cell after the word.
    editor.textarea.cancel_selection();
    editor
        .textarea
        .move_cursor(CursorMove::Jump(line, end as u16));
    editor.textarea.start_selection();
    editor
        .textarea
        .move_cursor(CursorMove::Jump(line, start as u16));
}

fn drag_editor_selection(app: &mut AppModel, col: u16, row: u16) {
    let Some(editor) = app.editor.as_mut() else {
        return;
    };
    if !editor.mouse_selecting {
        return;
    }

    let rect = editor.text_rect;
    if rect.height > 0 {
        let margin = (rect.height as i32 / 2).min(2);
        let top = rect.y as i32;
        let bottom = (rect.y + rect.height - 1) as i32;
        let row_i32 = row as i32;
        if row_i32 < top + margin {
            editor.scroll_lines(-((top + margin - row_i32).min(4) as i16));
        } else if row_i32 > bottom - margin {
            editor.scroll_lines((row_i32 - (bottom - margin)).min(4) as i16);
        }
    }

    let col = col.clamp(rect.x, rect.x + rect.width.saturating_sub(1));
    let row = row.clamp(rect.y, rect.y + rect.height.saturating_sub(1));
    if let Some((row, col)) = editor.text_pos_at(col, row) {
        editor.textarea.move_cursor(CursorMove::Jump(row, col));
    }
}

fn end_editor_selection(app: &mut AppModel) {
    let Some(editor) = app.editor.as_mut() else {
        return;
    };
    editor.mouse_selecting = false;
    let empty = editor
        .textarea
        .selection_range()
        .is_none_or(|(start, end)| start == end);
    if empty {
        editor.textarea.cancel_selection();
    }
}

fn confirm_delete(app: &mut AppModel) -> AppResult<()> {
    let is_journal = matches!(
        &app.overlay,
        Overlay::ConfirmDelete(crate::tui::state::DeleteContext::Journal { .. }, _)
    );
    if is_journal {
        delete_selected_journal(app)?;
    } else {
        delete_selected(app)?;
    }
    app.close_overlay();
    app.nav.focus = if is_journal {
        Focus::Journals
    } else {
        Focus::Entries
    };
    app.nav.scroll.reset_reader();
    app.refresh()
}

pub(crate) fn terminal_area<B: Backend>(terminal: &mut Terminal<B>) -> AppResult<Rect> {
    let size = terminal
        .size()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(Rect::new(0, 0, size.width, size.height))
}

/// The list viewport height of whichever edit dialog is open, needed to keep the
/// selection visible after a navigation. Only one edit dialog is open at a time,
/// so the first matching state wins.
fn open_dialog_list_height<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &AppModel,
) -> AppResult<u16> {
    let area = terminal_area(terminal)?;
    let height = if let Some(state) = app.edit_metadata_state() {
        render::metadata_dialog_layout(&app.appearance.theme, area, state.filtered.len())
            .list
            .height
    } else if let Some(state) = app.edit_feeling_state() {
        render::feelings_dialog_layout(
            &app.appearance.theme,
            area,
            state.item_count(),
            &state.selected,
        )
        .list
        .height
    } else if let Some(state) = app.edit_location_state() {
        render::location_dialog_layout(&app.appearance.theme, area, &state.list_labels())
            .list
            .height
    } else if let Some(state) = app.theme_picker_state() {
        render::theme_picker_layout(
            &app.appearance.theme,
            area,
            state.entries.len(),
            state.hint_state(),
        )
        .list
        .height
    } else {
        0
    };
    Ok(height)
}

/// The open edit dialog's list, as a shared navigation handle.
fn open_dialog_list_mut(app: &mut AppModel) -> Option<&mut dyn ListNav> {
    if app.edit_metadata_state().is_some() {
        return app.edit_metadata_state_mut().map(|s| s as &mut dyn ListNav);
    }
    if app.edit_feeling_state().is_some() {
        return app.edit_feeling_state_mut().map(|s| s as &mut dyn ListNav);
    }
    if app.edit_location_state().is_some() {
        return app.edit_location_state_mut().map(|s| s as &mut dyn ListNav);
    }
    if app.theme_picker_state().is_some() {
        return app.theme_picker_state_mut().map(|s| s as &mut dyn ListNav);
    }
    None
}

/// Move within the open dialog's list, then scroll so the selection stays visible.
fn navigate_open_dialog<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    nav: impl FnOnce(&mut dyn ListNav),
) -> AppResult<()> {
    let list_height = open_dialog_list_height(terminal, app)?;
    if let Some(list) = open_dialog_list_mut(app) {
        nav(list);
        list.ensure_selected_visible(list_height);
    }
    Ok(())
}

/// Scroll a just-opened dialog's list so its initial selection is on screen. A
/// dialog can open with the cursor well below the top — the theme picker seeds it
/// on the active theme — and the offset defaults to zero, so without this the
/// selection would sit off-screen until the first keypress.
fn reveal_open_dialog_selection<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
) -> AppResult<()> {
    let list_height = open_dialog_list_height(terminal, app)?;
    if let Some(list) = open_dialog_list_mut(app) {
        list.ensure_selected_visible(list_height);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
