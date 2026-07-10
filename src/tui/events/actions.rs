use crate::{AppResult, editor};
use journal_core::{Location, Metadata, MetadataField};
use journal_storage::EditOutcome;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use super::terminal::suspend_terminal;
use crate::tui::app::{App, EntryTarget, Focus};
use crate::tui::editor_state::EditorTarget;
use crate::tui::state::MetadataKind;

type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// Open the entry at `path` in the editor, transparently handling encrypted
/// entries (decrypt to a temp file, edit, re-encrypt) and plaintext ones. Records
/// the editor-open time against the entry when the edit actually changed it.
/// Returns the [`EditOutcome`].
fn edit_entry_at(
    terminal: &mut Term,
    app: &App,
    path: &Path,
    editor_cmd: &str,
) -> AppResult<EditOutcome> {
    let start = Instant::now();
    let outcome = suspend_terminal(terminal, || {
        app.store
            .edit_entry_via_editor(path, true, |body| editor::edit_body(editor_cmd, body))
    })?;
    if outcome == EditOutcome::Changed {
        app.store
            .add_writing_seconds(path, start.elapsed().as_secs())?;
    }
    Ok(outcome)
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
    let start = Instant::now();
    let created = suspend_terminal(terminal, || {
        app.store.create_entry_via_editor(
            &journal_name,
            &journal_core::Metadata::default(),
            |body| editor::edit_body(&editor_cmd, body),
        )
    })?;
    let elapsed = start.elapsed();
    if let Some(path) = &created {
        let report = app.store.process_entry_assets(
            path,
            app.config.attachments.download_remote_images,
            false,
        )?;
        app.set_status(save_status("Entry saved", &report));
        // A new entry always counts as written.
        app.store.add_writing_seconds(path, elapsed.as_secs())?;
        refresh_entry_path(app, path)?;
    }
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
    let outcome = edit_entry_at(terminal, app, &target.path, &editor)?;
    finish_existing_edit(app, &target.path, &target.title, outcome)
}

/// The shared post-edit tail for an existing entry, run by both the external and
/// internal editors: ingest any new image assets (or report the deletion),
/// refresh the entry, and back-fill weather after a real change.
fn finish_existing_edit(
    app: &mut App,
    path: &Path,
    title: &str,
    outcome: EditOutcome,
) -> AppResult<()> {
    match outcome {
        EditOutcome::Unchanged => {
            app.set_status("No changes");
            return Ok(());
        }
        EditOutcome::Changed => {
            let report = app.store.process_entry_assets(
                path,
                app.config.attachments.download_remote_images,
                false,
            )?;
            app.set_status(save_status(&format!("Edited {title}"), &report));
        }
        EditOutcome::Deleted => app.set_status("Empty entry deleted"),
    }
    refresh_entry_path(app, path)?;

    // After a real edit, back-fill weather for an entry that has coordinates but
    // no weather yet — never clobbering weather already captured (e.g. imported).
    if outcome == EditOutcome::Changed {
        let entry_info = app.resolved_selected_entry().and_then(|entry| {
            (entry.path == *path).then(|| (entry.location.clone(), entry.created_time()))
        });
        if let Some((Some(location), Some(datetime))) = entry_info
            && location.latitude.is_some()
            && location.longitude.is_some()
            && !app.store.entry_has_weather(path).unwrap_or(true)
        {
            app.capture_environment_for_entry(path, &location, datetime);
        }
    }
    Ok(())
}

/// Write only the metadata fields the editor changed to an existing entry, so an
/// unchanged field never rewrites the file (or bumps timestamps unnecessarily).
fn apply_metadata_changes(
    app: &mut App,
    path: &Path,
    original: &Metadata,
    current: &Metadata,
) -> AppResult<()> {
    let mut fields = Vec::new();
    if current.tags != original.tags {
        fields.push(MetadataField::Tags(current.tags.clone()));
    }
    if current.people != original.people {
        fields.push(MetadataField::People(current.people.clone()));
    }
    if current.activities != original.activities {
        fields.push(MetadataField::Activities(current.activities.clone()));
    }
    if current.feelings != original.feelings {
        fields.push(MetadataField::Feelings(current.feelings.clone()));
    }
    if current.mood != original.mood {
        fields.push(MetadataField::Mood(current.mood));
    }
    app.store.set_entry_metadata_fields(path, &fields)?;
    Ok(())
}

/// Save the open internal-editor buffer. The editor stays open until every
/// fallible write/refresh step succeeds.
pub(super) fn save_internal_editor(app: &mut App) -> AppResult<()> {
    let Some(editor) = app.editor.as_ref() else {
        return Ok(());
    };
    let text = editor.text();
    let elapsed = editor.start.elapsed();
    let original_body = editor.original.clone();
    let metadata = editor.metadata.clone();
    let original_metadata = editor.original_metadata.clone();
    let target = editor.target.clone();

    match target {
        EditorTarget::Existing { path, title } => {
            let outcome = if text.trim().is_empty() || text != original_body {
                app.store
                    .edit_entry_via_editor(&path, true, |_old| Ok(Some(text)))?
            } else {
                EditOutcome::Unchanged
            };
            let metadata_changed = metadata != original_metadata;
            if outcome == EditOutcome::Changed {
                app.store.add_writing_seconds(&path, elapsed.as_secs())?;
            }
            if outcome.kept() {
                apply_metadata_changes(app, &path, &original_metadata, &metadata)?;
            }
            let status_outcome = if metadata_changed && outcome == EditOutcome::Unchanged {
                EditOutcome::Changed
            } else {
                outcome
            };
            finish_existing_edit(app, &path, &title, status_outcome)?;
            app.editor = None;
            if outcome == EditOutcome::Deleted {
                app.nav.focus = Focus::Entries;
                app.nav.scroll.reset_entry_view();
            } else {
                app.nav.focus = Focus::EntryView;
            }
        }
        EditorTarget::New { journal } => {
            let created = app
                .store
                .create_entry_via_editor(&journal, &metadata, |_empty| Ok(Some(text)))?;
            match created {
                Some(path) => {
                    let report = app.store.process_entry_assets(
                        &path,
                        app.config.attachments.download_remote_images,
                        false,
                    )?;
                    app.set_status(save_status("Entry saved", &report));
                    app.store.add_writing_seconds(&path, elapsed.as_secs())?;
                    refresh_entry_path(app, &path)?;
                    if let Some(id) = journal_storage::entry_id(&path)
                        && app.select_entry_by_id(&id, true)
                    {
                        app.nav.focus = Focus::EntryView;
                    }
                    app.editor = None;
                }
                None => {
                    app.set_status("Nothing added");
                    app.nav.entry_view_fullscreen = false;
                    app.nav.focus = Focus::Entries;
                    app.editor = None;
                }
            }
        }
    }
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
        .map(|e| !e.body.trim().is_empty())
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
        .map(|e| (e.path.clone(), !e.body.trim().is_empty()))
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
    refresh_entry_path(app, &target.path)?;
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
    refresh_entry_path(app, &target.path)?;
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
    refresh_entry_path(app, &target.path)?;
    Ok(())
}

pub(super) fn set_location_on_entry(app: &mut App, location: Option<Location>) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }

    // Capture the date before the write so weather can be looked up for it.
    let datetime = app
        .resolved_selected_entry()
        .and_then(|entry| entry.created_time());
    let had_location = location.is_some();
    app.store.set_entry_metadata_field(
        &target.path,
        MetadataField::Location(location.clone().map(Box::new)),
    )?;

    // An explicit location change always refreshes the captured environment data;
    // clearing the location clears it. A name-only location (no coordinates) or an
    // entry with no date leaves any existing data untouched.
    match (location, datetime) {
        (Some(location), Some(datetime)) => {
            app.capture_environment_for_entry(&target.path, &location, datetime);
        }
        (None, _) => app.clear_environment_for_entry(&target.path),
        _ => {}
    }

    app.set_status(if had_location {
        "Location saved"
    } else {
        "Location cleared"
    });
    refresh_entry_path(app, &target.path)?;
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
    refresh_entry_path(app, &target.path)?;
    Ok(())
}

fn refresh_entry_path(app: &mut App, path: &Path) -> AppResult<()> {
    app.refresh_paths(&[path.to_path_buf()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::editor_state::EntryEditor;
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

    #[test]
    fn set_location_on_entry_writes_and_clears_front_matter() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let path = entry_dir.join("a.md");
        fs::write(&path, "+++\ntags = []\n+++\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        let location = Location {
            name: Some("Cafe".to_string()),
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Location::default()
        };
        set_location_on_entry(&mut app, Some(location)).unwrap();
        assert_eq!(app.selected_entry_location().as_deref(), Some("Cafe"));

        set_location_on_entry(&mut app, None).unwrap();
        assert_eq!(app.selected_entry_location(), None);
    }

    fn app_with_entry(body: &str) -> (tempfile::TempDir, App, PathBuf) {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let path = entry_dir.join("a.md");
        fs::write(&path, body).unwrap();

        let config = Config::new(dir.path().to_path_buf(), "internal");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        (dir, app, path)
    }

    #[test]
    fn open_editor_loads_body_and_focuses_view() {
        let (_dir, mut app, _path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");
        let expected = app.resolved_selected_entry().unwrap().body.clone();

        app.open_editor_for_selected();

        assert!(app.editor.is_some());
        assert_eq!(app.nav.focus, Focus::EntryView);
        assert_eq!(app.editor.as_ref().unwrap().text(), expected);
    }

    #[test]
    fn save_internal_editor_writes_body_and_bumps_edited_at() {
        let (_dir, mut app, path) = app_with_entry("+++\ntags = []\n+++\n\n# Original\n");
        let target = app.selected_entry_target().unwrap();

        let mut editor = EntryEditor::for_existing(
            target.path.clone(),
            target.title,
            "# Edited body",
            journal_core::Metadata::default(),
        );
        editor.original = "# Original".to_string();
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        assert!(app.editor.is_none());
        let content = app.store.read_entry_content(&path).unwrap();
        assert!(content.contains("# Edited body"));
        assert!(content.contains("edited_at"));
    }

    #[test]
    fn save_internal_editor_unchanged_body_does_not_rewrite() {
        let (_dir, mut app, path) = app_with_entry("+++\ntags = []\n+++\n\n# Original\n");
        let original = app.store.read_entry_content(&path).unwrap();

        app.open_editor_for_selected();
        save_internal_editor(&mut app).unwrap();

        assert!(app.editor.is_none());
        assert_eq!(app.status(), "No changes");
        assert_eq!(app.store.read_entry_content(&path).unwrap(), original);
    }

    #[test]
    fn save_internal_editor_error_keeps_buffer_open() {
        let (_dir, mut app, path) = app_with_entry("+++\ntags = []\n+++\n\n# Original\n");
        let target = app.selected_entry_target().unwrap();
        let mut editor = EntryEditor::for_existing(
            path.with_file_name("missing.md"),
            target.title,
            "# Edited body",
            journal_core::Metadata::default(),
        );
        editor.original = "# Original".to_string();
        app.editor = Some(editor);

        let result = save_internal_editor(&mut app);

        assert!(result.is_err());
        assert_eq!(app.editor.as_ref().unwrap().text(), "# Edited body");
    }

    #[test]
    fn save_internal_editor_empty_body_deletes_existing_entry() {
        let (_dir, mut app, path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");
        let target = app.selected_entry_target().unwrap();

        app.editor = Some(EntryEditor::for_existing(
            target.path.clone(),
            target.title,
            "   \n  ",
            journal_core::Metadata::default(),
        ));
        save_internal_editor(&mut app).unwrap();

        assert!(!path.exists());
        assert_eq!(app.status(), "Empty entry deleted");
    }

    #[test]
    fn save_internal_editor_creates_new_entry() {
        let (_dir, mut app, _path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");

        let mut editor = EntryEditor::for_new("work".to_string());
        editor.textarea.insert_str("Brand new thoughts");
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        assert!(app.status().starts_with("Entry saved"));
        assert!(
            app.library
                .entries
                .iter()
                .any(|entry| entry.body.contains("Brand new thoughts"))
        );
    }

    #[test]
    fn open_editor_seeds_buffered_metadata_from_entry() {
        let (_dir, mut app, _path) = app_with_entry("+++\ntags = [\"seed\"]\n+++\n\n# A\n");
        app.open_editor_for_selected();
        assert_eq!(
            app.editor.as_ref().unwrap().metadata.tags,
            vec!["seed".to_string()]
        );
    }

    #[test]
    fn save_internal_editor_applies_buffered_metadata_to_existing_entry() {
        let (_dir, mut app, path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");
        app.open_editor_for_selected();
        app.set_editor_metadata(
            MetadataKind::Tags,
            &["work".to_string(), "focus".to_string()],
        );

        save_internal_editor(&mut app).unwrap();

        let content = app.store.read_entry_content(&path).unwrap();
        assert!(content.contains("work"), "front matter was: {content}");
        assert!(content.contains("focus"), "front matter was: {content}");
    }

    #[test]
    fn save_internal_editor_writes_buffered_metadata_for_new_entry() {
        let (_dir, mut app, _path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");

        let mut editor = EntryEditor::for_new("work".to_string());
        editor.textarea.insert_str("New body");
        editor.metadata.mood = Some(3);
        editor.metadata.tags = vec!["idea".to_string()];
        app.editor = Some(editor);

        save_internal_editor(&mut app).unwrap();

        let saved = app
            .library
            .entries
            .iter()
            .find(|entry| entry.body.contains("New body"))
            .expect("new entry saved");
        assert_eq!(saved.mood, Some(3));
        assert!(saved.tags.contains(&"idea".to_string()));
    }

    #[test]
    fn cancel_editor_discards_changes() {
        let (_dir, mut app, path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");

        app.open_editor_for_selected();
        app.editor.as_mut().unwrap().textarea.insert_str("mutation");
        assert!(app.editor.as_ref().unwrap().is_dirty());

        app.cancel_editor();

        assert!(app.editor.is_none());
        let content = app.store.read_entry_content(&path).unwrap();
        assert!(!content.contains("mutation"));
    }
}
