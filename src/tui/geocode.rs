//! Background geocoding for the location dialog, over the shared [`Worker`]. The
//! worker resolves requests serially, which also keeps us under Nominatim's
//! one-request-per-second ceiling.

use crate::tui::worker::Worker;
use notema_context::{DeviceFix, GeocodeHit, device_location, geocode, reverse_geocode};
use notema_domain::Coordinates;

/// How many forward-geocode candidates to request (Nominatim `limit`).
const CANDIDATE_LIMIT: usize = 6;

/// The background geocoding worker, spawned on first use.
pub(crate) type GeocodeWorker = Worker<GeocodeRequest, GeocodeResult>;

/// What a request wants resolved: a typed address, coordinates to name, or the
/// device's own current location (which is then named like any coordinates).
pub(crate) enum GeocodeQuery {
    Address(String),
    Coordinates(Coordinates),
    Device,
}

/// A lookup handed to the worker, tagged with the request id the dialog assigned.
pub(crate) struct GeocodeRequest {
    pub(crate) id: u64,
    pub(crate) query: GeocodeQuery,
}

/// A finished lookup coming back. `hits` holds the candidates (forward) or the
/// zero/one reverse result; `Err` carries a human-readable failure for the
/// status line. `device_fix` is what a `Device` request grabbed, so the dialog
/// can seed its coordinates (and record accuracy/source) before the reverse
/// names are applied.
pub(crate) struct GeocodeResult {
    pub(crate) id: u64,
    pub(crate) reverse: bool,
    pub(crate) hits: Result<Vec<GeocodeHit>, String>,
    pub(crate) device_fix: Option<DeviceFix>,
}

/// Resolve one geocoding request. Runs on the worker thread.
pub(crate) fn resolve(request: GeocodeRequest) -> GeocodeResult {
    let mut device_fix = None;
    let (reverse, hits) = match request.query {
        GeocodeQuery::Address(query) => (
            false,
            geocode(&query, CANDIDATE_LIMIT).map_err(|error| error.to_string()),
        ),
        GeocodeQuery::Coordinates(coordinates) => (true, reverse_hits(coordinates)),
        // Grab the device's position, then name it through the same reverse path.
        GeocodeQuery::Device => match device_location() {
            Ok(fix) => {
                let hits = reverse_hits(fix.coordinates);
                device_fix = Some(fix);
                (true, hits)
            }
            Err(error) => (true, Err(error.to_string())),
        },
    };
    GeocodeResult {
        id: request.id,
        reverse,
        hits,
        device_fix,
    }
}

/// Reverse-geocode coordinates into the zero-or-one hit the dialog expects.
fn reverse_hits(coordinates: Coordinates) -> Result<Vec<GeocodeHit>, String> {
    reverse_geocode(coordinates)
        .map(|hit| hit.into_iter().collect())
        .map_err(|error| error.to_string())
}
