//! Paced reverse-geocode backfill: entries that carry only coordinates (a device
//! grab whose reverse lookup failed, or imported lat/lon) get an address filled in
//! from Nominatim, one at a time. It rides the shared [`GeocodeWorker`], so the
//! interactive dialog and this batch stay under Nominatim's one-request-per-second
//! ceiling; the result is written back quietly, without touching `edited_at`.

use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    time::{Duration, Instant},
};

use notema_context::GeocodeHit;
use notema_domain::{Location, MetadataField};

use crate::tui::app::AppModel;
use crate::tui::features::environment::{coords, is_writable};
use crate::tui::geocode::{GeocodeQuery, GeocodeRequest, GeocodeResult, GeocodeTarget};

/// Minimum gap between backfill dispatches — Nominatim asks for at most one
/// request per second, and the batch defers to interactive lookups besides.
const BACKFILL_THROTTLE: Duration = Duration::from_secs(1);

/// The paced address-backfill state: entries with coordinates but no named parts,
/// reverse-geocoded one at a time. See [`AppModel::prepare_address_backfill`].
#[derive(Default)]
pub(crate) struct AddressBackfill {
    /// Entries awaiting a reverse lookup, paced out one at a time.
    queue: VecDeque<PathBuf>,
    /// Paths ever queued this session, so a re-scan can't re-enqueue one still in
    /// flight (its names land and clear the predicate later).
    enqueued: HashSet<PathBuf>,
    /// The id of the backfill request currently in flight, if any.
    inflight: Option<u64>,
    /// When the last backfill request was dispatched, for throttling.
    last_dispatch: Option<Instant>,
}

/// Merge a reverse-geocoded hit onto an entry's location: the names come from the
/// hit, but the entry's own coordinates (and any device-fix accuracy/source) always
/// win — reverse geocoding never moves the pin.
fn merged_location(entry: &Location, hit: GeocodeHit) -> Location {
    hit.location.with_pin_from(entry)
}

impl AppModel {
    /// Queue any writable entry that has coordinates but no named parts — a pin still
    /// awaiting a reverse lookup. Called after entries load or refresh; already-queued
    /// paths are skipped so a re-scan can't double up.
    pub(crate) fn enqueue_address_backfill(&mut self) {
        let targets: Vec<PathBuf> = self
            .library
            .entries
            .iter()
            .filter(|entry| is_writable(entry.encryption_state))
            .filter(|entry| {
                entry.location.as_ref().is_some_and(|location| {
                    coords(location).is_some() && !location.has_named_parts()
                })
            })
            .map(|entry| entry.path.clone())
            .filter(|path| !self.address_backfill.enqueued.contains(path))
            .collect();
        for path in targets {
            self.address_backfill.enqueued.insert(path.clone());
            self.address_backfill.queue.push_back(path);
        }
    }

    /// Dispatch at most one queued reverse lookup, throttled and one-at-a-time, and
    /// only while no interactive lookup is pending so the dialog never queues behind
    /// the batch. Call each event-loop tick; cheap when idle.
    pub(crate) fn prepare_address_backfill(&mut self) -> Option<GeocodeRequest> {
        if self.address_backfill.inflight.is_some() {
            return None;
        }
        // Defer to the location dialog — it shares this worker.
        if self.geocode.has_pending() {
            return None;
        }
        if self
            .address_backfill
            .last_dispatch
            .is_some_and(|at| at.elapsed() < BACKFILL_THROTTLE)
        {
            return None;
        }
        while let Some(path) = self.address_backfill.queue.pop_front() {
            // An entry open in the editor is the dialog's to name.
            if self.editor_is_editing(&path) {
                continue;
            }
            let coordinates = self
                .library
                .entries
                .iter()
                .find(|entry| entry.path == path)
                .and_then(|entry| entry.location.as_ref())
                // Skip if it was named since it was queued (e.g. a location edit).
                .filter(|location| !location.has_named_parts())
                .and_then(coords);
            let Some(coordinates) = coordinates else {
                continue;
            };
            self.next_geocode_id += 1;
            let id = self.next_geocode_id;
            self.address_backfill.inflight = Some(id);
            self.address_backfill.last_dispatch = Some(Instant::now());
            return Some(GeocodeRequest {
                id,
                query: GeocodeQuery::Coordinates(coordinates),
                target: GeocodeTarget::Entry(path),
            });
        }
        None
    }

    /// Write one finished reverse lookup back to its entry file. A missing or failed
    /// lookup leaves the entry as-is, to be retried next session. Returns whether the
    /// entry changed, so the event loop knows to repaint.
    pub(crate) fn apply_address_result(&mut self, result: GeocodeResult) -> bool {
        let GeocodeTarget::Entry(path) = result.target else {
            return false;
        };
        if self.address_backfill.inflight == Some(result.id) {
            self.address_backfill.inflight = None;
        }
        // Skip an entry open in the editor; the dialog owns its location.
        if self.editor_is_editing(&path) {
            return false;
        }
        let hit = match result.hits {
            Ok(hits) if result.reverse => hits.into_iter().next(),
            // Nominatim had no data, or the lookup failed: retried next session.
            _ => None,
        };
        let Some(hit) = hit else {
            return false;
        };
        let Some(entry_location) = self
            .library
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .and_then(|entry| entry.location.clone())
        else {
            return false;
        };
        // The location was named while the lookup was in flight — leave it be.
        if entry_location.has_named_parts() {
            return false;
        }
        let merged = merged_location(&entry_location, hit);
        let fields = [MetadataField::Location(Some(Box::new(merged)))];
        if self
            .services
            .store
            .set_entry_metadata_fields_quiet(&path, &fields)
            .is_ok()
        {
            // Reload just this entry so the new address shows at once.
            let _ = self.refresh_paths(&[path]);
            return true;
        }
        false
    }

    /// Whether backfill work is pending — a job in flight or entries still queued —
    /// so the event loop can wake promptly to pace the next dispatch.
    pub(crate) fn address_backfill_active(&self) -> bool {
        self.address_backfill.inflight.is_some() || !self.address_backfill.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::test_support::new_app;
    use notema_domain::Coordinates;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    /// Build an app over a `work` journal holding one entry per body, selected.
    fn app_with_bodies(bodies: &[&str]) -> (TempDir, AppModel, Vec<PathBuf>) {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let paths: Vec<PathBuf> = bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                let path = entry_dir.join(format!("{index}.md"));
                fs::write(&path, body).unwrap();
                path
            })
            .collect();
        let mut app = new_app(Config::new(dir.path().to_path_buf()));
        app.select_journal_by_name("work");
        (dir, app, paths)
    }

    const COORDS_ONLY: &str =
        "+++\nschema_version = 1\n[location]\nlatitude = 52.52\nlongitude = 13.405\n+++\n\n# A\n";
    const NAMED: &str = "+++\nschema_version = 1\n[location]\nlatitude = 48.0\nlongitude = 2.0\ncity = \"Paris\"\n+++\n\n# B\n";
    const NO_LOCATION: &str = "+++\nschema_version = 1\n+++\n\n# C\n";

    fn entry_location<'a>(app: &'a AppModel, path: &PathBuf) -> Option<&'a Location> {
        app.library
            .entries
            .iter()
            .find(|entry| &entry.path == path)
            .and_then(|entry| entry.location.as_ref())
    }

    #[test]
    fn enqueue_picks_only_coordinate_only_writable_entries() {
        let (_dir, app, paths) = app_with_bodies(&[COORDS_ONLY, NAMED, NO_LOCATION]);
        // Only the coords-only entry is a backfill target; the named one is done and
        // the locationless one has nothing to reverse-geocode.
        assert_eq!(app.address_backfill.queue.len(), 1);
        assert!(app.address_backfill.queue.contains(&paths[0]));
        assert!(!app.address_backfill.queue.contains(&paths[1]));
        assert!(!app.address_backfill.queue.contains(&paths[2]));
    }

    #[test]
    fn prepare_dispatches_a_reverse_lookup_for_the_queued_entry() {
        let (_dir, mut app, paths) = app_with_bodies(&[COORDS_ONLY]);
        let request = app.prepare_address_backfill().expect("a request");
        assert_eq!(request.target, GeocodeTarget::Entry(paths[0].clone()));
        assert_eq!(
            request.query,
            GeocodeQuery::Coordinates(Coordinates::try_new(52.52, 13.405).unwrap())
        );
        assert_eq!(app.address_backfill.inflight, Some(request.id));
    }

    #[test]
    fn prepare_defers_while_an_interactive_lookup_is_pending() {
        let (_dir, mut app, _paths) = app_with_bodies(&[COORDS_ONLY]);
        // A dialog lookup in flight over the shared worker: the batch must wait.
        app.geocode.request(
            GeocodeRequest {
                id: 999,
                query: GeocodeQuery::Coordinates(Coordinates::try_new(0.0, 0.0).unwrap()),
                target: GeocodeTarget::Dialog,
            },
            |request| GeocodeResult {
                id: request.id,
                reverse: true,
                hits: Ok(Vec::new()),
                device_fix: None,
                target: request.target,
            },
        );
        assert!(app.prepare_address_backfill().is_none());
    }

    #[test]
    fn apply_writes_names_but_keeps_the_entry_coordinates() {
        let (_dir, mut app, paths) = app_with_bodies(&[COORDS_ONLY]);
        let hit = GeocodeHit {
            display_name: "Unter den Linden, Berlin".to_string(),
            location: Location {
                road: Some("Unter den Linden".to_string()),
                city: Some("Berlin".to_string()),
                // A different pin — the entry's own coordinates must win.
                latitude: Some(1.0),
                longitude: Some(2.0),
                ..Location::default()
            },
            timezone: None,
        };
        let result = GeocodeResult {
            id: 1,
            reverse: true,
            hits: Ok(vec![hit]),
            device_fix: None,
            target: GeocodeTarget::Entry(paths[0].clone()),
        };
        assert!(app.apply_address_result(result));
        let location = entry_location(&app, &paths[0]).expect("location");
        assert_eq!(location.city.as_deref(), Some("Berlin"));
        assert_eq!(location.road.as_deref(), Some("Unter den Linden"));
        assert_eq!(location.latitude, Some(52.52));
        assert_eq!(location.longitude, Some(13.405));
    }

    #[test]
    fn apply_leaves_the_entry_untouched_when_reverse_finds_nothing() {
        let (_dir, mut app, paths) = app_with_bodies(&[COORDS_ONLY]);
        let result = GeocodeResult {
            id: 1,
            reverse: true,
            hits: Ok(Vec::new()),
            device_fix: None,
            target: GeocodeTarget::Entry(paths[0].clone()),
        };
        assert!(!app.apply_address_result(result));
        let location = entry_location(&app, &paths[0]).expect("location");
        assert!(!location.has_named_parts());
    }
}
