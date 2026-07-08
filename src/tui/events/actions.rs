use crate::{AppResult, editor};
use journal_storage::MetadataField;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    path::{Path, PathBuf},
};

use super::terminal::suspend_terminal;
use crate::tui::app::{App, EntryTarget, Focus};
use crate::tui::state::MetadataKind;

type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// Open the entry at `path` in the editor, transparently handling encrypted
/// entries (decrypt to a temp file, edit, re-encrypt) and plaintext ones.
/// Returns `true` if the entry was kept, `false` if it was deleted for being empty.
fn edit_entry_at(terminal: &mut Term, app: &App, path: &Path, editor_cmd: &str) -> AppResult<bool> {
    suspend_terminal(terminal, || {
        app.store
            .edit_entry_via_editor(path, true, |body| editor::edit_body(editor_cmd, body))
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

    let journal = app.store.create_journal(&value)?;
    app.refresh()?;
    app.select_journal_by_name(&journal.name);
    app.set_status(format!("Created journal {}", journal.name));
    app.close_overlay();
    Ok(())
}

pub(super) fn create_entry_in_selected_journal(
    terminal: &mut Term,
    app: &mut App,
) -> AppResult<Option<PathBuf>> {
    if app.selected_journal().is_some() {
        new_entry(terminal, app)
    } else {
        app.set_status("Create a journal first with n");
        Ok(None)
    }
}

fn new_entry(terminal: &mut Term, app: &mut App) -> AppResult<Option<PathBuf>> {
    let Some(journal) = app.selected_journal().cloned() else {
        app.set_status("No journal selected");
        return Ok(None);
    };

    let editor_cmd = app.config.editor.command.clone();
    let journal_name = journal.name;
    let created = suspend_terminal(terminal, || {
        app.store.create_entry_via_editor(
            &journal_name,
            &journal_storage::Metadata::default(),
            |body| editor::edit_body(&editor_cmd, body),
        )
    })?;
    if let Some(path) = &created {
        let report = app.store.process_entry_assets(
            path,
            app.config.attachments.download_remote_images,
            false,
        )?;
        app.set_status(save_status("Entry saved", &report));
    }
    app.refresh()?;
    Ok(created)
}

/// Build a save status message, appending image ingest details when relevant.
fn save_status(base: &str, report: &journal_storage::AssetReport) -> String {
    if report.is_noop() {
        return base.to_string();
    }
    let mut parts = vec![base.to_string()];
    if report.stored > 0 {
        parts.push(format!(
            "{} image{} stored",
            report.stored,
            if report.stored == 1 { "" } else { "s" }
        ));
    }
    if report.removed > 0 {
        parts.push(format!("{} removed", report.removed));
    }
    if !report.failed.is_empty() {
        parts.push(format!(
            "{} image{} not stored",
            report.failed.len(),
            if report.failed.len() == 1 { "" } else { "s" }
        ));
    }
    parts.join(" — ")
}

/// Reports a friendly status and returns `false` when the target is a locked
/// encrypted entry that cannot be read or written without the identity.
fn reject_if_locked(app: &mut App, target: &EntryTarget) -> bool {
    if target.locked {
        app.set_status("Encryption identity not available");
        return false;
    }
    true
}

pub(super) fn edit_selected(terminal: &mut Term, app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }

    let editor = app.config.editor.command.clone();
    let kept = edit_entry_at(terminal, app, &target.path, &editor)?;
    if kept {
        let report = app.store.process_entry_assets(
            &target.path,
            app.config.attachments.download_remote_images,
            false,
        )?;
        app.set_status(save_status(&format!("Edited {}", target.title), &report));
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
    if !reject_if_locked(app, &target) {
        return Ok(());
    }
    // Opening an entry lands on the focused viewer; full screen is a second,
    // explicit step (multi-column) reached by pressing Enter again.
    app.nav.entry_view_fullscreen = false;
    app.nav.focus = Focus::EntryView;
    Ok(())
}

pub(super) fn delete_selected(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };
    let has_body = app
        .library
        .entries
        .iter()
        .find(|e| e.path == target.path)
        .map(|e| !e.content.trim().is_empty())
        .unwrap_or(false);

    if has_body {
        app.store.move_entry_to_trash(&target.path)?;
        app.set_status("Moved to trash");
    } else {
        app.store.delete_empty_entry(&target.path)?;
        app.set_status("Deleted");
    }
    Ok(())
}

pub(super) fn delete_selected_journal(app: &mut App) -> AppResult<()> {
    let Some(journal) = app.selected_journal() else {
        return Ok(());
    };
    let journal_name = journal.name.clone();
    let journal_path = journal.path.clone();

    let entries: Vec<(PathBuf, bool)> = app
        .library
        .entries
        .iter()
        .filter(|e| e.journal == journal_name)
        .map(|e| (e.path.clone(), !e.content.trim().is_empty()))
        .collect();

    let display = journal_storage::journal_display_name(&journal_name).to_string();
    app.store
        .delete_journal(&journal_name, &journal_path, &entries)?;
    app.set_status(format!("Deleted journal {display}"));
    Ok(())
}

pub(super) fn toggle_archive_selected_journal(app: &mut App) -> AppResult<()> {
    let Some(journal) = app.selected_journal() else {
        return Ok(());
    };
    let old_name = journal.name.clone();
    let archive = !journal.archived;
    let display = journal.display_name().to_string();

    let new_journal = app.store.set_journal_archived(&old_name, archive)?;
    // The rename changes the journal's identity, so config keys pointing at the
    // old name would go stale (CLI resolution, next-launch reselect). Retarget
    // them before reloading.
    app.retarget_journal_in_config(&old_name, &new_journal.name)?;
    app.refresh()?;
    app.select_journal_by_name(&new_journal.name);
    // Keep focus on the journals column so the user can keep managing journals.
    app.nav.focus = Focus::Journals;
    app.set_status(format!(
        "{} journal {display}",
        if archive { "Archived" } else { "Unarchived" }
    ));
    Ok(())
}

pub(super) fn set_metadata_on_entry(
    app: &mut App,
    kind: MetadataKind,
    values: &[String],
) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }

    let field = match kind {
        MetadataKind::Tags => MetadataField::Tags(values.to_vec()),
        MetadataKind::People => MetadataField::People(values.to_vec()),
        MetadataKind::Activities => MetadataField::Activities(values.to_vec()),
    };
    app.store.set_entry_metadata_field(&target.path, field)?;

    app.set_status(format!("{} saved", kind.title()));
    app.refresh()?;
    Ok(())
}

pub(super) fn set_feelings_on_entry(app: &mut App, feelings: &[String]) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }

    app.store
        .set_entry_metadata_field(&target.path, MetadataField::Feelings(feelings.to_vec()))?;

    app.set_status("Feelings saved");
    app.refresh()?;
    Ok(())
}

pub(super) fn set_mood_on_entry(app: &mut App, mood: Option<i8>) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }

    app.store
        .set_entry_metadata_field(&target.path, MetadataField::Mood(mood))?;

    app.set_status("Mood saved");
    app.refresh()?;
    Ok(())
}

pub(super) fn toggle_starred_on_entry(app: &mut App) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }

    let starred = !app.selected_entry_starred();
    app.store
        .set_entry_metadata_field(&target.path, MetadataField::Starred(starred))?;

    app.set_status(if starred { "Starred" } else { "Unstarred" });
    app.refresh()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use journal_storage::JournalStore;
    use std::fs;
    use tempfile::tempdir;

    fn new_app(config: Config) -> App {
        let config_path = config.journal.path.join("config.toml");
        let store = JournalStore::for_config(&config_path, &config.journal.path).unwrap();
        App::new(config_path, config, store).unwrap()
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
    }

    #[test]
    fn set_feelings_on_entry_writes_front_matter_and_refreshes_app() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let path = entry_dir.join("a.md");
        fs::write(&path, "+++\ntags = []\nfeelings = []\n+++\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        let feelings = vec!["calm".to_string(), "focused".to_string()];

        set_feelings_on_entry(&mut app, &feelings).unwrap();

        assert_eq!(app.selected_entry_feelings(), feelings);
    }
}
