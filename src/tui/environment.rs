//! Background environment lookups — weather and air quality — over the shared
//! [`Worker`], fired when an entry's location is set or changed. Each request
//! carries the target
//! entry's path so the reply is persisted to the right file regardless of what's
//! selected by the time it lands, and a per-path id so a rapid re-request
//! supersedes an earlier one and stale replies are dropped.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use chrono::{DateTime, FixedOffset};
use journal_context_provider::{fetch_air_quality, fetch_weather};
use journal_core::{AirQuality, Weather};

use crate::tui::worker::Worker;

/// An environment lookup (weather + air quality) for one entry, tagged with the
/// id the worker assigned.
pub(crate) struct EnvironmentRequest {
    id: u64,
    path: PathBuf,
    lat: f64,
    lon: f64,
    datetime: DateTime<FixedOffset>,
}

/// A finished lookup. Each field is `Ok(None)` when Open-Meteo had no sample, or
/// `Err` on a network failure — both are dropped when draining. Weather and air
/// quality come from separate endpoints, so one can land while the other doesn't.
struct EnvironmentResult {
    id: u64,
    path: PathBuf,
    weather: Result<Option<Weather>, String>,
    air_quality: Result<Option<AirQuality>, String>,
}

/// Background environment worker, plus the per-path supersede bookkeeping: `next_id`
/// hands out request ids and `latest` records the most recent id dispatched for
/// each entry, so a reply is kept only if it's still the newest for its path.
#[derive(Default)]
pub(crate) struct EnvironmentWorker {
    worker: Worker<EnvironmentRequest, EnvironmentResult>,
    next_id: u64,
    latest: HashMap<PathBuf, u64>,
}

impl EnvironmentWorker {
    /// Dispatch a lookup for `path`, superseding any earlier one for it.
    pub(crate) fn request(
        &mut self,
        path: PathBuf,
        lat: f64,
        lon: f64,
        datetime: DateTime<FixedOffset>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        self.latest.insert(path.clone(), id);
        self.worker.request(
            EnvironmentRequest {
                id,
                path,
                lat,
                lon,
                datetime,
            },
            resolve,
        );
    }

    /// Forget any in-flight lookup for `path` so its eventual reply is dropped —
    /// used when the entry's location is cleared.
    pub(crate) fn forget(&mut self, path: &Path) {
        self.latest.remove(path);
    }

    /// Drain finished lookups, yielding `(path, weather, air_quality)` for replies
    /// that are still the newest for their entry and carry at least one of the two
    /// (each is `None` when its endpoint had no sample or failed).
    pub(crate) fn drain(&mut self) -> Vec<(PathBuf, Option<Weather>, Option<AirQuality>)> {
        self.worker
            .drain()
            .into_iter()
            .filter_map(|result| {
                if self.latest.get(&result.path) != Some(&result.id) {
                    return None;
                }
                self.latest.remove(&result.path);
                let weather = result.weather.ok().flatten();
                let air_quality = result.air_quality.ok().flatten();
                if weather.is_none() && air_quality.is_none() {
                    return None;
                }
                Some((result.path, weather, air_quality))
            })
            .collect()
    }

    /// Whether a lookup is still outstanding.
    pub(crate) fn has_pending(&self) -> bool {
        self.worker.has_pending()
    }
}

/// Resolve one request: weather and air quality from their separate endpoints.
/// Runs on the worker thread.
fn resolve(request: EnvironmentRequest) -> EnvironmentResult {
    let weather = fetch_weather(request.lat, request.lon, request.datetime)
        .map_err(|error| error.to_string());
    let air_quality = fetch_air_quality(request.lat, request.lon, request.datetime)
        .map_err(|error| error.to_string());
    EnvironmentResult {
        id: request.id,
        path: request.path,
        weather,
        air_quality,
    }
}
