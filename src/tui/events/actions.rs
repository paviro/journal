use crate::{
    AppResult, crypto,
    markdown::split_front_matter,
    storage::{
        create_encrypted_entry, create_entry, create_journal, edit_encrypted_entry,
        is_encrypted_entry_file, move_entry_to_trash, open_editor,
        read_entry_content_with_identity, set_updated_at_now,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

use super::terminal::suspend_terminal;
use crate::tui::app::{App, MarkdownView};

pub(super) fn edit_viewer_entry(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let Some(viewer) = app.viewer.as_ref() else {
        return Ok(());
    };

    let path = viewer.path.clone();
    if is_encrypted_entry_file(&path) && app.unlocked_identity.is_none() {
        app.set_status("Encryption identity not available");
        return Ok(());
    }
    let editor = app.config.editor.clone();
    suspend_terminal(terminal, || {
        if is_encrypted_entry_file(&path) {
            edit_encrypted_entry(
                &path,
                &editor,
                &app.encryption_paths,
                unlocked_identity(app)?,
            )
        } else {
            open_editor(&editor, &path)?;
            set_updated_at_now(&path)
        }
    })?;
    refresh_viewer(app)?;
    app.refresh()?;
    app.set_status(format!("Edited {}", path.display()));
    Ok(())
}

pub(super) fn submit_new_journal(app: &mut App) -> AppResult<()> {
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

pub(super) fn create_entry_in_selected_journal(
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
    if crypto::should_encrypt(&app.encryption_paths) && app.unlocked_identity.is_none() {
        app.set_status("Encryption identity not available");
        return Ok(());
    }
    suspend_terminal(terminal, || {
        if crypto::should_encrypt(&app.encryption_paths) {
            create_encrypted_entry(
                &root,
                &journal_name,
                &editor,
                &app.encryption_paths,
                unlocked_identity(app)?,
            )
        } else {
            create_entry(&root, &journal_name, &editor)
        }
    })?;
    app.set_status("Entry saved");
    app.refresh()?;
    Ok(())
}

pub(super) fn edit_selected(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if is_encrypted_entry_file(&target.path) && app.unlocked_identity.is_none() {
        app.set_status("Encryption identity not available");
        return Ok(());
    }

    let editor = app.config.editor.clone();
    suspend_terminal(terminal, || {
        if is_encrypted_entry_file(&target.path) {
            edit_encrypted_entry(
                &target.path,
                &editor,
                &app.encryption_paths,
                unlocked_identity(app)?,
            )
        } else {
            open_editor(&editor, &target.path)?;
            set_updated_at_now(&target.path)
        }
    })?;
    app.set_status(format!("Edited {}", target.path.display()));
    app.refresh()?;
    Ok(())
}

pub(super) fn view_selected(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if is_encrypted_entry_file(&target.path) && app.unlocked_identity.is_none() {
        app.set_status("Encryption identity not available");
        return Ok(());
    }

    let title = app
        .selected_entry_view()
        .map(|(title, _)| title)
        .unwrap_or_else(|| target.title.clone());
    let content = read_entry_content_with_identity(&target.path, app.unlocked_identity.as_ref())?;
    let (_, body) = split_front_matter(&content);
    app.viewer = Some(MarkdownView {
        title,
        path: target.path,
        content: body.trim_start().to_string(),
        scroll: 0,
    });
    Ok(())
}

fn refresh_viewer(app: &mut App) -> AppResult<()> {
    let Some(path) = app.viewer.as_ref().map(|viewer| viewer.path.clone()) else {
        return Ok(());
    };

    if is_encrypted_entry_file(&path) && app.unlocked_identity.is_none() {
        app.set_status("Encryption identity not available");
        return Ok(());
    }

    let content = read_entry_content_with_identity(&path, app.unlocked_identity.as_ref())?;
    let (_, body) = split_front_matter(&content);
    let Some(viewer) = app.viewer.as_mut() else {
        return Ok(());
    };
    viewer.content = body.trim_start().to_string();
    viewer.scroll = 0;
    Ok(())
}

fn unlocked_identity(app: &App) -> AppResult<&crate::crypto::UnlockedIdentity> {
    app.unlocked_identity
        .as_ref()
        .ok_or_else(|| "encrypted entry requires unlocked journal encryption identity".into())
}

pub(super) fn delete_selected(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };
    move_entry_to_trash(&app.config.journal_root, &target.path)?;

    app.set_status("Moved to trash");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, crypto};
    use std::fs;
    use tempfile::tempdir;

    fn new_app(config: Config) -> App {
        let encryption_paths = crypto::EncryptionPaths::for_config(
            &config.journal_root.join("config.toml"),
            &config.journal_root,
        )
        .unwrap();
        App::new(config, encryption_paths).unwrap()
    }

    #[test]
    fn view_selected_locked_entry_sets_status_without_opening_viewer() {
        let dir = tempdir().unwrap();
        let path = dir
            .path()
            .join("work")
            .join("2026")
            .join("07")
            .join("01")
            .join("2026-07-01T10-23-00-secret.md.age");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "locked ciphertext placeholder").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        view_selected(&mut app).unwrap();

        assert_eq!(app.status, "Encryption identity not available");
        assert!(app.viewer.is_none());
    }
}
