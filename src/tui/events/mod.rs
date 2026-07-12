mod action;
mod actions;
mod keyboard;
mod mouse;

use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::{
    AppResult,
    tui::{
        app::{App, Focus, reader_is_available},
        editor_state::{EditorPrompt, EditorTarget},
        render,
        state::{ListNav, Overlay, ToastVariant},
    },
};
use ratatui_textarea::CursorMove;

use action::{Action, InsightsAction, ReaderAction};
use actions::{
    delete_selected, delete_selected_journal, open_reader_link, save_internal_editor,
    set_feelings_on_entry, set_location_on_entry, set_metadata_on_entry, set_mood_on_entry,
    submit_new_journal, toggle_archive_selected_journal, toggle_starred_on_entry, view_selected,
};
use keyboard::{keep_selection_visible, move_focus_left, move_focus_right};

pub(crate) use keyboard::handle_key;
pub(crate) use mouse::{fold_leading_wheel, handle_mouse, handle_scroll, is_wheel, update_hover};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DispatchOutcome {
    Continue,
    Quit,
}

impl DispatchOutcome {
    pub(crate) const fn should_quit(self) -> bool {
        matches!(self, Self::Quit)
    }
}

/// How long the "Fetching weather and air quality…" modal waits before giving up
/// and saving without the data.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Drive the [`Overlay::FetchingEnvironment`] modal: once the editor's background
/// fetch lands (or the timeout fires) close it and re-run the deferred save.
/// Returns whether it acted, so the event loop knows to repaint. No-op when the
/// modal isn't open.
pub(crate) fn poll_fetching_environment(app: &mut App) -> AppResult<bool> {
    let Overlay::FetchingEnvironment(started) = app.overlay else {
        return Ok(false);
    };
    let landed = app
        .editor
        .as_ref()
        .is_none_or(|editor| editor.pending_environment.is_none());
    let timed_out = started.elapsed() >= FETCH_TIMEOUT;
    if !(landed || timed_out) {
        return Ok(false);
    }
    // Timed out with nothing yet: give up waiting so the save proceeds bare.
    if timed_out && let Some(editor) = app.editor.as_mut() {
        editor.pending_environment = None;
    }
    app.close_overlay();
    if let Err(error) = save_internal_editor(app) {
        report_action_error(app, &error);
    }
    Ok(true)
}

pub(crate) fn dispatch_action(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    action: Action,
) -> AppResult<DispatchOutcome> {
    let result = apply_action(terminal, app, action);
    recover_action_error(app, result)
}

fn recover_action_error(
    app: &mut App,
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

fn report_action_error(app: &mut App, error: &anyhow::Error) {
    let detail = error.to_string();
    let first_line = detail.lines().next().unwrap_or("Unknown error");
    let mut concise: String = first_line.chars().take(120).collect();
    if first_line.chars().count() > 120 {
        concise.push('…');
    }
    app.toast(ToastVariant::Error, format!("Action failed: {concise}"));
}

fn apply_action(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    action: Action,
) -> AppResult<DispatchOutcome> {
    use crate::tui::app::EditMetadataFocus;

    match action {
        Action::PointerInput { event, area } => {
            return mouse::apply_pointer(terminal, app, event, area);
        }
        Action::PointerScroll { event, area, delta } => {
            mouse::apply_scroll(app, event, area, delta);
        }
        Action::PointerHover { column, row, area } => {
            mouse::apply_hover(app, column, row, area);
        }
        Action::Quit => return Ok(DispatchOutcome::Quit),

        Action::FocusLeft => move_focus_left(app),
        Action::FocusRight => {
            let available = reader_is_available(terminal.size()?.width);
            move_focus_right(app, available);
        }
        Action::MoveSelection(delta) => {
            app.move_selection(delta);
            keep_selection_visible(terminal, app)?;
        }

        Action::Reader(action) => match action {
            ReaderAction::ScrollLines(delta) => app.scroll_reader(delta),
            ReaderAction::ScrollPages(delta) => app.page_reader(delta),
            ReaderAction::ScrollToStart => app.nav.scroll.reader = 0,
            ReaderAction::ScrollToEnd => app.nav.scroll.reader = u16::MAX,
            ReaderAction::SetFullscreen(fullscreen) => app.nav.reader_fullscreen = fullscreen,
        },
        Action::Insights(action) => match action {
            InsightsAction::ScrollLines(delta) => app.scroll_insights(delta),
            InsightsAction::ScrollPages(delta) => app.page_insights(delta),
            InsightsAction::ScrollToStart => app.nav.scroll.insights = 0,
            InsightsAction::ScrollToEnd => app.nav.scroll.insights = u16::MAX,
            InsightsAction::SetFullscreen(fullscreen) => {
                app.nav.insights_fullscreen = fullscreen;
            }
            InsightsAction::ToggleScope => {
                app.nav.insights_scope = app.nav.insights_scope.toggle();
                app.nav.scroll.reset_insights();
            }
            InsightsAction::CycleTimeframe => {
                app.nav.insights_timeframe = app.nav.insights_timeframe.next();
                app.nav.scroll.reset_insights();
            }
        },

        Action::BeginSearch => {
            app.begin_search();
        }
        Action::ExitSearch => {
            app.exit_search();
        }
        Action::EditSelected => app.open_editor_for_selected(),
        Action::EditorSave => {
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
        }
        Action::EditorRequestDiscard => request_editor_discard(app),
        Action::EditorDiscard => app.cancel_editor(),
        Action::EditorToggleFullscreen => {
            app.nav.reader_fullscreen = !app.nav.reader_fullscreen;
        }
        Action::EditorOpenMetadataMenu => set_editor_prompt(app, EditorPrompt::MetadataMenu),
        Action::EditorOpenHelp => set_editor_prompt(app, EditorPrompt::Help { scroll: 0 }),
        Action::EditorClosePrompt => set_editor_prompt(app, EditorPrompt::None),
        Action::EditorScrollHelp(delta) => scroll_editor_help(app, delta),
        Action::EditorInput(key) => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.input(key);
            }
        }
        Action::EditorSelectAll => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.select_all();
            }
        }
        Action::EditorScroll(delta) => {
            if let Some(editor) = app.editor.as_mut() {
                editor.scroll_lines(delta);
            }
        }
        Action::EditorStartSelection { col, row } => start_editor_selection(app, col, row),
        Action::EditorDragSelection { col, row } => drag_editor_selection(app, col, row),
        Action::EditorEndSelection => end_editor_selection(app),
        Action::ViewSelected => view_selected(app)?,
        Action::OpenReaderLink(target) => open_reader_link(app, &target)?,
        Action::BeginDelete => app.begin_confirm_delete(),
        Action::ConfirmDelete => confirm_delete(app)?,
        Action::CancelOverlay => {
            if app.has_overlay() {
                if matches!(app.overlay, Overlay::NewJournal(_)) {
                    app.toast(ToastVariant::Info, "Cancelled");
                }
                app.close_overlay();
            }
        }
        Action::OpenMetadataMenu => app.open_metadata_menu(),
        Action::BeginEditMetadata(kind) => {
            set_editor_prompt(app, EditorPrompt::None);
            match kind {
                crate::tui::state::MetadataKind::Tags => app.begin_edit_tags(),
                crate::tui::state::MetadataKind::People => app.begin_edit_people(),
                crate::tui::state::MetadataKind::Activities => app.begin_edit_activities(),
            }
            reveal_open_dialog_selection(terminal, app)?;
        }
        Action::BeginEditFeelings => {
            set_editor_prompt(app, EditorPrompt::None);
            // No open-scroll to the selection here: feelings groups are collapsible,
            // so there's no stable single row to reveal.
            app.begin_edit_feelings();
        }
        Action::BeginEditMood => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_mood();
        }
        Action::ToggleStarred => commit_entry_edit(app, toggle_starred_on_entry)?,
        Action::NewEntry => app.open_editor_for_new(),
        Action::NewJournal => app.begin_new_journal_input(),
        Action::ToggleArchiveJournal => {
            toggle_archive_selected_journal(app)?;
            keep_selection_visible(terminal, app)?;
        }

        Action::JournalInputSubmit => submit_new_journal(app)?,

        Action::InputKey(key) => app.handle_text_input_key(key),
        Action::InputSelectAll => {
            if let Some(input) = app.focused_text_input_mut() {
                input.select_all();
            }
        }

        Action::MoveDialogSelection(delta) => {
            let theme_picker = matches!(app.overlay, Overlay::ThemePicker(_));
            navigate_open_dialog(terminal, app, |list| {
                if delta < 0 {
                    list.move_up();
                } else if delta > 0 {
                    list.move_down();
                }
            })?;
            if theme_picker {
                app.theme_picker_preview();
            }
        }
        Action::MetadataToggle => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.toggle_selected();
            }
        }
        Action::MetadataSwitchFocus => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.focus = match state.focus {
                    EditMetadataFocus::List => EditMetadataFocus::Input,
                    EditMetadataFocus::Input => EditMetadataFocus::List,
                };
            }
        }
        Action::MetadataAddFromInput => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.add_from_input();
            }
        }
        Action::MetadataSave => {
            let Some((kind, tags)) = app
                .edit_metadata_state()
                .map(|s| (s.kind, s.selected.clone()))
            else {
                return Ok(DispatchOutcome::Continue);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_metadata(kind, &tags),
                |app| set_metadata_on_entry(app, kind, &tags),
            )?;
        }

        Action::FeelingsToggle => {
            let list_height = open_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.toggle_selected();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsExpand => {
            let list_height = open_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.expand_selected();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsCollapse => {
            let list_height = open_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.collapse_selected();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsSwitchFocus => {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.switch_focus();
            }
        }
        Action::FeelingsSave => {
            let Some(feelings) = app.edit_feeling_state().map(|s| s.selected.clone()) else {
                return Ok(DispatchOutcome::Continue);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_feelings(&feelings),
                |app| set_feelings_on_entry(app, &feelings),
            )?;
        }

        Action::AdjustMood(delta) => {
            if let Some(state) = app.edit_mood_state_mut() {
                state.draft = state.draft.saturating_add(delta).clamp(-5, 5);
            }
        }
        Action::MoodSave => {
            let Some(mood) = app.edit_mood_state().map(|s| s.draft) else {
                return Ok(DispatchOutcome::Continue);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_mood(Some(mood)),
                |app| set_mood_on_entry(app, Some(mood)),
            )?;
        }
        Action::MoodClear => {
            let saved = app.edit_mood_state().and_then(|s| s.saved);
            edit_or_commit(
                app,
                |app| app.set_editor_mood(None),
                |app| {
                    if saved.is_some() {
                        set_mood_on_entry(app, None)?;
                    }
                    Ok(())
                },
            )?;
        }

        Action::BeginEditLocation => {
            // No open-scroll here: the dialog opens focused on the query field, so
            // its preset list draws no selection to reveal.
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_location();
        }
        Action::LocationSwitchFocus => {
            if let Some(state) = app.edit_location_state_mut() {
                state.switch_focus();
            }
        }
        Action::LocationResolve => app.resolve_location_query(),
        Action::LocationGrabDevice => app.grab_device_location(),
        Action::LocationSelectRow => {
            if let Some(state) = app.edit_location_state_mut() {
                state.select_row();
            }
            let Some(location) = app.edit_location_state().map(|state| state.composed()) else {
                return Ok(DispatchOutcome::Continue);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_location(location.clone()),
                |app| set_location_on_entry(app, location.clone()),
            )?;
        }
        Action::LocationSave => {
            let Some(location) = app.edit_location_state().map(|state| state.composed()) else {
                return Ok(DispatchOutcome::Continue);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_location(location.clone()),
                |app| set_location_on_entry(app, location.clone()),
            )?;
        }
        Action::LocationClear => {
            edit_or_commit(
                app,
                |app| app.set_editor_location(None),
                |app| set_location_on_entry(app, None),
            )?;
        }

        Action::OpenSettingsMenu => app.open_settings_menu(),
        Action::OpenThemePicker => {
            app.open_theme_picker();
            reveal_open_dialog_selection(terminal, app)?;
        }
        Action::ThemePickerSelect(index) => app.theme_picker_select(index),
        Action::ThemePickerConfirm => app.theme_picker_confirm(),
        Action::ThemePickerCancel => app.theme_picker_cancel(),
        Action::ThemePickerCycleChrome => app.theme_picker_cycle_chrome(),
        Action::ThemePickerCycleMode => app.theme_picker_cycle_mode(),

        Action::OpenImageViewer(index) => app.begin_image_viewer(index),
        Action::StepImageViewer(delta) => app.image_viewer_step(delta),

        Action::ToggleHints => {
            app.state.ui.show_hints = !app.state.ui.show_hints;
            crate::config::save_state(&app.config_path, &app.state)?;
        }

        Action::ToggleJournals => {
            app.state.ui.show_journals = !app.state.ui.show_journals;
            if app.state.ui.show_journals {
                // Focus the column so narrow/medium layouts actually reveal it.
                app.nav.focus = Focus::Journals;
            } else if app.nav.focus == Focus::Journals {
                // Don't leave focus on a now-hidden pane.
                app.nav.focus = Focus::Entries;
            }
            crate::config::save_state(&app.config_path, &app.state)?;
        }
    }

    // One-shot compose (`notema log` with no body) quits as soon as its editor
    // closes — whether the entry was saved or discarded.
    if app.compose && app.editor.is_none() {
        return Ok(DispatchOutcome::Quit);
    }

    Ok(DispatchOutcome::Continue)
}

struct ReaderSnapshot {
    id: String,
    focus: Focus,
    fullscreen: bool,
    reader_scroll: u16,
}

impl ReaderSnapshot {
    fn capture(app: &App) -> Option<Self> {
        let target = app.selected_entry_target()?;
        Some(Self {
            id: target.id,
            focus: app.nav.focus,
            fullscreen: app.nav.reader_fullscreen,
            reader_scroll: app.nav.scroll.reader,
        })
    }

    fn restore(self, app: &mut App) -> bool {
        if !app.select_entry_by_id(&self.id, false) {
            return false;
        }
        app.nav.focus = self.focus;
        app.nav.reader_fullscreen = self.fullscreen;
        app.nav.scroll.reader = self.reader_scroll;
        true
    }
}

fn restore_reader_or_close(app: &mut App, snapshot: Option<ReaderSnapshot>) {
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
fn commit_entry_edit(app: &mut App, edit: impl FnOnce(&mut App) -> AppResult<()>) -> AppResult<()> {
    let snapshot = ReaderSnapshot::capture(app);
    edit(app)?;
    restore_reader_or_close(app, snapshot);
    app.close_overlay();
    Ok(())
}

/// Route a metadata-dialog save to the open editor's buffer (closing the dialog),
/// or commit it to the selected entry when no editor is open.
fn edit_or_commit(
    app: &mut App,
    to_editor: impl FnOnce(&mut App),
    to_entry: impl FnOnce(&mut App) -> AppResult<()>,
) -> AppResult<()> {
    if app.editor.is_some() {
        to_editor(app);
        app.close_overlay();
        Ok(())
    } else {
        commit_entry_edit(app, to_entry)
    }
}

fn set_editor_prompt(app: &mut App, prompt: EditorPrompt) {
    if let Some(editor) = app.editor.as_mut() {
        editor.prompt = prompt;
    }
}

fn request_editor_discard(app: &mut App) {
    if app.editor.as_ref().is_some_and(|editor| editor.is_dirty()) {
        set_editor_prompt(app, EditorPrompt::ConfirmDiscard);
    } else {
        app.cancel_editor();
    }
}

fn scroll_editor_help(app: &mut App, delta: i16) {
    if let Some(EditorPrompt::Help { scroll }) =
        app.editor.as_mut().map(|editor| &mut editor.prompt)
    {
        *scroll = scroll.saturating_add_signed(delta);
    }
}

fn start_editor_selection(app: &mut App, col: u16, row: u16) {
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

fn drag_editor_selection(app: &mut App, col: u16, row: u16) {
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

fn end_editor_selection(app: &mut App) {
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

fn confirm_delete(app: &mut App) -> AppResult<()> {
    let is_journal = matches!(
        &app.overlay,
        Overlay::ConfirmDelete(crate::tui::state::DeleteContext::Journal { .. })
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

pub(crate) fn terminal_area(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> AppResult<Rect> {
    let size = terminal.size()?;
    Ok(Rect::new(0, 0, size.width, size.height))
}

/// The list viewport height of whichever edit dialog is open, needed to keep the
/// selection visible after a navigation. Only one edit dialog is open at a time,
/// so the first matching state wins.
fn open_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let area = terminal_area(terminal)?;
    let height = if let Some(state) = app.edit_metadata_state() {
        render::metadata_dialog_layout(area, state.filtered.len())
            .list
            .height
    } else if let Some(state) = app.edit_feeling_state() {
        render::feelings_dialog_layout(area, state.item_count(), &state.selected)
            .list
            .height
    } else if let Some(state) = app.edit_location_state() {
        render::location_dialog_layout(area, &state.list_labels())
            .list
            .height
    } else if let Some(state) = app.theme_picker_state() {
        render::theme_picker_layout(area, state.entries.len(), state.mode_switchable())
            .list
            .height
    } else {
        0
    };
    Ok(height)
}

/// The open edit dialog's list, as a shared navigation handle.
fn open_dialog_list_mut(app: &mut App) -> Option<&mut dyn ListNav> {
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
fn navigate_open_dialog(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
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
fn reveal_open_dialog_selection(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let list_height = open_dialog_list_height(terminal, app)?;
    if let Some(list) = open_dialog_list_mut(app) {
        list.ensure_selected_visible(list_height);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
