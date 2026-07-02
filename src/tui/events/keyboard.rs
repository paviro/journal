use crate::AppResult;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available},
    render,
};

use super::actions::{
    create_entry_in_selected_journal, delete_selected, edit_selected, set_feelings_on_entry,
    set_mood_on_entry, set_tags_on_entry, submit_new_journal, view_selected,
};

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    let width = terminal.size()?.width;
    let entry_view_available = entry_view_is_available(width);
    app.normalize_focus(entry_view_available);

    if app.entry_view_expanded {
        if handle_expanded_entry_key(terminal, app, key)? {
            return Ok(true);
        }
        return Ok(false);
    }

    if app.new_journal_input().is_some() {
        handle_new_journal_input(app, key)?;
        return Ok(false);
    }

    if app.is_confirming_delete() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                delete_selected(app)?;
                app.close_overlay();
                app.refresh()?;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.close_overlay(),
            _ => {}
        }
        return Ok(false);
    }

    if app.edit_tag_state().is_some() {
        handle_edit_tags_key(app, key)?;
        return Ok(false);
    }

    if app.edit_feeling_state().is_some() {
        handle_edit_feelings_key(app, key)?;
        return Ok(false);
    }

    if app.edit_mood_state().is_some() {
        handle_edit_mood_key(app, key)?;
        return Ok(false);
    }

    if app.mode == Mode::Search {
        handle_search_key(terminal, app, key, entry_view_available)?;
        return Ok(false);
    }

    if handle_entry_view_scroll(app, key.code) {
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('r') => app.refresh()?,
        KeyCode::Char('/') => app.begin_search(),
        KeyCode::Left => move_focus_left(app),
        KeyCode::Right => handle_right(app, entry_view_available)?,
        KeyCode::Enter => handle_enter(app, entry_view_available)?,
        KeyCode::Up => move_selection_visible(terminal, app, -1)?,
        KeyCode::Down => move_selection_visible(terminal, app, 1)?,
        KeyCode::Char('e') if app.can_act_on_selected_entry() => edit_selected(terminal, app)?,

        KeyCode::Char('n') => {
            if app.focus == Focus::Journals {
                app.begin_new_journal_input();
            } else {
                create_entry_in_selected_journal(terminal, app)?;
            }
        }
        KeyCode::Char('d') if app.can_act_on_selected_entry() => app.begin_confirm_delete(),
        KeyCode::Char('t') if app.can_act_on_selected_entry() => app.begin_edit_tags(),
        KeyCode::Char('f') if app.can_act_on_selected_entry() => app.begin_edit_feelings(),
        KeyCode::Char('m') if app.can_act_on_selected_entry() => app.begin_edit_mood(),
        _ => {}
    }

    Ok(false)
}

fn handle_edit_mood_key(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Esc => {
            app.close_overlay();
        }
        KeyCode::Enter => {
            let mood = app.edit_mood_state().map(|s| s.draft);
            set_mood_on_entry(app, mood)?;
            app.close_overlay();
        }
        KeyCode::Delete | KeyCode::Backspace => {
            let mood = app.edit_mood_state().and_then(|s| s.saved);
            if mood.is_some() {
                set_mood_on_entry(app, None)?;
            }
            app.close_overlay();
        }
        KeyCode::Left => {
            if let Some(state) = app.edit_mood_state_mut()
                && state.draft > -5
            {
                state.draft -= 1;
            }
        }
        KeyCode::Right => {
            if let Some(state) = app.edit_mood_state_mut()
                && state.draft < 5
            {
                state.draft += 1;
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_edit_feelings_key(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Esc => {
            app.close_overlay();
        }
        KeyCode::Enter => {
            let feelings = app
                .edit_feeling_state()
                .map(|state| state.selected.clone())
                .unwrap_or_default();
            set_feelings_on_entry(app, &feelings)?;
            app.close_overlay();
        }
        KeyCode::Up => {
            if let Some(state) = app.edit_feeling_state_mut()
                && state.cursor > 0
            {
                state.cursor -= 1;
            }
        }
        KeyCode::Down => {
            if let Some(state) = app.edit_feeling_state_mut()
                && state.cursor + 1 < state.all_feelings.len()
            {
                state.cursor += 1;
            }
        }
        KeyCode::Char(' ') => {
            if let Some(state) = app.edit_feeling_state_mut() {
                let feeling = state.all_feelings[state.cursor].clone();
                if let Some(pos) = state.selected.iter().position(|value| value == &feeling) {
                    state.selected.remove(pos);
                } else {
                    state.selected.push(feeling);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_search_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
    entry_view_available: bool,
) -> AppResult<()> {
    if handle_entry_view_scroll(app, key.code) {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => app.exit_search(),
        KeyCode::Left if app.focus == Focus::EntryView => app.focus = Focus::Entries,
        KeyCode::Right
            if app.focus == Focus::Entries
                && !entry_view_available
                && app.has_selected_entry_target() =>
        {
            view_selected(app)?
        }
        KeyCode::Right if app.focus == Focus::Entries && entry_view_available => {
            app.focus = Focus::EntryView;
        }
        KeyCode::Enter if app.can_act_on_selected_entry() => view_selected(app)?,
        KeyCode::Char('e') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            edit_selected(terminal, app)?
        }

        KeyCode::Char('d') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            app.begin_confirm_delete()
        }
        KeyCode::Backspace if app.focus == Focus::Entries => {
            app.search.query.pop();
            app.update_search_results();
        }
        KeyCode::Char(ch) if app.focus == Focus::Entries => {
            app.search.query.push(ch);
            app.update_search_results();
        }
        KeyCode::Up => move_selection_visible(terminal, app, -1)?,
        KeyCode::Down => move_selection_visible(terminal, app, 1)?,
        _ => {}
    }

    Ok(())
}

/// Handle EntryView scroll keys, returning `true` when the key was consumed.
fn handle_entry_view_scroll(app: &mut App, key: KeyCode) -> bool {
    if app.focus != Focus::EntryView {
        return false;
    }
    match key {
        KeyCode::Up | KeyCode::Char('k') => app.scroll_entry_view(-1),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_entry_view(1),
        KeyCode::PageUp => app.page_entry_view(-1),
        KeyCode::PageDown => app.page_entry_view(1),
        KeyCode::Home => app.scroll.entry_view = 0,
        KeyCode::End => app.scroll.entry_view = u16::MAX,
        _ => return false,
    }
    true
}

fn move_selection_visible(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    delta: isize,
) -> AppResult<()> {
    app.move_selection(delta);
    keep_selection_visible(terminal, app)
}

fn keep_selection_visible(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let size = terminal.size()?;
    let layout = render::tui_layout(Rect::new(0, 0, size.width, size.height), app);
    if app.focus == Focus::Journals && app.mode == Mode::Browse {
        if let Some(area) = layout.journals {
            render::ensure_index_visible(
                &mut app.scroll.journal,
                app.selected_journal,
                app.journals.len(),
                render::panel_inner(area).height,
            );
        }
    } else if let Some(area) = layout.entries {
        let text_width = area.width.saturating_sub(11);
        let rows = render::entry_row_metadata(app, text_width);
        render::ensure_entry_visible(
            &mut app.scroll.entry,
            &rows,
            app.selected_entry_index,
            render::panel_inner(area).height,
        );
    }

    Ok(())
}

fn move_focus_left(app: &mut App) {
    app.focus = match app.focus {
        Focus::EntryView => Focus::Entries,
        Focus::Entries => Focus::Journals,
        Focus::Journals => Focus::Journals,
    };
}

pub(super) fn handle_right(app: &mut App, entry_view_available: bool) -> AppResult<()> {
    if app.focus == Focus::Entries && !entry_view_available && app.has_selected_entry_target() {
        view_selected(app)?;
    } else {
        move_focus_right(app, entry_view_available);
    }

    Ok(())
}

pub(super) fn move_focus_right(app: &mut App, entry_view_available: bool) {
    app.focus = match app.focus {
        Focus::Journals => Focus::Entries,
        Focus::Entries if entry_view_available => Focus::EntryView,
        Focus::Entries => Focus::Entries,
        Focus::EntryView => Focus::EntryView,
    };
}

pub(super) fn handle_enter(app: &mut App, entry_view_available: bool) -> AppResult<()> {
    if app.focus == Focus::Journals {
        move_focus_right(app, entry_view_available);
    } else if app.can_act_on_selected_entry() {
        view_selected(app)?;
    }

    Ok(())
}

fn handle_expanded_entry_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Left => {
            app.entry_view_expanded = false;
            app.focus = Focus::Entries;
        }
        KeyCode::Up | KeyCode::Char('k') => app.scroll_entry_view(-1),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_entry_view(1),
        KeyCode::PageUp => app.page_entry_view(-1),
        KeyCode::PageDown => app.page_entry_view(1),
        KeyCode::Home => app.scroll.entry_view = 0,
        KeyCode::End => app.scroll.entry_view = u16::MAX,
        KeyCode::Char('e') if app.has_selected_entry_target() => {
            edit_selected(terminal, app)?;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_new_journal_input(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Esc => {
            app.close_overlay();
            app.set_status("Cancelled");
        }
        KeyCode::Enter => submit_new_journal(app)?,
        KeyCode::Backspace => {
            if let Some(input) = app.new_journal_input_mut() {
                input.pop();
            }
        }
        KeyCode::Char(ch) => {
            if let Some(input) = app.new_journal_input_mut() {
                input.push(ch);
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_edit_tags_key(app: &mut App, key: KeyEvent) -> AppResult<()> {
    use crate::tui::state::EditTagFocus;

    match key.code {
        KeyCode::Esc => {
            app.close_overlay();
        }
        KeyCode::Tab => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.focus = match state.focus {
                    EditTagFocus::List => EditTagFocus::Input,
                    EditTagFocus::Input => EditTagFocus::List,
                };
            }
        }
        KeyCode::Enter
            if app
                .edit_tag_state()
                .is_some_and(|s| s.focus == EditTagFocus::List) =>
        {
            let tags: Vec<String> = app
                .edit_tag_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            set_tags_on_entry(app, &tags)?;
            app.close_overlay();
        }
        KeyCode::Enter => {
            // Input mode — add typed tag to selection
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
        KeyCode::Up
            if app
                .edit_tag_state()
                .is_some_and(|s| s.focus == EditTagFocus::List) =>
        {
            if let Some(state) = app.edit_tag_state_mut()
                && state.cursor > 0
            {
                state.cursor -= 1;
            }
        }
        KeyCode::Down
            if app
                .edit_tag_state()
                .is_some_and(|s| s.focus == EditTagFocus::List) =>
        {
            if let Some(state) = app.edit_tag_state_mut()
                && state.cursor + 1 < state.filtered.len()
            {
                state.cursor += 1;
            }
        }
        KeyCode::Char(' ')
            if app
                .edit_tag_state()
                .is_some_and(|s| s.focus == EditTagFocus::List) =>
        {
            if let Some(state) = app.edit_tag_state_mut() {
                let tag_idx = state.filtered[state.cursor];
                let tag = state.all_tags[tag_idx].0.to_lowercase();
                if let Some(pos) = state.selected.iter().position(|t| t == &tag) {
                    state.selected.remove(pos);
                } else {
                    state.selected.push(tag);
                }
            }
        }
        KeyCode::Backspace
            if app
                .edit_tag_state()
                .is_some_and(|s| s.focus == EditTagFocus::Input) =>
        {
            if let Some(state) = app.edit_tag_state_mut() {
                state.input.pop();
                state.rebuild_filter();
            }
        }
        KeyCode::Char(ch)
            if app
                .edit_tag_state()
                .is_some_and(|s| s.focus == EditTagFocus::Input) =>
        {
            if let Some(state) = app.edit_tag_state_mut() {
                state.input.push(ch);
                state.rebuild_filter();
            }
        }
        _ => {}
    }
    Ok(())
}
