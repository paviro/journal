use crate::{
    AppResult, crypto,
    markdown::{entry_has_body, set_feelings_in_front_matter, set_tags_in_front_matter},
    storage::{
        create_encrypted_entry, create_entry, create_journal, edit_encrypted_entry,
        is_encrypted_entry_file, move_entry_to_trash, open_editor,
        read_entry_content_with_identity, set_updated_at_now,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{fs, io, path::Path};

use super::terminal::suspend_terminal;
use crate::tui::app::{App, Focus};

type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// Returns `true` if the caller may proceed. When an encryption identity is
/// required but not unlocked, sets a status message and returns `false`.
fn ensure_identity_available(app: &mut App, needs_identity: bool) -> bool {
    if needs_identity && app.unlocked_identity.is_none() {
        app.set_status("Encryption identity not available");
        return false;
    }
    true
}

/// Open the entry at `path` in the editor, transparently handling encrypted
/// entries (decrypt to a temp file, edit, re-encrypt) and plaintext ones.
/// Returns `true` if the entry was kept, `false` if it was deleted for being empty.
fn edit_entry_at(terminal: &mut Term, app: &App, path: &Path, editor: &str) -> AppResult<bool> {
    suspend_terminal(terminal, || {
        if is_encrypted_entry_file(path) {
            edit_encrypted_entry(
                path,
                editor,
                &app.encryption_paths,
                unlocked_identity(app)?,
                true,
            )?;
            Ok(path.exists())
        } else {
            open_editor(editor, path)?;
            if !entry_has_body(&fs::read_to_string(path)?) {
                fs::remove_file(path)?;
                return Ok(false);
            }
            set_updated_at_now(path)?;
            Ok(true)
        }
    })
}

pub(super) fn submit_new_journal(app: &mut App) -> AppResult<()> {
    let value = app
        .new_journal_input()
        .unwrap_or_default()
        .trim()
        .to_string();
    if value.is_empty() {
        app.set_status("Nothing added");
        app.close_overlay();
        return Ok(());
    }

    let journal = create_journal(&app.config.journal_root, &value)?;
    app.refresh()?;
    app.select_journal_by_name(&journal.name);
    app.set_status(format!("Created journal {}", journal.name));
    app.close_overlay();
    Ok(())
}

pub(super) fn create_entry_in_selected_journal(
    terminal: &mut Term,
    app: &mut App,
) -> AppResult<()> {
    if app.selected_journal().is_some() {
        new_entry(terminal, app)
    } else {
        app.set_status("Create a journal first with n");
        Ok(())
    }
}

fn new_entry(terminal: &mut Term, app: &mut App) -> AppResult<()> {
    let Some(journal) = app.selected_journal().cloned() else {
        app.set_status("No journal selected");
        return Ok(());
    };

    let root = app.config.journal_root.clone();
    let editor = app.config.editor.clone();
    let journal_name = journal.name;
    if !ensure_identity_available(app, crypto::should_encrypt(&app.encryption_paths)) {
        return Ok(());
    }
    let created = suspend_terminal(terminal, || {
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
    if created.is_some() {
        app.set_status("Entry saved");
    }
    app.refresh()?;
    Ok(())
}

pub(super) fn edit_selected(terminal: &mut Term, app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !ensure_identity_available(app, is_encrypted_entry_file(&target.path)) {
        return Ok(());
    }

    let editor = app.config.editor.clone();
    let kept = edit_entry_at(terminal, app, &target.path, &editor)?;
    if kept {
        app.set_status(format!("Edited {}", target.path.display()));
    } else {
        app.set_status("Empty entry deleted");
    }
    app.refresh()?;
    Ok(())
}

pub(super) fn view_selected(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };
    if !ensure_identity_available(app, is_encrypted_entry_file(&target.path)) {
        return Ok(());
    }
    app.entry_view_expanded = true;
    app.focus = Focus::EntryView;
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

pub(super) fn set_tags_on_entry(app: &mut App, tags: &[String]) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if is_encrypted_entry_file(&target.path) {
        let Some(ref identity) = app.unlocked_identity else {
            app.set_status("Encryption identity not available");
            return Ok(());
        };
        let content = read_entry_content_with_identity(&target.path, Some(identity))?;
        let Some(new_content) = set_tags_in_front_matter(&content, tags) else {
            return Ok(());
        };
        let temp_path = std::env::temp_dir().join(format!(
            ".journal-tag-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&temp_path, &new_content)?;
        crypto::encrypt_file(&app.encryption_paths, &temp_path, &target.path)?;
        let _ = fs::remove_file(&temp_path);
    } else {
        let content = fs::read_to_string(&target.path)?;
        let Some(new_content) = set_tags_in_front_matter(&content, tags) else {
            return Ok(());
        };
        fs::write(&target.path, new_content)?;
        set_updated_at_now(&target.path)?;
    }

    app.set_status("Tags saved");
    app.refresh()?;
    Ok(())
}

pub(super) fn set_feelings_on_entry(app: &mut App, feelings: &[String]) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if is_encrypted_entry_file(&target.path) {
        let Some(ref identity) = app.unlocked_identity else {
            app.set_status("Encryption identity not available");
            return Ok(());
        };
        let content = read_entry_content_with_identity(&target.path, Some(identity))?;
        let Some(new_content) = set_feelings_in_front_matter(&content, feelings) else {
            return Ok(());
        };
        let temp_path = std::env::temp_dir().join(format!(
            ".journal-feelings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&temp_path, &new_content)?;
        crypto::encrypt_file(&app.encryption_paths, &temp_path, &target.path)?;
        let _ = fs::remove_file(&temp_path);
    } else {
        let content = fs::read_to_string(&target.path)?;
        let Some(new_content) = set_feelings_in_front_matter(&content, feelings) else {
            return Ok(());
        };
        fs::write(&target.path, new_content)?;
        set_updated_at_now(&target.path)?;
    }

    app.set_status("Feelings saved");
    app.refresh()?;
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

        assert_eq!(app.status(), "Encryption identity not available");
        assert!(!app.entry_view_expanded);
    }

    #[test]
    fn set_feelings_on_entry_writes_front_matter_and_refreshes_app() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let path = entry_dir.join("a.md");
        fs::write(&path, "---\ntags: []\nfeelings: []\n...\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        let feelings = vec!["calm".to_string(), "focused".to_string()];

        set_feelings_on_entry(&mut app, &feelings).unwrap();

        let content = fs::read_to_string(path).unwrap();
        let (front_matter, _) = crate::markdown::split_front_matter(&content);
        assert_eq!(
            front_matter.map(crate::markdown::front_matter_feelings),
            Some(feelings.clone())
        );
        assert_eq!(app.selected_entry_feelings(), feelings);
    }
}
