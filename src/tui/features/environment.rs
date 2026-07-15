use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::tui::app::AppModel;
use crate::tui::editor_state::EditorTarget;
use crate::tui::environment::{EnvironmentRequest, EnvironmentTarget, environment_fields};
use chrono::{DateTime, FixedOffset, Local};
use notema_domain::{Coordinates, EntryEncryptionState, Location};

/// Minimum gap between backfill dispatches — the Open-Meteo rate-limit knob.
/// One entry per second (two calls) stays well under the free-tier ceiling.
const BACKFILL_THROTTLE: Duration = Duration::from_secs(1);

/// The paced environment-backfill state: entries with a location but no captured
/// environment, fetched one at a time so the interactive fetch never queues behind
/// a batch and Open-Meteo isn't spammed. See [`AppModel::prepare_environment_backfill`].
#[derive(Default)]
pub(crate) struct EnvironmentBackfill {
    /// Entries awaiting a backfill fetch, paced out one at a time.
    pub(crate) queue: VecDeque<PathBuf>,
    /// Paths ever queued this session, so a re-scan can't re-enqueue one still in
    /// flight (its environment lands and clears the predicate later).
    pub(crate) enqueued: HashSet<PathBuf>,
    /// The id of the backfill request currently in flight, if any.
    pub(crate) inflight: Option<u64>,
    /// When the last backfill request was dispatched, for throttling.
    last_dispatch: Option<Instant>,
}

/// The `(lat, lon)` of a location, or `None` when it isn't pinned to coordinates.
fn coords(location: &Location) -> Option<Coordinates> {
    location.coordinates()
}

/// Whether the store can read and rewrite this entry (unlocked or plaintext).
fn is_writable(state: EntryEncryptionState) -> bool {
    matches!(
        state,
        EntryEncryptionState::Plain | EntryEncryptionState::EncryptedUnlocked
    )
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
    /// (a direct location-set, or a paced backfill). Returns the request id.
    ///
    /// Claims `path` in the backfill ledger and drops any queued copy, so this
    /// fetch is the sole owner of the entry's environment: a location-set that
    /// also enqueued the entry for backfill can't then fetch it a second time.
    fn prepare_entry_environment_request(
        &mut self,
        path: PathBuf,
        coordinates: Coordinates,
        datetime: DateTime<FixedOffset>,
    ) -> EnvironmentRequest {
        self.backfill.enqueued.insert(path.clone());
        self.backfill.queue.retain(|queued| queued != &path);
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
    fn editor_is_editing(&self, path: &Path) -> bool {
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
                    if self.backfill.inflight == Some(result.id) {
                        self.backfill.inflight = None;
                    }
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

    /// Queue any writable, located entry that never got environment (celestial absent
    /// marks it, since celestial is always computable). Called after entries load
    /// or refresh; already-queued paths are skipped so a re-scan can't double up.
    pub(crate) fn enqueue_environment_backfill(&mut self) {
        let targets: Vec<PathBuf> = self
            .library
            .entries
            .iter()
            .filter(|entry| entry.celestial.is_none())
            .filter(|entry| is_writable(entry.encryption_state))
            .filter(|entry| entry.location.as_ref().and_then(coords).is_some())
            .map(|entry| entry.path.clone())
            .filter(|path| !self.backfill.enqueued.contains(path))
            .collect();
        for path in targets {
            self.backfill.enqueued.insert(path.clone());
            self.backfill.queue.push_back(path);
        }
    }

    /// Dispatch at most one queued backfill fetch, throttled and one-at-a-time so
    /// the interactive worker never queues behind a batch. Call each event-loop
    /// tick; cheap when the queue is empty or a job is already in flight.
    pub(crate) fn prepare_environment_backfill(&mut self) -> Option<EnvironmentRequest> {
        if self.backfill.inflight.is_some() {
            return None;
        }
        if self
            .backfill
            .last_dispatch
            .is_some_and(|at| at.elapsed() < BACKFILL_THROTTLE)
        {
            return None;
        }
        while let Some(path) = self.backfill.queue.pop_front() {
            // An entry now open in the editor is captured on its own save.
            if self.editor_is_editing(&path) {
                continue;
            }
            let params = self
                .library
                .entries
                .iter()
                .find(|entry| entry.path == path)
                // Skip if it already has environment — e.g. an editor edit attached
                // it after this path was queued, so the queued job is now stale.
                .filter(|entry| entry.celestial.is_none())
                .and_then(|entry| {
                    let coordinates = coords(entry.location.as_ref()?)?;
                    let datetime = entry
                        .created_time()
                        .unwrap_or_else(|| Local::now().fixed_offset());
                    Some((coordinates, datetime))
                });
            let Some((coordinates, datetime)) = params else {
                continue;
            };
            let request = self.prepare_entry_environment_request(path, coordinates, datetime);
            self.backfill.inflight = Some(request.id);
            self.backfill.last_dispatch = Some(Instant::now());
            return Some(request);
        }
        None
    }

    /// Whether backfill work is pending — a job in flight or entries still queued —
    /// so the event loop can wake promptly to pace the next dispatch.
    pub(crate) fn environment_backfill_active(&self) -> bool {
        self.backfill.inflight.is_some() || !self.backfill.queue.is_empty()
    }
}
