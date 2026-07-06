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
    view_selected,
};
use keyboard::{keep_selection_visible, move_focus_left, move_focus_right};

pub(crate) use keyboard::handle_key;
pub(crate) use mouse::handle_mouse;

pub(crate) fn dispatch_action(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    action: Action,
) -> AppResult<bool> {
    use crate::tui::state::EditTagFocus;

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

        Action::TagsMoveUp => {
            let list_height = tag_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_tag_state_mut() {
                state.move_up();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::TagsMoveDown => {
            let list_height = tag_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_tag_state_mut() {
                state.move_down();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::TagsToggle => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.toggle_selected();
            }
        }
        Action::TagsSwitchFocus => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.focus = match state.focus {
                    EditTagFocus::List => EditTagFocus::Input,
                    EditTagFocus::Input => EditTagFocus::List,
                };
            }
        }
        Action::TagsInput(ch) => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.input.push(ch);
                state.rebuild_filter();
            }
        }
        Action::TagsBackspace => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.input.pop();
                state.rebuild_filter();
            }
        }
        Action::TagsAddFromInput => {
            if let Some(state) = app.edit_tag_state_mut() {
                let tag = state.input.trim().to_lowercase();
                if !tag.is_empty() && !state.selected.contains(&tag) {
                    state.selected.push(tag.clone());
                    if !state
                        .all_tags
                        .iter()
                        .any(|(t, _)| t.eq_ignore_ascii_case(&tag))
                    {
                        state.all_tags.push((tag, 0));
                    }
                }
                state.input.clear();
                state.rebuild_filter();
            }
        }
        Action::TagsSave => {
            let snapshot = EntryViewSnapshot::capture(app);
            let tags = app
                .edit_tag_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            let kind = app
                .edit_tag_state()
                .map(|s| s.kind)
                .unwrap_or(crate::tui::state::MetadataKind::Tags);
            set_metadata_on_entry(app, kind, &tags)?;
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
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
            if let Some(state) = app.edit_feeling_state_mut() {
                state.toggle_selected();
            }
        }
        Action::FeelingsSave => {
            let snapshot = EntryViewSnapshot::capture(app);
            let feelings = app
                .edit_feeling_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            set_feelings_on_entry(app, &feelings)?;
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
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
            let snapshot = EntryViewSnapshot::capture(app);
            let mood = app.edit_mood_state().map(|s| s.draft);
            set_mood_on_entry(app, mood)?;
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
        }
        Action::MoodClear => {
            let snapshot = EntryViewSnapshot::capture(app);
            let mood = app.edit_mood_state().and_then(|s| s.saved);
            if mood.is_some() {
                set_mood_on_entry(app, None)?;
            }
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
        }

        Action::OpenImageViewer(index) => app.begin_image_viewer(index),
        Action::ImageViewerNext => app.image_viewer_step(1),
        Action::ImageViewerPrev => app.image_viewer_step(-1),

        Action::SearchInput(ch) => app.search_insert(ch),
        Action::SearchBackspace => app.search_backspace(),
        Action::SearchCursorLeft => app.search_cursor_left(),
        Action::SearchCursorRight => app.search_cursor_right(),

        Action::ToggleHints => {
            app.config.show_hints = !app.config.show_hints;
            crate::config::save_config(&app.config_path, &app.config)?;
        }

        Action::ToggleJournals => {
            app.config.show_journals = !app.config.show_journals;
            if app.config.show_journals {
                // Focus the column so narrow/medium layouts actually reveal it.
                app.nav.focus = Focus::Journals;
            } else if app.nav.focus == Focus::Journals {
                // Don't leave focus on a now-hidden pane.
                app.nav.focus = Focus::Entries;
            }
            crate::config::save_config(&app.config_path, &app.config)?;
        }
    }

    Ok(false)
}

struct EntryViewSnapshot {
    id: String,
    focus: Focus,
    entry_view_scroll: u16,
}

impl EntryViewSnapshot {
    fn capture(app: &App) -> Option<Self> {
        let target = app.selected_entry_target()?;
        Some(Self {
            id: target.id,
            focus: app.nav.focus,
            entry_view_scroll: app.nav.scroll.entry_view,
        })
    }

    fn restore(self, app: &mut App) -> bool {
        if !app.select_entry_by_id(&self.id, false) {
            return false;
        }
        app.nav.focus = self.focus;
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

fn terminal_area(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<Rect> {
    let size = terminal.size()?;
    Ok(Rect::new(0, 0, size.width, size.height))
}

fn tag_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let filtered_len = app.edit_tag_state().map_or(0, |state| state.filtered.len());
    Ok(
        render::tags_dialog_layout(terminal_area(terminal)?, filtered_len)
            .list
            .height,
    )
}

fn feelings_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let all_len = app
        .edit_feeling_state()
        .map_or(0, |state| state.all_feelings.len());
    Ok(
        render::feelings_dialog_layout(terminal_area(terminal)?, all_len)
            .list
            .height,
    )
}

#[cfg(test)]
mod tests;
