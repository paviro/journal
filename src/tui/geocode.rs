//! Background geocoding for the location dialog, over the shared [`Worker`]. The
//! worker resolves requests serially, which also keeps us under Nominatim's
//! one-request-per-second ceiling.

use crate::tui::worker::Worker;
use journal_storage::{GeocodeHit, geocode, reverse_geocode};

/// How many forward-geocode candidates to request (Nominatim `limit`).
const CANDIDATE_LIMIT: usize = 6;

/// The background geocoding worker, spawned on first use.
pub(crate) type GeocodeWorker = Worker<GeocodeRequest, GeocodeResult>;

/// What a request wants resolved: a typed address, or coordinates to name.
pub(crate) enum GeocodeQuery {
    Address(String),
    Coords { lat: f64, lon: f64 },
}

/// A lookup handed to the worker, tagged with the request id the dialog assigned.
pub(crate) struct GeocodeRequest {
    pub(crate) id: u64,
    pub(crate) query: GeocodeQuery,
}

/// A finished lookup coming back. `hits` holds the candidates (forward) or the
/// zero/one reverse result; `Err` carries a human-readable failure for the
/// status line.
pub(crate) struct GeocodeResult {
    pub(crate) id: u64,
    pub(crate) reverse: bool,
    pub(crate) hits: Result<Vec<GeocodeHit>, String>,
}

/// Resolve one geocoding request. Runs on the worker thread.
pub(crate) fn resolve(request: GeocodeRequest) -> GeocodeResult {
    let (reverse, hits) = match request.query {
        GeocodeQuery::Address(query) => (
            false,
            geocode(&query, CANDIDATE_LIMIT).map_err(|error| error.to_string()),
        ),
        GeocodeQuery::Coords { lat, lon } => (
            true,
            reverse_geocode(lat, lon)
                .map(|hit| hit.into_iter().collect())
                .map_err(|error| error.to_string()),
        ),
    };
    GeocodeResult {
        id: request.id,
        reverse,
        hits,
    }
}
