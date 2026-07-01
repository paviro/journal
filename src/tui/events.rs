use crate::{
    AppResult,
    markdown::split_front_matter,
    storage::{create_entry, create_journal, move_entry_to_trash, open_editor, set_updated_at_now},
};
use crossterm::{
    event::{KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{fs, io};

use super::app::{App, Focus, MarkdownView, Mode, preview_is_visible};

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    let preview_visible = preview_is_visible(terminal.size()?.width);
    app.normalize_focus(preview_visible);

    if app.viewer.is_some() {
        handle_viewer_key(app, key);
        return Ok(false);
    }

    if app.new_journal_input.is_some() {
        handle_new_journal_input(app, key)?;
        return Ok(false);
    }

    if app.confirm_delete {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                delete_selected(app)?;
                app.confirm_delete = false;
                app.refresh()?;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.confirm_delete = false,
            _ => {}
        }
        return Ok(false);
    }

    if app.mode == Mode::Search {
        handle_search_key(terminal, app, key, preview_visible)?;
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('r') => app.refresh()?,
        KeyCode::Char('/') => app.begin_search(),
        KeyCode::Left => {
            app.focus = match app.focus {
                Focus::Preview => Focus::Entries,
                Focus::Entries => Focus::Journals,
                Focus::Journals => Focus::Journals,
            };
        }
        KeyCode::Right => {
            app.focus = match app.focus {
                Focus::Journals => Focus::Entries,
                Focus::Entries if preview_visible => Focus::Preview,
                Focus::Entries => Focus::Entries,
                Focus::Preview => Focus::Preview,
            };
        }
        KeyCode::Tab => {
            app.focus = next_focus(app.focus, preview_visible);
        }
        KeyCode::Up if app.focus == Focus::Preview => app.scroll_preview(-1),
        KeyCode::Down if app.focus == Focus::Preview => app.scroll_preview(1),
        KeyCode::Char('k') if app.focus == Focus::Preview => app.scroll_preview(-1),
        KeyCode::Char('j') if app.focus == Focus::Preview => app.scroll_preview(1),
        KeyCode::PageUp if app.focus == Focus::Preview => app.page_preview(-1),
        KeyCode::PageDown if app.focus == Focus::Preview => app.page_preview(1),
        KeyCode::Home if app.focus == Focus::Preview => app.preview_scroll = 0,
        KeyCode::End if app.focus == Focus::Preview => app.preview_scroll = u16::MAX,
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Enter | KeyCode::Char('e') if app.can_act_on_selected_entry() => {
            edit_selected(terminal, app)?
        }
        KeyCode::Char('v') if app.can_act_on_selected_entry() => view_selected(app)?,
        KeyCode::Char('n') => create_entry_in_selected_journal(terminal, app)?,
        KeyCode::Char('j') | KeyCode::Char('J') => app.begin_new_journal_input(),
        KeyCode::Char('d') if app.can_act_on_selected_entry() => app.confirm_delete = true,
        _ => {}
    }

    Ok(false)
}

fn handle_search_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
    preview_visible: bool,
) -> AppResult<()> {
    match key.code {
        KeyCode::Esc => app.exit_search(),
        KeyCode::Left if app.focus == Focus::Preview => app.focus = Focus::Entries,
        KeyCode::Right | KeyCode::Tab if app.focus == Focus::Entries && preview_visible => {
            app.focus = Focus::Preview;
        }
        KeyCode::Tab if app.focus == Focus::Preview => app.focus = Focus::Entries,
        KeyCode::Up if app.focus == Focus::Preview => app.scroll_preview(-1),
        KeyCode::Down if app.focus == Focus::Preview => app.scroll_preview(1),
        KeyCode::Char('k') if app.focus == Focus::Preview => app.scroll_preview(-1),
        KeyCode::Char('j') if app.focus == Focus::Preview => app.scroll_preview(1),
        KeyCode::PageUp if app.focus == Focus::Preview => app.page_preview(-1),
        KeyCode::PageDown if app.focus == Focus::Preview => app.page_preview(1),
        KeyCode::Home if app.focus == Focus::Preview => app.preview_scroll = 0,
        KeyCode::End if app.focus == Focus::Preview => app.preview_scroll = u16::MAX,
        KeyCode::Enter if app.can_act_on_selected_entry() => edit_selected(terminal, app)?,
        KeyCode::Char('e') if app.focus == Focus::Preview && app.has_selected_entry_target() => {
            edit_selected(terminal, app)?
        }
        KeyCode::Char('v') if app.focus == Focus::Preview && app.has_selected_entry_target() => {
            view_selected(app)?
        }
        KeyCode::Char('d') if app.focus == Focus::Preview && app.has_selected_entry_target() => {
            app.confirm_delete = true
        }
        KeyCode::Backspace if app.focus == Focus::Entries => {
            app.search_query.pop();
            app.update_search_results()?;
        }
        KeyCode::Char(ch) if app.focus == Focus::Entries => {
            app.search_query.push(ch);
            app.update_search_results()?;
        }
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        _ => {}
    }

    Ok(())
}

fn next_focus(focus: Focus, preview_visible: bool) -> Focus {
    match (focus, preview_visible) {
        (Focus::Journals, _) => Focus::Entries,
        (Focus::Entries, true) => Focus::Preview,
        (Focus::Entries, false) => Focus::Journals,
        (Focus::Preview, _) => Focus::Journals,
    }
}

fn handle_viewer_key(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
        app.viewer = None;
        return;
    }

    let Some(viewer) = app.viewer.as_mut() else {
        return;
    };

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            viewer.scroll = viewer.scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            viewer.scroll = viewer.scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            viewer.scroll = viewer.scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            viewer.scroll = viewer.scroll.saturating_add(10);
        }
        KeyCode::Home => viewer.scroll = 0,
        KeyCode::End => viewer.scroll = u16::MAX,
        _ => {}
    }
}

fn handle_new_journal_input(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Esc => {
            app.new_journal_input = None;
            app.set_status("Cancelled");
        }
        KeyCode::Enter => submit_new_journal(app)?,
        KeyCode::Backspace => {
            if let Some(input) = app.new_journal_input.as_mut() {
                input.pop();
            }
        }
        KeyCode::Char(ch) => {
            if let Some(input) = app.new_journal_input.as_mut() {
                input.push(ch);
            }
        }
        _ => {}
    }
    Ok(())
}

fn submit_new_journal(app: &mut App) -> AppResult<()> {
    let value = app
        .new_journal_input
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    if value.is_empty() {
        app.set_status("Nothing added");
        app.new_journal_input = None;
        return Ok(());
    }

    let journal = create_journal(&app.config.journal_root, &value)?;
    app.refresh()?;
    app.select_journal_by_name(&journal.name);
    app.set_status(format!("Created journal {}", journal.name));
    app.new_journal_input = None;
    Ok(())
}

fn create_entry_in_selected_journal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    if app.selected_journal().is_some() {
        new_entry(terminal, app)
    } else {
        app.set_status("Create a journal first with j");
        Ok(())
    }
}

fn new_entry(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let Some(journal) = app.selected_journal().cloned() else {
        app.set_status("No journal selected");
        return Ok(());
    };

    let root = app.config.journal_root.clone();
    let editor = app.config.editor.clone();
    let journal_name = journal.name;
    suspend_terminal(terminal, || create_entry(&root, &journal_name, &editor))?;
    app.set_status("Entry saved");
    app.refresh()?;
    Ok(())
}

fn edit_selected(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    let editor = app.config.editor.clone();
    suspend_terminal(terminal, || open_editor(&editor, &target.path))?;
    set_updated_at_now(&target.path)?;
    app.set_status(format!("Edited {}", target.path.display()));
    app.refresh()?;
    Ok(())
}

fn view_selected(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    let content = fs::read_to_string(&target.path)?;
    let (_, body) = split_front_matter(&content);
    app.viewer = Some(MarkdownView {
        title: target.title,
        content: body.trim_start().to_string(),
        scroll: 0,
    });
    Ok(())
}

fn delete_selected(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };
    move_entry_to_trash(&app.config.journal_root, &target.path)?;

    app.set_status("Moved to trash");
    Ok(())
}

fn suspend_terminal<T>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    action: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    let result = action();
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    enable_raw_mode()?;
    terminal.clear()?;
    result
}
