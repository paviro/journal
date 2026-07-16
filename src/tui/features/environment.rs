use std::path::{Path, PathBuf};

use crate::tui::app::AppModel;
use crate::tui::editor_state::EditorTarget;
use crate::tui::environment::{EnvironmentRequest, EnvironmentTarget, environment_fields};
use chrono::{DateTime, FixedOffset, Local};
use notema_domain::{Coordinates, Location};

/// The `(lat, lon)` of a location, or `None` when it isn't pinned to coordinates.
pub(crate) fn coords(location: &Location) -> Option<Coordinates> {
    location.coordinates()
}

impl AppModel {
    /// The time a fetched environment should be dated to: now for a new entry, the
    /// edited entry's own creation time otherwise.
    fn editor_context_datetime(&self) -> DateTime<FixedOffset> {
        match self.editor.as_ref().map(|editor| &editor.target) {
            Some(EditorTarget::Existing { .. }) => self
                .resolved_selected_entry()
                .and_then(|entry| entry.created_time())
                .unwrap_or_else(|| Local::now().fixed_offset()),
            _ => Local::now().fixed_offset(),
        }
    }

    /// Spawn a background environment fetch for the open editor's current location,
    /// bumping the request id so a stale reply for an older location is dropped.
    /// A cleared or coordless location just abandons any pending fetch and result.
    pub(crate) fn prepare_editor_environment(&mut self) -> Option<EnvironmentRequest> {
        let mut datetime = self.editor_context_datetime();
        let coordinates = {
            let editor = self.editor.as_mut()?;
            let Some(coordinates) = editor.metadata.location.as_ref().and_then(coords) else {
                editor.pending_environment = None;
                editor.environment = None;
                return None;
            };
            // Date the fetch to the place's local day when the entry is timezoned,
            // so sunrise/sunset and the weather sample match where it was written.
            if let Some(zone) = editor.zone {
                datetime = notema_context::rezone(datetime, zone);
            }
            coordinates
        };
        // Allocate from the app-level counter so a stale result from a discarded
        // editor session can't share an id with this one and be mistaken for it.
        self.next_environment_id += 1;
        let id = self.next_environment_id;
        if let Some(editor) = self.editor.as_mut() {
            editor.pending_environment = Some(id);
            editor.environment = None;
        }
        Some(EnvironmentRequest {
            id,
            coordinates,
            datetime,
            target: EnvironmentTarget::Editor,
        })
    }

    /// Spawn a background environment fetch whose result is written back to `path`
    /// (a location just set directly on an entry). Returns the request id.
    fn prepare_entry_environment_request(
        &mut self,
        path: PathBuf,
        coordinates: Coordinates,
        datetime: DateTime<FixedOffset>,
    ) -> EnvironmentRequest {
        self.next_environment_id += 1;
        let id = self.next_environment_id;
        EnvironmentRequest {
            id,
            coordinates,
            datetime,
            target: EnvironmentTarget::Entry(path),
        }
    }

    /// Kick off a background environment fetch for a location just set directly on an
    /// entry (no editor). The result is written back when it lands. No-op when the
    /// location has no coordinates.
    pub(crate) fn prepare_entry_environment_for(
        &mut self,
        path: PathBuf,
        location: &Location,
        datetime: DateTime<FixedOffset>,
    ) -> Option<EnvironmentRequest> {
        coords(location)
            .map(|coordinates| self.prepare_entry_environment_request(path, coordinates, datetime))
    }

    /// Whether the entry at `path` is the one currently open in the editor — a
    /// background write-back must not clobber the live buffer.
    pub(crate) fn editor_is_editing(&self, path: &Path) -> bool {
        matches!(
            self.editor.as_ref().map(|editor| &editor.target),
            Some(EditorTarget::Existing { path: open, .. }) if open == path
        )
    }

    /// Drain finished environment lookups and route each one: editor results attach to
    /// the draft (matched by id), entry results are written back to their file.
    /// Returns whether anything changed, so the event loop knows to repaint.
    pub(crate) fn apply_environment_results(&mut self) -> bool {
        let results = self.environment.drain();
        let mut changed = false;
        for result in results {
            match result.target {
                EnvironmentTarget::Editor => {
                    if let Some(editor) = self.editor.as_mut()
                        && editor.pending_environment == Some(result.id)
                    {
                        editor.pending_environment = None;
                        editor.environment = Some(result.environment);
                        changed = true;
                    }
                }
                EnvironmentTarget::Entry(path) => {
                    // Skip an entry open in the editor; it's captured on its save.
                    if self.editor_is_editing(&path) {
                        continue;
                    }
                    let fields = environment_fields(&result.environment);
                    if !fields.is_empty()
                        && self
                            .services
                            .store
                            .set_entry_metadata_fields_quiet(&path, &fields)
                            .is_ok()
                    {
                        // Reload just this entry so the new environment shows at once.
                        let _ = self.refresh_paths(&[path]);
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}
