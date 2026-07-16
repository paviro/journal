use crate::AppResult;
use chrono::Local;
use notema_domain::{EntryEncryptionState, Location, MetadataField};
use notema_storage::EditOutcome;
use std::path::{Path, PathBuf};

use crate::tui::app::{AppModel, EntryTarget, Focus};
use crate::tui::editor_state::EditorTarget;
use crate::tui::environment::environment_fields;
use crate::tui::state::{MetadataKind, Overlay, ToastVariant};
use std::time::Instant;

use super::{Effect, OpenTarget};

pub(super) fn submit_new_journal(app: &mut AppModel) -> AppResult<()> {
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

    let journal = app.services.store.create_journal(&value)?;
    app.refresh()?;
    app.select_journal_by_name(&journal.name);
    app.toast(
        ToastVariant::Success,
        format!("Created journal {}", journal.name),
    );
    app.close_overlay();
    Ok(())
}

/// Build a save status message with asset ingest details when relevant.
fn save_status(base: &str, report: &notema_storage::AssetReport) -> String {
    if report.is_noop() {
        return base.to_string();
    }
    let mut parts = vec![base.to_string()];
    let images_stored = report.images_stored();
    if images_stored > 0 {
        parts.push(format!(
            "{} image{} stored",
            images_stored,
            if images_stored == 1 { "" } else { "s" }
        ));
    }
    if report.attachments_stored > 0 {
        parts.push(format!(
            "{} attachment{} stored",
            report.attachments_stored,
            if report.attachments_stored == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if report.removed > 0 {
        parts.push(format!("{} removed", report.removed));
    }
    let images_not_stored = report.images_not_stored();
    if images_not_stored > 0 {
        parts.push(format!(
            "{} image{} not stored",
            images_not_stored,
            if images_not_stored == 1 { "" } else { "s" }
        ));
    }
    let attachments_not_stored = report.attachments_not_stored();
    if attachments_not_stored > 0 {
        parts.push(format!(
            "{} attachment{} not stored",
            attachments_not_stored,
            if attachments_not_stored == 1 { "" } else { "s" }
        ));
    }
    parts.join(" — ")
}

/// A save that dropped assets is a warning, not a clean success.
fn save_variant(report: &notema_storage::AssetReport) -> ToastVariant {
    if report.failed.is_empty() {
        ToastVariant::Success
    } else {
        ToastVariant::Warning
    }
}

fn asset_options(app: &AppModel) -> notema_storage::EntryAssetOptions {
    notema_storage::EntryAssetOptions {
        download_remote: app.services.config.attachments.download_remote_images,
        replace_offline: false,
    }
}

/// Reports a friendly status and returns `false` when the target is a locked
/// encrypted entry that cannot be read or written without the identity.
fn reject_if_locked(app: &mut AppModel, target: &EntryTarget) -> bool {
    if target.locked {
        app.toast(ToastVariant::Error, "Encryption identity not available");
        return false;
    }
    true
}

fn reject_if_front_matter_invalid(app: &mut AppModel) -> bool {
    app.editor.is_some() || app.allow_selected_entry_edit()
}

/// The shared post-edit tail for an existing entry: ingest assets, refresh the
/// entry, and back-fill weather after a real change.
fn finish_existing_edit(
    app: &mut AppModel,
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
fn editor_environment_fields(app: &AppModel) -> Vec<MetadataField> {
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
fn defer_for_pending_environment(app: &mut AppModel) -> bool {
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
pub(super) fn save_internal_editor(app: &mut AppModel) -> AppResult<()> {
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
        EditorTarget::Existing {
            journal,
            path,
            title,
            revision,
        } => {
            if text == original_body && metadata == original_metadata {
                app.reload_selected_entry_from_disk()?;
                app.toast(ToastVariant::Info, "No changes");
                app.editor = None;
                app.nav.focus = Focus::Reader;
                return Ok(());
            }
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
            let save_result = app.services.store.save_entry_edit_if_revision(
                &path,
                revision,
                notema_storage::EntryEdit {
                    body: &text,
                    metadata: &metadata,
                    original_metadata: &original_metadata,
                    writing_seconds: (text != original_body).then_some(elapsed.as_secs()),
                    remove_if_empty: true,
                    extra_fields: &extra_fields,
                },
                asset_options(app),
            );
            let saved = match save_result {
                Ok(saved) => saved,
                Err(error)
                    if matches!(
                        error.downcast_ref::<notema_storage::StorageError>(),
                        Some(notema_storage::StorageError::EntryRevisionConflict { .. })
                    ) =>
                {
                    if text.trim().is_empty() {
                        return Err(anyhow::anyhow!(
                            "entry changed on disk; the editor remains open and the original was not deleted"
                        ));
                    }
                    let mut draft = notema_storage::EntryDraft::new(&journal, &text, &metadata);
                    draft.writing_seconds = (text != original_body).then_some(elapsed.as_secs());
                    let created =
                        app.services
                            .store
                            .create_entry_copy(&path, draft, asset_options(app))?;
                    app.toast(
                        ToastVariant::Warning,
                        save_status(
                            "Original changed on disk; saved your edit as a new entry",
                            &created.assets,
                        ),
                    );
                    refresh_entry_path(app, &created.path)?;
                    if let Some(id) = notema_storage::entry_id(&created.path) {
                        app.select_entry_by_id(&id, true);
                    }
                    app.editor = None;
                    app.nav.focus = Focus::Reader;
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            finish_existing_edit(app, &path, &title, saved.outcome, &saved.assets)?;
            app.editor = None;
            if saved.outcome == EditOutcome::Deleted {
                app.nav.focus = Focus::Entries;
                app.nav.scroll.reset_reader();
            } else {
                app.nav.focus = Focus::Reader;
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
                draft.celestial = Some(&environment.celestial);
                draft.air_quality = environment.air_quality.as_ref();
                draft.writing_seconds = Some(elapsed.as_secs());
                // Stamp the entry with its place's timezone rather than the system's,
                // when one was resolved for the location (see set_editor_location).
                if let Some(zone) = app.editor.as_ref().and_then(|editor| editor.zone) {
                    let created_at = notema_context::rezone(Local::now().fixed_offset(), zone);
                    draft.created_at = Some(created_at);
                    draft.edited_at = Some(created_at);
                    draft.timezone = Some(zone.name());
                }
                Some(app.services.store.create_entry(draft, asset_options(app))?)
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
                        app.nav.focus = Focus::Reader;
                    }
                    app.editor = None;
                }
                None => {
                    app.toast(ToastVariant::Info, "Nothing added");
                    app.nav.reader_fullscreen = false;
                    app.nav.focus = Focus::Entries;
                    app.editor = None;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn view_selected(app: &mut AppModel) -> AppResult<()> {
    if !app.reload_selected_entry_from_disk()? {
        return Ok(());
    }
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };
    if !reject_if_locked(app, &target) {
        return Ok(());
    }
    if let Some(warning) = app.selected_entry_edit_warning() {
        app.toast(
            ToastVariant::Warning,
            format!("{warning}. Showing the body only; repair its +++ metadata block to edit."),
        );
    }
    // Opening an entry lands on the focused viewer; full screen is a second,
    // explicit step (multi-column) reached by pressing Enter again.
    app.nav.reader_fullscreen = false;
    app.nav.focus = Focus::Reader;
    Ok(())
}

pub(super) fn open_reader_link(
    app: &mut AppModel,
    target: &str,
    heading_line: Option<usize>,
) -> AppResult<Option<Effect>> {
    if let Some(anchor) = target.strip_prefix('#') {
        let Some(line) = heading_line else {
            app.toast(
                ToastVariant::Warning,
                format!("Heading not found: {anchor}"),
            );
            return Ok(None);
        };
        app.nav.scroll.reader = line.min(u16::MAX as usize) as u16;
        app.flash_reader_heading(line);
        return Ok(None);
    }
    if !(target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with("mailto:"))
    {
        // A link into the selected entry's own asset folder — an imported
        // audio/video/pdf attachment. Hand the stored plaintext file to the OS
        // default app. (Only plaintext entries reach here; encrypted assets are
        // `.age` on disk and never record a clickable hit.)
        if let Some(path) = selected_entry_attachment_path(app, target)? {
            return Ok(Some(Effect::Open {
                target: OpenTarget::Path(path),
                success_message: "Opened attachment".to_string(),
            }));
        }
        app.toast(ToastVariant::Warning, "Unsupported link target");
        return Ok(None);
    }
    Ok(Some(Effect::Open {
        target: OpenTarget::Uri(target.to_string()),
        success_message: "Opened link".to_string(),
    }))
}

/// Resolve a reader link into the selected entry's own asset folder to an
/// absolute on-disk path, or `None` for any other target. Encrypted entries
/// return `None`: their assets live on disk as `.age` and can't be opened
/// directly.
fn selected_entry_attachment_path(app: &AppModel, target: &str) -> AppResult<Option<PathBuf>> {
    let Some(entry) = app.resolved_selected_entry() else {
        return Ok(None);
    };
    if entry.encryption_state != EntryEncryptionState::Plain {
        return Ok(None);
    }
    let Some(file_name) = notema_storage::stored_asset_reference_for(&entry.path, target) else {
        return Ok(None);
    };
    notema_storage::resolve_entry_asset_path(&entry.path, &file_name)
}

pub(super) fn delete_selected(app: &mut AppModel) -> AppResult<()> {
    if !app.reload_selected_entry_from_disk()? {
        return Ok(());
    }
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
        app.services.store.move_entry_to_trash(&target.path)?;
        app.toast(ToastVariant::Success, "Moved to trash");
    } else {
        app.services.store.delete_empty_entry(&target.path)?;
        app.toast(ToastVariant::Success, "Deleted");
    }
    Ok(())
}

pub(super) fn delete_selected_journal(app: &mut AppModel) -> AppResult<()> {
    let Some(journal) = app.selected_journal() else {
        return Ok(());
    };
    let journal_name = journal.name.clone();
    let journal_path = journal.path.clone();
    let journal_id = journal.id.clone();

    let current = app
        .services
        .store
        .list_journals()?
        .into_iter()
        .find(|candidate| candidate.id == journal_id)
        .ok_or_else(|| anyhow::anyhow!("selected journal no longer exists"))?;
    if current.name != journal_name || current.path != journal_path {
        anyhow::bail!("selected journal changed on disk; refresh and try again");
    }

    let fresh_entries = app.services.store.read_entries(
        app.services
            .store
            .collect_entry_paths()?
            .into_iter()
            .filter(|entry| entry.journal == journal_name)
            .collect(),
    )?;
    let entries: Vec<(PathBuf, bool)> = fresh_entries
        .iter()
        .map(|e| (e.path.clone(), !e.body.trim().is_empty()))
        .collect();

    let display = notema_storage::journal_display_name(&journal_name).to_string();
    app.services
        .store
        .delete_journal(&journal_name, &journal_path, &entries)?;
    app.toast(ToastVariant::Success, format!("Deleted journal {display}"));
    Ok(())
}

pub(super) fn toggle_archive_selected_journal(app: &mut AppModel) -> AppResult<()> {
    let Some(journal) = app.selected_journal() else {
        return Ok(());
    };
    let old_name = journal.name.clone();
    let journal_id = journal.id.clone();
    let archive = !journal.archived;
    let display = journal.display_name().to_string();

    let current = app
        .services
        .store
        .list_journals()?
        .into_iter()
        .find(|candidate| candidate.id == journal_id)
        .ok_or_else(|| anyhow::anyhow!("selected journal no longer exists"))?;
    if current.name != old_name || current.path != journal.path {
        anyhow::bail!("selected journal changed on disk; refresh and try again");
    }

    let new_journal = app
        .services
        .store
        .set_journal_archived(&old_name, archive)?;
    // The rename changes the journal's folder name, so the name-keyed
    // `config.journal.default` would go stale. Retarget it before reloading.
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
    app: &mut AppModel,
    kind: MetadataKind,
    values: &[String],
) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }
    if !reject_if_front_matter_invalid(app) {
        return Ok(());
    }

    let field = match kind {
        MetadataKind::Tags => MetadataField::Tags(values.to_vec()),
        MetadataKind::People => MetadataField::People(values.to_vec()),
        MetadataKind::Activities => MetadataField::Activities(values.to_vec()),
    };
    app.services
        .store
        .set_entry_metadata_field(&target.path, field)?;

    app.toast(ToastVariant::Success, format!("{} saved", kind.title()));
    refresh_entry_path(app, &target.path)?;
    Ok(())
}

pub(super) fn set_feelings_on_entry(app: &mut AppModel, feelings: &[String]) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }
    if !reject_if_front_matter_invalid(app) {
        return Ok(());
    }

    app.services
        .store
        .set_entry_metadata_field(&target.path, MetadataField::Feelings(feelings.to_vec()))?;

    app.toast(ToastVariant::Success, "Feelings saved");
    refresh_entry_path(app, &target.path)?;
    Ok(())
}

pub(super) fn set_mood_on_entry(app: &mut AppModel, mood: Option<i8>) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }
    if !reject_if_front_matter_invalid(app) {
        return Ok(());
    }

    app.services
        .store
        .set_entry_metadata_field(&target.path, MetadataField::Mood(mood))?;

    app.toast(ToastVariant::Success, "Mood saved");
    refresh_entry_path(app, &target.path)?;
    Ok(())
}

pub(super) fn set_location_on_entry(
    app: &mut AppModel,
    location: Option<Location>,
) -> AppResult<Option<crate::tui::environment::EnvironmentRequest>> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(None);
    };

    if !reject_if_locked(app, &target) {
        return Ok(None);
    }
    if !reject_if_front_matter_invalid(app) {
        return Ok(None);
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
    app.services
        .store
        .set_entry_metadata_fields(&target.path, &fields)?;

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
    Ok(match (location, datetime) {
        (Some(location), Some(datetime)) => {
            app.prepare_entry_environment_for(target.path.clone(), &location, datetime)
        }
        _ => None,
    })
}

pub(super) fn toggle_starred_on_entry(app: &mut AppModel) -> AppResult<()> {
    let Some(target) = app.selected_entry_target() else {
        return Ok(());
    };

    if !reject_if_locked(app, &target) {
        return Ok(());
    }
    if !reject_if_front_matter_invalid(app) {
        return Ok(());
    }

    let starred = !app.selected_entry_starred();
    app.services
        .store
        .set_entry_metadata_field(&target.path, MetadataField::Starred(starred))?;

    app.toast(
        ToastVariant::Success,
        if starred { "Starred" } else { "Unstarred" },
    );
    refresh_entry_path(app, &target.path)?;
    Ok(())
}

fn refresh_entry_path(app: &mut AppModel, path: &Path) -> AppResult<()> {
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

    fn new_app(config: Config) -> AppModel {
        let config_path = config.journal.path.join("config.toml");
        let store = JournalStore::for_config(&config_path, &config.journal.path).unwrap();
        AppModel::new(config_path, config, store).unwrap()
    }

    /// The newest toast as `(message, variant)`.
    fn last_toast(app: &AppModel) -> (&str, ToastVariant) {
        let toast = app.toasts.items().last().expect("a toast was pushed");
        (toast.message.as_str(), toast.variant)
    }

    #[test]
    fn save_status_reports_images_and_attachments_separately() {
        let report = notema_storage::AssetReport {
            stored: 3,
            attachments_stored: 1,
            failed: vec![notema_storage::AssetFailure::AttachmentIngest {
                source: "clip.mp4".to_string(),
                error: "gone".to_string(),
            }],
            ..notema_storage::AssetReport::default()
        };

        assert_eq!(
            save_status("Saved", &report),
            "Saved — 2 images stored — 1 attachment stored — 1 attachment not stored"
        );
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
    fn internal_reader_link_scrolls_to_and_flashes_heading() {
        let dir = tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        let mut app = new_app(config);
        open_reader_link(&mut app, "#details", Some(17)).unwrap();

        assert_eq!(app.nav.scroll.reader, 17);
        assert_eq!(app.reader_anchor_flash.as_ref().unwrap().line, 17);
    }

    #[test]
    fn unsupported_reader_link_is_rejected_without_launching() {
        let dir = tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        let mut app = new_app(config);

        open_reader_link(&mut app, "file:///tmp/private", None).unwrap();

        assert_eq!(last_toast(&app).0, "Unsupported link target");
    }

    #[test]
    fn set_feelings_on_entry_writes_front_matter_and_refreshes_app() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let path = entry_dir.join("a.md");
        fs::write(&path, "+++\nschema_version = 1\n+++\n\n# A\n").unwrap();

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
        fs::write(&path, "+++\nschema_version = 1\n+++\n\n# A\n").unwrap();

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

    fn app_with_entry(body: &str) -> (tempfile::TempDir, AppModel, PathBuf) {
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
        let (_dir, mut app, _path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");
        let expected = app.resolved_selected_entry().unwrap().body.clone();

        app.open_editor_for_selected().unwrap();

        assert!(app.editor.is_some());
        assert_eq!(app.nav.focus, Focus::Reader);
        assert_eq!(app.editor.as_ref().unwrap().text(), expected);
    }

    #[test]
    fn open_editor_reloads_entry_from_disk() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# Cached\n");
        fs::write(path, "+++\nschema_version = 1\n+++\n\n# Changed on disk\n").unwrap();

        app.open_editor_for_selected().unwrap();

        assert_eq!(app.editor.as_ref().unwrap().text(), "# Changed on disk\n");
        assert_eq!(
            app.resolved_selected_entry().unwrap().body,
            "# Changed on disk\n"
        );
    }

    #[test]
    fn malformed_entry_stays_readable_but_editor_does_not_open() {
        let (_dir, mut app, _path) =
            app_with_entry("+++\nactivities = [\"reading\"]\n+++\n\nDraft text\n");

        let (title, reader) = app.selected_reader().unwrap();
        assert!(title.starts_with("! "));
        assert!(reader.contains("missing schema_version = 1"));
        assert!(reader.contains("Draft text"));

        app.open_editor_for_selected().unwrap();

        assert!(app.editor.is_none());
        let (message, variant) = last_toast(&app);
        assert_eq!(variant, ToastVariant::Error);
        assert!(message.contains("missing schema_version = 1"));
    }

    #[test]
    fn viewing_malformed_entry_warns_without_exiting() {
        let (_dir, mut app, _path) =
            app_with_entry("+++\nactivities = [\"reading\"]\n+++\n\nDraft text\n");

        view_selected(&mut app).unwrap();

        assert_eq!(app.nav.focus, Focus::Reader);
        let (message, variant) = last_toast(&app);
        assert_eq!(variant, ToastVariant::Warning);
        assert!(message.contains("Showing the body only"));
    }

    #[test]
    fn save_internal_editor_writes_body_and_bumps_edited_at() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# Original\n");
        let target = app.selected_entry_target().unwrap();
        let journal = app.resolved_selected_entry().unwrap().journal.clone();
        let (_, revision) = app
            .services
            .store
            .read_entry_with_revision(&journal, &target.path)
            .unwrap();

        let mut editor = EntryEditor::for_existing(
            journal,
            target.path.clone(),
            target.title,
            revision,
            "# Edited body",
            notema_domain::Metadata::default(),
        );
        editor.original = "# Original".to_string();
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        assert!(app.editor.is_none());
        let content = app.services.store.read_entry_content(&path).unwrap();
        assert!(content.contains("# Edited body"));
        assert!(content.contains("edited_at"));
    }

    #[test]
    fn save_internal_editor_unchanged_body_does_not_rewrite() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# Original\n");
        let original = app.services.store.read_entry_content(&path).unwrap();

        app.open_editor_for_selected().unwrap();
        save_internal_editor(&mut app).unwrap();

        assert!(app.editor.is_none());
        assert_eq!(last_toast(&app), ("No changes", ToastVariant::Info));
        assert_eq!(
            app.services.store.read_entry_content(&path).unwrap(),
            original
        );
    }

    #[test]
    fn save_internal_editor_error_keeps_buffer_open() {
        let (_dir, mut app, path) = app_with_entry("+++\n[entry]\ntags = []\n+++\n\n# Original\n");
        let target = app.selected_entry_target().unwrap();
        let journal = app.resolved_selected_entry().unwrap().journal.clone();
        let (_, revision) = app
            .services
            .store
            .read_entry_with_revision(&journal, &path)
            .unwrap();
        let mut editor = EntryEditor::for_existing(
            journal,
            path,
            target.title,
            revision,
            "# Edited body",
            notema_domain::Metadata::default(),
        );
        editor.original = "# Original".to_string();
        editor.metadata.tags.push("cannot-write".to_string());
        app.editor = Some(editor);

        let result = save_internal_editor(&mut app);

        assert!(result.is_err());
        assert_eq!(app.editor.as_ref().unwrap().text(), "# Edited body");
    }

    #[test]
    fn external_edit_saves_buffer_as_new_entry_without_overwriting_original() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# Original\n");
        app.open_editor_for_selected().unwrap();
        app.editor
            .as_mut()
            .unwrap()
            .textarea
            .insert_str("My long edit");
        let external = "+++\nschema_version = 1\n+++\n\n# External\n";
        fs::write(&path, external).unwrap();

        save_internal_editor(&mut app).unwrap();

        assert!(app.editor.is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        let entries = app.services.store.scan_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|entry| entry.body.contains("My long edit"))
        );
        let (message, variant) = last_toast(&app);
        assert_eq!(variant, ToastVariant::Warning);
        assert!(message.contains("saved your edit as a new entry"));
    }

    #[test]
    fn unchanged_editor_does_not_copy_an_externally_changed_entry() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# Original\n");
        app.open_editor_for_selected().unwrap();
        let external = "+++\nschema_version = 1\n+++\n\n# External\n";
        fs::write(&path, external).unwrap();

        save_internal_editor(&mut app).unwrap();

        assert!(app.editor.is_none());
        assert_eq!(last_toast(&app), ("No changes", ToastVariant::Info));
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        assert_eq!(app.services.store.scan_entries().unwrap().len(), 1);
        assert_eq!(app.resolved_selected_entry().unwrap().body, "# External\n");
    }

    #[test]
    fn save_internal_editor_empty_body_deletes_existing_entry() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");
        let target = app.selected_entry_target().unwrap();
        let journal = app.resolved_selected_entry().unwrap().journal.clone();
        let (_, revision) = app
            .services
            .store
            .read_entry_with_revision(&journal, &target.path)
            .unwrap();

        let mut editor = EntryEditor::for_existing(
            journal,
            target.path.clone(),
            target.title,
            revision,
            "   \n  ",
            notema_domain::Metadata::default(),
        );
        editor.original = "# A\n".to_string();
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        assert!(!path.exists());
        assert_eq!(
            last_toast(&app),
            ("Empty entry deleted", ToastVariant::Success)
        );
    }

    #[test]
    fn save_internal_editor_creates_new_entry() {
        let (_dir, mut app, _path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");

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
        use notema_context::compute_celestial;
        use notema_domain::Location;

        let (_dir, mut app, _path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");

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
            celestial: compute_celestial(
                notema_domain::Coordinates::try_new(52.52, 13.405).unwrap(),
                datetime,
            ),
            weather: None,
            air_quality: None,
            warnings: Vec::new(),
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
    fn new_located_entry_is_stamped_with_its_place_timezone() {
        use notema_domain::Location;

        let (_dir, mut app, _path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");

        let mut editor = EntryEditor::for_new("work".to_string());
        editor.textarea.insert_str("In Tokyo");
        editor.metadata.location = Some(Location {
            latitude: Some(35.68),
            longitude: Some(139.767),
            ..Location::default()
        });
        // The resolved zone the location dialog would have stored for this place.
        editor.zone = Some(chrono_tz::Tz::Asia__Tokyo);
        app.editor = Some(editor);
        save_internal_editor(&mut app).unwrap();

        let entry = app
            .library
            .entries
            .iter()
            .find(|entry| entry.body.contains("In Tokyo"))
            .expect("entry created");
        // The timestamp carries Tokyo's offset, not the machine's.
        assert_eq!(
            entry.created_time().unwrap().offset().local_minus_utc(),
            9 * 3600
        );
        // And the IANA name is recorded on disk.
        let raw = std::fs::read_to_string(&entry.path).unwrap();
        assert!(
            raw.contains("timezone = \"Asia/Tokyo\""),
            "entry front-matter records the place's zone: {raw}"
        );
    }

    #[test]
    fn new_entry_save_defers_while_context_pending() {
        let (_dir, mut app, _path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");

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
    fn direct_location_set_fires_an_environment_fetch() {
        // Dated (so a weather lookup can fire) but initially locationless.
        let body = "+++\nschema_version = 1\n[time]\ncreated_at = \"2026-07-01T10:00:00+00:00\"\n+++\n\n# A\n";
        let (_dir, mut app, _path) = app_with_entry(body);

        let location = Location {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Location::default()
        };
        // Setting a location on an entry captures its environment there and then —
        // this live capture is what replaces the old automatic sweep.
        let request = set_location_on_entry(&mut app, Some(location)).unwrap();
        assert!(request.is_some());
    }

    #[test]
    fn open_editor_seeds_buffered_metadata_from_entry() {
        let (_dir, mut app, _path) =
            app_with_entry("+++\nschema_version = 1\n\n[entry]\ntags = [\"seed\"]\n+++\n\n# A\n");
        app.open_editor_for_selected().unwrap();
        assert_eq!(
            app.editor.as_ref().unwrap().metadata.tags,
            vec!["seed".to_string()]
        );
    }

    #[test]
    fn save_internal_editor_applies_buffered_metadata_to_existing_entry() {
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");
        app.open_editor_for_selected().unwrap();
        app.set_editor_metadata(
            MetadataKind::Tags,
            &["work".to_string(), "focus".to_string()],
        );

        save_internal_editor(&mut app).unwrap();

        let content = app.services.store.read_entry_content(&path).unwrap();
        assert!(content.contains("work"), "front matter was: {content}");
        assert!(content.contains("focus"), "front matter was: {content}");
    }

    #[test]
    fn save_internal_editor_writes_buffered_metadata_for_new_entry() {
        let (_dir, mut app, _path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");

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
        let (_dir, mut app, path) = app_with_entry("+++\nschema_version = 1\n+++\n\n# A\n");

        app.open_editor_for_selected().unwrap();
        app.editor.as_mut().unwrap().textarea.insert_str("mutation");
        assert!(app.editor.as_ref().unwrap().is_dirty());

        app.cancel_editor();

        assert!(app.editor.is_none());
        let content = app.services.store.read_entry_content(&path).unwrap();
        assert!(!content.contains("mutation"));
    }
}
