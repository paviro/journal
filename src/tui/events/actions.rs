use crate::AppResult;
use notema_core::{Location, MetadataField};
use notema_storage::EditOutcome;
use std::path::{Path, PathBuf};

use crate::tui::app::{App, EntryTarget, Focus};
use crate::tui::editor_state::EditorTarget;
use crate::tui::environment::environment_fields;
use crate::tui::state::{MetadataKind, Overlay, ToastVariant};
use std::time::Instant;

pub(super) fn submit_new_journal(app: &mut App) -> AppResult<()> {
    let value = app
        .new_journal_input()
        .map(|input| input.as_str().trim())
        .unwrap_or_default()
        .to_string();
    if value.is_empty() {
        app.toast(ToastVariant::Info, "Nothing added");
        app.close_overlay();
        return Ok(());
    }

    let journal = app.store.create_journal(&value)?;
    app.refresh()?;
    app.select_journal_by_name(&journal.name);
    app.toast(
        ToastVariant::Success,
        format!("Created journal {}", journal.name),
    );
    app.close_overlay();
    Ok(())
}

/// Build a save status message, appending image ingest details when relevant.
fn save_status(base: &str, report: &notema_storage::AssetReport) -> String {
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

/// A save toast's variant: a save that dropped images is a warning, not a
/// clean success.
fn save_variant(report: &notema_storage::AssetReport) -> ToastVariant {
    if report.failed.is_empty() {
        ToastVariant::Success
    } else {
        ToastVariant::Warning
    }
}

fn asset_options(app: &App) -> notema_storage::EntryAssetOptions {
    notema_storage::EntryAssetOptions {
        download_remote: app.config.attachments.download_remote_images,
        replace_offline: false,
    }
}

/// Reports a friendly status and returns `false` when the target is a locked
/// encrypted entry that cannot be read or written without the identity.
fn reject_if_locked(app: &mut App, target: &EntryTarget) -> bool {
    if target.locked {
        app.toast(ToastVariant::Error, "Encryption identity not available");
        return false;
    }
    true
}

/// The shared post-edit tail for an existing entry: ingest any new image assets
/// (or report the deletion), refresh the entry, and back-fill weather after a
/// real change.
fn finish_existing_edit(
    app: &mut App,
    path: &Path,
    title: &str,
    outcome: EditOutcome,
    report: &notema_storage::AssetReport,
) -> AppResult<()> {
    match outcome {
        EditOutcome::Unchanged => {
            app.toast(ToastVariant::Info, "No changes");
            return Ok(());
        }
        EditOutcome::Changed => app.toast(
            save_variant(report),
            save_status(&format!("Edited {title}"), report),
        ),
        EditOutcome::Deleted => app.toast(ToastVariant::Success, "Empty entry deleted"),
    }
    refresh_entry_path(app, path)?;
    Ok(())
}

fn clear_environment_fields() -> Vec<MetadataField> {
    vec![
        MetadataField::Weather(None),
        MetadataField::Celestial(None),
        MetadataField::AirQuality(None),
    ]
}

/// The metadata fields for the editor's landed background environment, or empty when
/// none arrived (a coordless location, or the fetch returned nothing).
fn editor_environment_fields(app: &App) -> Vec<MetadataField> {
    app.editor
        .as_ref()
        .and_then(|editor| editor.environment.as_ref())
        .map(environment_fields)
        .unwrap_or_default()
}

/// If the editor is still waiting on its background environment fetch, open the
/// "Fetching…" modal and tell the caller to abort this save. The event loop
/// re-runs the save once the fetch lands or the timeout fires (see
/// [`super::poll_fetching_environment`]).
fn defer_for_pending_environment(app: &mut App) -> bool {
    if app
        .editor
        .as_ref()
        .is_some_and(|editor| editor.pending_environment.is_some())
    {
        app.overlay = Overlay::FetchingEnvironment(Instant::now());
        true
    } else {
        false
    }
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
            let location_changed = metadata.location != original_metadata.location;
            // Only a changed location attaches fresh environment; when it's just
            // removed we clear the stale fields, and an unchanged location leaves
            // whatever's there (parse-time backfill fills a missing one).
            let extra_fields = if text.trim().is_empty() || !location_changed {
                Vec::new()
            } else if metadata.location.is_none() {
                clear_environment_fields()
            } else {
                // Wait for the background fetch spawned when the location changed.
                if defer_for_pending_environment(app) {
                    return Ok(());
                }
                editor_environment_fields(app)
            };
            let saved = app.store.save_entry_edit(
                &path,
                notema_storage::EntryEdit {
                    body: &text,
                    metadata: &metadata,
                    original_metadata: &original_metadata,
                    writing_seconds: (text != original_body).then_some(elapsed.as_secs()),
                    remove_if_empty: true,
                    extra_fields: &extra_fields,
                },
                asset_options(app),
            )?;
            finish_existing_edit(app, &path, &title, saved.outcome, &saved.assets)?;
            app.editor = None;
            if saved.outcome == EditOutcome::Deleted {
                app.nav.focus = Focus::Entries;
                app.nav.scroll.reset_entry_view();
            } else {
                app.nav.focus = Focus::EntryView;
            }
        }
        EditorTarget::New { journal } => {
            let created = if text.trim().is_empty() {
                None
            } else {
                // Wait for the background fetch spawned when the location was set.
                if defer_for_pending_environment(app) {
                    return Ok(());
                }
                let environment = app
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.environment.clone())
                    .unwrap_or_default();
                let mut draft = notema_storage::EntryDraft::new(&journal, &text, &metadata);
                draft.weather = environment.weather.as_ref();
                draft.celestial = environment.celestial.as_ref();
                draft.air_quality = environment.air_quality.as_ref();
                draft.writing_seconds = Some(elapsed.as_secs());
                Some(app.store.create_entry(draft, asset_options(app))?)
            };
            match created {
                Some(created) => {
                    app.toast(
                        save_variant(&created.assets),
                        save_status("Entry saved", &created.assets),
                    );
                    let path = created.path;
                    refresh_entry_path(app, &path)?;
                    if let Some(id) = notema_storage::entry_id(&path)
                        && app.select_entry_by_id(&id, true)
                    {
                        app.nav.focus = Focus::EntryView;
                    }
                    app.editor = None;
                }
                None => {
                    app.toast(ToastVariant::Info, "Nothing added");
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
        app.toast(ToastVariant::Success, "Moved to trash");
    } else {
        app.store.delete_empty_entry(&target.path)?;
        app.toast(ToastVariant::Success, "Deleted");
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

    let display = notema_storage::journal_display_name(&journal_name).to_string();
    app.store
        .delete_journal(&journal_name, &journal_path, &entries)?;
    app.toast(ToastVariant::Success, format!("Deleted journal {display}"));
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
    app.toast(
        ToastVariant::Success,
        format!(
            "{} journal {display}",
            if archive { "Archived" } else { "Unarchived" }
        ),
    );
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

    app.toast(ToastVariant::Success, format!("{} saved", kind.title()));
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

    app.toast(ToastVariant::Success, "Feelings saved");
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

    app.toast(ToastVariant::Success, "Mood saved");
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
    // Write the location now; clearing also drops the stale environment fields.
    let mut fields = vec![MetadataField::Location(location.clone().map(Box::new))];
    if location.is_none() {
        fields.extend(clear_environment_fields());
    }
    app.store.set_entry_metadata_fields(&target.path, &fields)?;

    app.toast(
        ToastVariant::Success,
        if had_location {
            "Location saved"
        } else {
            "Location cleared"
        },
    );
    refresh_entry_path(app, &target.path)?;

    // Fetch weather/air/celestial in the background; it's written back when it
    // lands, without touching `edited_at`. No date means no lookup is possible.
    if let (Some(location), Some(datetime)) = (location, datetime) {
        app.spawn_entry_environment_for(target.path.clone(), &location, datetime);
    }
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

    app.toast(
        ToastVariant::Success,
        if starred { "Starred" } else { "Unstarred" },
    );
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
    use notema_storage::JournalStore;
    use std::fs;
    use tempfile::tempdir;

    fn new_app(config: Config) -> App {
        let config_path = config.journal.path.join("config.toml");
        let store = JournalStore::for_config(&config_path, &config.journal.path).unwrap();
        App::new(config_path, config, store).unwrap()
    }

    /// The newest toast as `(message, variant)`.
    fn last_toast(app: &App) -> (&str, ToastVariant) {
        let toast = app.toasts.items().last().expect("a toast was pushed");
        (toast.message.as_str(), toast.variant)
    }

    #[test]
    fn view_selected_locked_entry_toasts_without_opening_viewer() {
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

        let config = Config::new(dir.path().to_path_buf());
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        view_selected(&mut app).unwrap();

        assert_eq!(
            last_toast(&app),
            ("Encryption identity not available", ToastVariant::Error)
        );
    }

    #[test]
    fn set_feelings_on_entry_writes_front_matter_and_refreshes_app() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let path = entry_dir.join("a.md");
        fs::write(&path, "+++\ntags = []\nfeelings = []\n+++\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf());
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

        let config = Config::new(dir.path().to_path_buf());
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

        let config = Config::new(dir.path().to_path_buf());
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
            notema_core::Metadata::default(),
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
        assert_eq!(last_toast(&app), ("No changes", ToastVariant::Info));
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
            notema_core::Metadata::default(),
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
            notema_core::Metadata::default(),
        ));
        save_internal_editor(&mut app).unwrap();

        assert!(!path.exists());
        assert_eq!(
            last_toast(&app),
            ("Empty entry deleted", ToastVariant::Success)
        );
    }

    #[test]
    fn save_internal_editor_creates_new_entry() {
        let (_dir, mut app, _path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");

        let mut editor = EntryEditor::for_new("work".to_string());
        editor.textarea.insert_str("Brand new thoughts");
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        let (message, variant) = last_toast(&app);
        assert!(message.starts_with("Entry saved"));
        assert_eq!(variant, ToastVariant::Success);
        assert!(
            app.library
                .entries
                .iter()
                .any(|entry| entry.body.contains("Brand new thoughts"))
        );
    }

    #[test]
    fn new_entry_attaches_prefetched_environment() {
        use crate::tui::environment::Environment;
        use notema_context_provider::compute_celestial;
        use notema_core::Location;

        let (_dir, mut app, _path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");

        let datetime = chrono::Local::now().fixed_offset();
        let mut editor = EntryEditor::for_new("work".to_string());
        editor.textarea.insert_str("Located entry");
        editor.metadata.location = Some(Location {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Location::default()
        });
        // The background fetch already landed (celestial is offline, always present).
        editor.environment = Some(Environment {
            celestial: Some(compute_celestial(52.52, 13.405, datetime)),
            weather: None,
            air_quality: None,
        });
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        assert!(
            app.library
                .entries
                .iter()
                .any(|entry| entry.body.contains("Located entry") && entry.celestial.is_some()),
            "prefetched environment is attached to the created entry"
        );
    }

    #[test]
    fn new_entry_save_defers_while_context_pending() {
        let (_dir, mut app, _path) = app_with_entry("+++\ntags = []\n+++\n\n# A\n");

        let mut editor = EntryEditor::for_new("work".to_string());
        editor.textarea.insert_str("Waiting on weather");
        editor.pending_environment = Some(1);
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        // The save is deferred: the modal is up, the editor stays open, nothing written.
        assert!(matches!(
            app.overlay,
            crate::tui::state::Overlay::FetchingEnvironment(_)
        ));
        assert!(app.editor.is_some());
        assert!(
            !app.library
                .entries
                .iter()
                .any(|entry| entry.body.contains("Waiting on weather"))
        );
    }

    #[test]
    fn located_entry_without_context_is_backfill_queued_once() {
        let body = "+++\n[location]\nlatitude = 52.52\nlongitude = 13.405\n+++\n\n# A\n";
        let (_dir, app, _path) = app_with_entry(body);

        // Loading the store already scanned and queued the located, environment-less
        // entry (celestial absent marks it); a re-scan must not double-queue it.
        assert_eq!(app.backfill_queue.len(), 1);
        let mut app = app;
        app.enqueue_environment_backfill();
        assert_eq!(app.backfill_queue.len(), 1);
    }

    #[test]
    fn direct_location_set_claims_entry_so_backfill_cannot_duplicate() {
        // Dated (so a weather lookup can fire) but initially locationless, so it
        // starts off the backfill queue.
        let body = "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+00:00\"\n+++\n\n# A\n";
        let (_dir, mut app, path) = app_with_entry(body);
        assert!(app.backfill_queue.is_empty());

        let location = Location {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Location::default()
        };
        set_location_on_entry(&mut app, Some(location)).unwrap();

        // Setting the location enqueues the entry for backfill and also fires an
        // immediate fetch. The fetch claims the path, so it must not remain queued —
        // otherwise backfill would fetch and write the same environment a second time.
        assert!(app.backfill_enqueued.contains(&path));
        assert!(!app.backfill_queue.contains(&path));
        app.dispatch_environment_backfill();
        assert!(!app.backfill_queue.contains(&path));
    }

    #[test]
    fn backfill_dispatch_skips_entry_that_already_has_environment() {
        use notema_context_provider::compute_celestial;
        let body = "+++\n[location]\nlatitude = 52.52\nlongitude = 13.405\n+++\n\n# A\n";
        let (_dir, mut app, path) = app_with_entry(body);
        assert_eq!(app.backfill_queue.len(), 1);

        // A direct location-set both queues the entry for backfill and fires an
        // immediate fetch. Simulate that fetch landing first: environment present.
        let datetime = chrono::Local::now().fixed_offset();
        for entry in &mut app.library.entries {
            if entry.path == path {
                entry.celestial = Some(compute_celestial(52.52, 13.405, datetime));
            }
        }

        // The queued job is now stale — dispatch drains it without a duplicate fetch.
        app.dispatch_environment_backfill();
        assert!(app.backfill_inflight.is_none());
        assert!(app.backfill_queue.is_empty());
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
