mod action;
mod actions;
mod keyboard;
mod mouse;
mod terminal;

use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::{
    AppResult,
    tui::{
        app::{App, Focus, entry_view_is_available},
        editor_state::{EditorPrompt, EditorTarget},
        render,
        state::{ListNav, Overlay},
    },
};
use ratatui_textarea::CursorMove;

use action::Action;
use actions::{
    create_entry_in_selected_journal, delete_selected, delete_selected_journal, edit_selected,
    save_internal_editor, set_feelings_on_entry, set_location_on_entry, set_metadata_on_entry,
    set_mood_on_entry, submit_new_journal, toggle_archive_selected_journal,
    toggle_starred_on_entry, view_selected,
};
use keyboard::{keep_selection_visible, move_focus_left, move_focus_right};

pub(crate) use keyboard::handle_key;
pub(crate) use mouse::{fold_leading_wheel, handle_mouse, handle_scroll, is_wheel};

pub(crate) fn dispatch_action(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    action: Action,
) -> AppResult<bool> {
    use crate::tui::state::EditMetadataFocus;

    match action {
        Action::Quit => return Ok(true),

        Action::FocusLeft => move_focus_left(app),
        Action::FocusRight => {
            let available = entry_view_is_available(terminal.size()?.width);
            move_focus_right(app, available);
        }
        Action::MoveUp => {
            app.move_selection(-1);
            keep_selection_visible(terminal, app)?;
        }
        Action::MoveDown => {
            app.move_selection(1);
            keep_selection_visible(terminal, app)?;
        }

        Action::ScrollEntryView(delta) => app.scroll_entry_view(delta),
        Action::PageEntryView(delta) => app.page_entry_view(delta),
        Action::ScrollEntryViewToStart => app.nav.scroll.entry_view = 0,
        Action::ScrollEntryViewToEnd => app.nav.scroll.entry_view = u16::MAX,

        Action::ScrollInsights(delta) => app.scroll_insights(delta),
        Action::PageInsights(delta) => app.page_insights(delta),
        Action::ScrollInsightsToStart => app.nav.scroll.insights = 0,
        Action::ScrollInsightsToEnd => app.nav.scroll.insights = u16::MAX,

        Action::BeginSearch => {
            app.begin_search();
        }
        Action::ExitSearch => {
            app.exit_search();
        }
        Action::EditSelected => {
            if app.config.editor.is_internal() {
                app.open_editor_for_selected();
            } else {
                let snapshot = EntryViewSnapshot::capture(app);
                edit_selected(terminal, app)?;
                restore_entry_view_or_close(app, snapshot);
            }
        }
        Action::EditorSave => {
            let restore_existing = matches!(
                app.editor.as_ref().map(|editor| &editor.target),
                Some(EditorTarget::Existing { .. })
            );
            let snapshot = restore_existing
                .then(|| EntryViewSnapshot::capture(app))
                .flatten();
            save_internal_editor(app)?;
            if restore_existing && app.editor.is_none() {
                restore_entry_view_or_close(app, snapshot);
            }
        }
        Action::EditorRequestDiscard => request_editor_discard(app),
        Action::EditorDiscard => app.cancel_editor(),
        Action::EditorToggleFullscreen => {
            app.nav.entry_view_fullscreen = !app.nav.entry_view_fullscreen;
        }
        Action::EditorOpenMetadataMenu => set_editor_prompt(app, EditorPrompt::MetadataMenu),
        Action::EditorOpenHelp => set_editor_prompt(app, EditorPrompt::Help { scroll: 0 }),
        Action::EditorClosePrompt => set_editor_prompt(app, EditorPrompt::None),
        Action::EditorScrollHelp(delta) => scroll_editor_help(app, delta),
        Action::EditorBeginMetadata(kind) => {
            set_editor_prompt(app, EditorPrompt::None);
            match kind {
                crate::tui::state::MetadataKind::Tags => app.begin_edit_tags(),
                crate::tui::state::MetadataKind::People => app.begin_edit_people(),
                crate::tui::state::MetadataKind::Activities => app.begin_edit_activities(),
            }
        }
        Action::EditorInput(key) => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.input(key);
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
        Action::ExpandEntryView => app.nav.entry_view_fullscreen = true,
        Action::CollapseEntryView => app.nav.entry_view_fullscreen = false,
        Action::ExpandInsights => app.nav.insights_fullscreen = true,
        Action::CollapseInsights => app.nav.insights_fullscreen = false,
        Action::BeginDelete => app.begin_confirm_delete(),
        Action::ConfirmDelete => confirm_delete(app)?,
        Action::CancelOverlay => {
            if app.has_overlay() {
                if matches!(app.overlay, Overlay::NewJournal(_)) {
                    app.set_status("Cancelled");
                }
                app.close_overlay();
            }
        }
        Action::OpenMetadataMenu => app.open_metadata_menu(),
        Action::BeginEditTags => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_tags();
        }
        Action::BeginEditPeople => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_people();
        }
        Action::BeginEditActivities => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_activities();
        }
        Action::BeginEditFeelings => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_feelings();
        }
        Action::BeginEditMood => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_mood();
        }
        Action::ToggleStarred => commit_entry_edit(app, toggle_starred_on_entry)?,
        Action::NewEntry => {
            if app.config.editor.is_internal() {
                app.open_editor_for_new();
            } else {
                let snapshot = EntryViewSnapshot::capture(app);
                let restore_to_viewer = snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.focus == Focus::EntryView);
                let created = create_entry_in_selected_journal(terminal, app)?;
                if restore_to_viewer {
                    let created_id = created.as_deref().and_then(journal_storage::entry_id);
                    if let Some(id) = created_id {
                        if app.select_entry_by_id(&id, true) {
                            app.nav.focus = Focus::EntryView;
                        } else {
                            restore_entry_view_or_close(app, snapshot);
                        }
                    } else {
                        restore_entry_view_or_close(app, snapshot);
                    }
                }
            }
        }
        Action::NewJournal => app.begin_new_journal_input(),
        Action::ToggleInsightsScope => {
            app.nav.insights_scope = app.nav.insights_scope.toggle();
            app.nav.scroll.reset_insights();
        }
        Action::CycleInsightsTimeframe => {
            app.nav.insights_timeframe = app.nav.insights_timeframe.next();
            app.nav.scroll.reset_insights();
        }
        Action::ToggleArchiveJournal => {
            toggle_archive_selected_journal(app)?;
            keep_selection_visible(terminal, app)?;
        }

        Action::JournalInputChar(ch) => {
            if let Some(input) = app.new_journal_input_mut() {
                input.push(ch);
            }
        }
        Action::JournalInputBackspace => {
            if let Some(input) = app.new_journal_input_mut() {
                input.pop();
            }
        }
        Action::JournalInputSubmit => submit_new_journal(app)?,

        Action::MetadataMoveUp | Action::FeelingsMoveUp | Action::LocationMoveUp => {
            navigate_open_dialog(terminal, app, |list| list.move_up())?;
        }
        Action::MetadataMoveDown | Action::FeelingsMoveDown | Action::LocationMoveDown => {
            navigate_open_dialog(terminal, app, |list| list.move_down())?;
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
        Action::MetadataInput(ch) => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.input.push(ch);
                state.rebuild_filter();
            }
        }
        Action::MetadataBackspace => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.input.pop();
                state.rebuild_filter();
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
                return Ok(false);
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
        Action::FeelingsInput(ch) => {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.input.push(ch);
                state.rebuild_filter();
            }
        }
        Action::FeelingsBackspace => {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.input.pop();
                state.rebuild_filter();
            }
        }
        Action::FeelingsSave => {
            let Some(feelings) = app.edit_feeling_state().map(|s| s.selected.clone()) else {
                return Ok(false);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_feelings(&feelings),
                |app| set_feelings_on_entry(app, &feelings),
            )?;
        }

        Action::MoodDecrease => {
            if let Some(state) = app.edit_mood_state_mut()
                && state.draft > -5
            {
                state.draft -= 1;
            }
        }
        Action::MoodIncrease => {
            if let Some(state) = app.edit_mood_state_mut()
                && state.draft < 5
            {
                state.draft += 1;
            }
        }
        Action::MoodSave => {
            let Some(mood) = app.edit_mood_state().map(|s| s.draft) else {
                return Ok(false);
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
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_location();
        }
        Action::LocationSwitchFocus => {
            if let Some(state) = app.edit_location_state_mut() {
                state.switch_focus();
            }
        }
        Action::LocationInput(ch) => {
            if let Some(state) = app.edit_location_state_mut() {
                state.input_char(ch);
            }
        }
        Action::LocationBackspace => {
            if let Some(state) = app.edit_location_state_mut() {
                state.backspace();
            }
        }
        Action::LocationResolve => app.resolve_location_query(),
        Action::LocationGrabDevice => app.grab_device_location(),
        Action::LocationSelectRow => {
            if let Some(state) = app.edit_location_state_mut() {
                state.select_row();
            }
            let Some(location) = app.edit_location_state().map(|state| state.composed()) else {
                return Ok(false);
            };
            commit_entry_edit(app, |app| set_location_on_entry(app, location.clone()))?;
        }
        Action::LocationSave => {
            let Some(location) = app.edit_location_state().map(|state| state.composed()) else {
                return Ok(false);
            };
            commit_entry_edit(app, |app| set_location_on_entry(app, location.clone()))?;
        }
        Action::LocationClear => {
            commit_entry_edit(app, |app| set_location_on_entry(app, None))?;
        }

        Action::OpenImageViewer(index) => app.begin_image_viewer(index),
        Action::ImageViewerNext => app.image_viewer_step(1),
        Action::ImageViewerPrev => app.image_viewer_step(-1),

        Action::SearchInput(ch) => app.search_insert(ch),
        Action::SearchBackspace => app.search_backspace(),
        Action::SearchCursorLeft => app.search_cursor_left(),
        Action::SearchCursorRight => app.search_cursor_right(),

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

    Ok(false)
}

struct EntryViewSnapshot {
    id: String,
    focus: Focus,
    fullscreen: bool,
    entry_view_scroll: u16,
}

impl EntryViewSnapshot {
    fn capture(app: &App) -> Option<Self> {
        let target = app.selected_entry_target()?;
        Some(Self {
            id: target.id,
            focus: app.nav.focus,
            fullscreen: app.nav.entry_view_fullscreen,
            entry_view_scroll: app.nav.scroll.entry_view,
        })
    }

    fn restore(self, app: &mut App) -> bool {
        if !app.select_entry_by_id(&self.id, false) {
            return false;
        }
        app.nav.focus = self.focus;
        app.nav.entry_view_fullscreen = self.fullscreen;
        app.nav.scroll.entry_view = self.entry_view_scroll;
        true
    }
}

fn restore_entry_view_or_close(app: &mut App, snapshot: Option<EntryViewSnapshot>) {
    let Some(snapshot) = snapshot else {
        return;
    };
    let was_in_viewer = snapshot.focus == Focus::EntryView;
    if !snapshot.restore(app) && was_in_viewer {
        app.nav.focus = Focus::Entries;
        app.nav.scroll.reset_entry_view();
    }
}

/// Apply an edit-overlay change to the selected entry, then restore the entry
/// view (the reload reorders entries) and close the overlay.
fn commit_entry_edit(app: &mut App, edit: impl FnOnce(&mut App) -> AppResult<()>) -> AppResult<()> {
    let snapshot = EntryViewSnapshot::capture(app);
    edit(app)?;
    restore_entry_view_or_close(app, snapshot);
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
    app.nav.scroll.reset_entry_view();
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
        render::feelings_dialog_layout(
            area,
            state.item_count(),
            render::feelings_selected_line_count(&state.selected),
        )
        .list
        .height
    } else if let Some(state) = app.edit_location_state() {
        render::location_dialog_layout(area, render::location_list_rows(&state.list_labels()))
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

#[cfg(test)]
mod tests;
