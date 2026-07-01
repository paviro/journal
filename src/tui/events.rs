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
    let width = terminal.size()?.width;
    let preview_visible = preview_is_visible(width);
    app.normalize_focus(preview_visible);

    if app.viewer.is_some() {
        handle_viewer_key(terminal, app, key, preview_visible)?;
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
        KeyCode::Left => move_focus_left(app),
        KeyCode::Right => handle_right(app, preview_visible)?,
        KeyCode::Enter => handle_enter(app, preview_visible)?,
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
        KeyCode::Char('e') if app.can_act_on_selected_entry() => edit_selected(terminal, app)?,
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
        KeyCode::Right
            if app.focus == Focus::Entries
                && !preview_visible
                && app.has_selected_entry_target() =>
        {
            view_selected(app)?
        }
        KeyCode::Right if app.focus == Focus::Entries && preview_visible => {
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
        KeyCode::Enter if app.can_act_on_selected_entry() => view_selected(app)?,
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

fn move_focus_left(app: &mut App) {
    app.focus = match app.focus {
        Focus::Preview => Focus::Entries,
        Focus::Entries => Focus::Journals,
        Focus::Journals => Focus::Journals,
    };
}

fn handle_right(app: &mut App, preview_visible: bool) -> AppResult<()> {
    if app.focus == Focus::Entries && !preview_visible && app.has_selected_entry_target() {
        view_selected(app)?;
    } else {
        move_focus_right(app, preview_visible);
    }

    Ok(())
}

fn move_focus_right(app: &mut App, preview_available: bool) {
    app.focus = match app.focus {
        Focus::Journals => Focus::Entries,
        Focus::Entries if preview_available => Focus::Preview,
        Focus::Entries => Focus::Entries,
        Focus::Preview => Focus::Preview,
    };
}

fn handle_enter(app: &mut App, preview_available: bool) -> AppResult<()> {
    if app.focus == Focus::Journals {
        move_focus_right(app, preview_available);
    } else if app.can_act_on_selected_entry() {
        view_selected(app)?;
    }

    Ok(())
}

fn handle_viewer_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
    preview_visible: bool,
) -> AppResult<()> {
    if viewer_key_closes(key.code, preview_visible) {
        app.viewer = None;
        return Ok(());
    }

    if matches!(key.code, KeyCode::Char('e')) {
        edit_viewer_entry(terminal, app)?;
        return Ok(());
    }

    let Some(viewer) = app.viewer.as_mut() else {
        return Ok(());
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

    Ok(())
}

fn viewer_key_closes(key: KeyCode, preview_visible: bool) -> bool {
    matches!(key, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q'))
        || (key == KeyCode::Left && !preview_visible)
}

fn edit_viewer_entry(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let Some(viewer) = app.viewer.as_ref() else {
        return Ok(());
    };

    let path = viewer.path.clone();
    let editor = app.config.editor.clone();
    suspend_terminal(terminal, || open_editor(&editor, &path))?;
    set_updated_at_now(&path)?;
    refresh_viewer(app)?;
    app.refresh()?;
    app.set_status(format!("Edited {}", path.display()));
    Ok(())
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
        path: target.path,
        content: body.trim_start().to_string(),
        scroll: 0,
    });
    Ok(())
}

fn refresh_viewer(app: &mut App) -> AppResult<()> {
    let Some(viewer) = app.viewer.as_mut() else {
        return Ok(());
    };

    let content = fs::read_to_string(&viewer.path)?;
    let (_, body) = split_front_matter(&content);
    viewer.content = body.trim_start().to_string();
    viewer.scroll = 0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn enter_on_journals_moves_to_entries_like_right_arrow() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut enter_app = App::new(config.clone()).unwrap();
        let mut right_app = App::new(config).unwrap();

        enter_app.focus = Focus::Journals;
        right_app.focus = Focus::Journals;

        handle_enter(&mut enter_app, true).unwrap();
        move_focus_right(&mut right_app, true);

        assert_eq!(enter_app.focus, Focus::Entries);
        assert_eq!(enter_app.focus, right_app.focus);
    }

    #[test]
    fn right_on_entry_opens_viewer_when_preview_panel_is_hidden() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n---\n\n# A\nBody\n").unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        handle_right(&mut app, false).unwrap();

        assert!(app.viewer.is_some());
        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn right_on_entry_focuses_preview_when_preview_panel_is_visible() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n---\n\n# A\nBody\n").unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        handle_right(&mut app, true).unwrap();

        assert!(app.viewer.is_none());
        assert_eq!(app.focus, Focus::Preview);
    }

    #[test]
    fn left_closes_viewer_only_when_preview_panel_is_hidden() {
        assert!(viewer_key_closes(KeyCode::Left, false));
        assert!(!viewer_key_closes(KeyCode::Left, true));
    }
}
