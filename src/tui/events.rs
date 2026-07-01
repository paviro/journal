use crate::{
    AppResult,
    markdown::split_front_matter,
    storage::{
        create_entry, create_journal, move_entry_to_trash, open_editor, search_all,
        set_updated_at_now,
    },
};
use crossterm::{
    event::{KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{fs, io};

use super::app::{App, Focus, MarkdownView, Mode};

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
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
        handle_search_key(terminal, app, key)?;
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('r') => app.refresh()?,
        KeyCode::Char('/') => {
            app.mode = Mode::Search;
            app.focus = Focus::Items;
            app.search_query.clear();
            app.search_hits.clear();
            app.selected_item = 0;
            app.preview_scroll = 0;
        }
        KeyCode::Left => {
            app.focus = match app.focus {
                Focus::Preview => Focus::Items,
                Focus::Items => Focus::Journals,
                Focus::Journals => Focus::Journals,
            };
        }
        KeyCode::Right => {
            app.focus = match app.focus {
                Focus::Journals => Focus::Items,
                Focus::Items => Focus::Preview,
                Focus::Preview => Focus::Preview,
            };
        }
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::Journals => Focus::Items,
                Focus::Items => Focus::Preview,
                Focus::Preview => Focus::Journals,
            };
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
        KeyCode::Enter | KeyCode::Char('e') => edit_selected(terminal, app)?,
        KeyCode::Char('v') => view_selected(app)?,
        KeyCode::Char('n') => new_item(terminal, app)?,
        KeyCode::Char('j') | KeyCode::Char('J') => app.begin_new_journal_input(),
        KeyCode::Char('d') => app.confirm_delete = true,
        _ => {}
    }

    Ok(false)
}

fn handle_search_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<()> {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Browse;
            app.search_query.clear();
            app.search_hits.clear();
            app.selected_item = 0;
            app.preview_scroll = 0;
        }
        KeyCode::Left if app.focus == Focus::Preview => app.focus = Focus::Items,
        KeyCode::Right | KeyCode::Tab if app.focus == Focus::Items => {
            app.focus = Focus::Preview;
        }
        KeyCode::Up if app.focus == Focus::Preview => app.scroll_preview(-1),
        KeyCode::Down if app.focus == Focus::Preview => app.scroll_preview(1),
        KeyCode::Char('k') if app.focus == Focus::Preview => app.scroll_preview(-1),
        KeyCode::Char('j') if app.focus == Focus::Preview => app.scroll_preview(1),
        KeyCode::PageUp if app.focus == Focus::Preview => app.page_preview(-1),
        KeyCode::PageDown if app.focus == Focus::Preview => app.page_preview(1),
        KeyCode::Home if app.focus == Focus::Preview => app.preview_scroll = 0,
        KeyCode::End if app.focus == Focus::Preview => app.preview_scroll = u16::MAX,
        KeyCode::Enter | KeyCode::Char('e') => edit_selected(terminal, app)?,
        KeyCode::Char('v') => view_selected(app)?,
        KeyCode::Backspace => {
            app.search_query.pop();
            app.search_hits = search_all(&app.config.journal_root, &app.search_query)?;
            app.selected_item = 0;
            app.preview_scroll = 0;
        }
        KeyCode::Char(ch) => {
            app.search_query.push(ch);
            app.search_hits = search_all(&app.config.journal_root, &app.search_query)?;
            app.selected_item = 0;
            app.preview_scroll = 0;
        }
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        _ => {}
    }

    Ok(())
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

fn new_item(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> AppResult<()> {
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
    let Some(path) = app.selected_markdown_path() else {
        return Ok(());
    };

    let editor = app.config.editor.clone();
    suspend_terminal(terminal, || open_editor(&editor, &path))?;
    set_updated_at_now(&path)?;
    app.set_status(format!("Edited {}", path.display()));
    app.refresh()?;
    Ok(())
}

fn view_selected(app: &mut App) -> AppResult<()> {
    let Some(path) = app.selected_markdown_path() else {
        return Ok(());
    };

    let content = fs::read_to_string(&path)?;
    let (_, body) = split_front_matter(&content);
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Markdown")
        .to_string();
    app.viewer = Some(MarkdownView {
        title,
        content: body.trim_start().to_string(),
        scroll: 0,
    });
    Ok(())
}

fn delete_selected(app: &mut App) -> AppResult<()> {
    match app.mode {
        Mode::Search => {
            let Some(hit) = app.selected_search_hit() else {
                return Ok(());
            };
            move_entry_to_trash(&app.config.journal_root, &hit.path)?;
        }
        Mode::Browse => {
            let Some(path) = app.selected_entry_path() else {
                return Ok(());
            };
            move_entry_to_trash(&app.config.journal_root, &path)?;
        }
    }

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
