use crate::AppResult;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available},
    render,
};

use super::actions::{
    create_entry_in_selected_journal, delete_selected, edit_selected, submit_new_journal,
    view_selected,
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
        _ => {}
    }

    Ok(false)
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
        let rows = render::entry_row_metadata(app);
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
