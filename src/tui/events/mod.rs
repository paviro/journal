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
        render,
        state::{ListNav, Overlay},
    },
};

use action::Action;
use actions::{
    create_entry_in_selected_journal, delete_selected, delete_selected_journal, edit_selected,
    set_feelings_on_entry, set_metadata_on_entry, set_mood_on_entry, submit_new_journal,
    toggle_archive_selected_journal, toggle_starred_on_entry, view_selected,
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
            let snapshot = EntryViewSnapshot::capture(app);
            edit_selected(terminal, app)?;
            restore_entry_view_or_close(app, snapshot);
        }
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
        Action::BeginEditTags => app.begin_edit_tags(),
        Action::BeginEditPeople => app.begin_edit_people(),
        Action::BeginEditActivities => app.begin_edit_activities(),
        Action::BeginEditFeelings => app.begin_edit_feelings(),
        Action::BeginEditMood => app.begin_edit_mood(),
        Action::ToggleStarred => commit_entry_edit(app, toggle_starred_on_entry)?,
        Action::NewEntry => {
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

        Action::MetadataMoveUp => {
            let list_height = metadata_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_metadata_state_mut() {
                state.move_up();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::MetadataMoveDown => {
            let list_height = metadata_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_metadata_state_mut() {
                state.move_down();
                state.ensure_selected_visible(list_height);
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
                let tag = state.input.trim().to_lowercase();
                if !tag.is_empty() && !state.selected.contains(&tag) {
                    state.selected.push(tag.clone());
                    if !state
                        .all_values
                        .iter()
                        .any(|(t, _)| t.eq_ignore_ascii_case(&tag))
                    {
                        state.all_values.push((tag, 0));
                    }
                }
                state.input.clear();
                state.rebuild_filter();
            }
        }
        Action::MetadataSave => {
            let Some((kind, tags)) = app
                .edit_metadata_state()
                .map(|s| (s.kind, s.selected.clone()))
            else {
                return Ok(false);
            };
            commit_entry_edit(app, |app| set_metadata_on_entry(app, kind, &tags))?;
        }

        Action::FeelingsMoveUp => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.move_up();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsMoveDown => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.move_down();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsToggle => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.toggle_selected();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsExpand => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.expand_selected();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsCollapse => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
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
            commit_entry_edit(app, |app| set_feelings_on_entry(app, &feelings))?;
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
            commit_entry_edit(app, |app| set_mood_on_entry(app, Some(mood)))?;
        }
        Action::MoodClear => {
            let saved = app.edit_mood_state().and_then(|s| s.saved);
            commit_entry_edit(app, |app| {
                if saved.is_some() {
                    set_mood_on_entry(app, None)?;
                }
                Ok(())
            })?;
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

fn metadata_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let filtered_len = app
        .edit_metadata_state()
        .map_or(0, |state| state.filtered.len());
    Ok(
        render::metadata_dialog_layout(terminal_area(terminal)?, filtered_len)
            .list
            .height,
    )
}

fn feelings_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let (all_len, selected_lines) = app.edit_feeling_state().map_or((0, 1), |state| {
        (
            state.item_count(),
            render::feelings_selected_line_count(&state.selected),
        )
    });
    Ok(
        render::feelings_dialog_layout(terminal_area(terminal)?, all_len, selected_lines)
            .list
            .height,
    )
}

#[cfg(test)]
mod tests;
