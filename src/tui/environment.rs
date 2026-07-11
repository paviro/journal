//! Background weather/air-quality/celestial fetching over the shared [`Worker`].
//! A located entry's environment is fetched off the event loop — spawned when a
//! location is picked (attached to the entry on save) or paced out at startup to
//! backfill entries that never had it — so no save ever blocks on the network.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use notema_context_provider::{compute_celestial, fetch_air_quality, fetch_weather};
use notema_core::{AirQuality, Celestial, MetadataField, Weather};

use crate::tui::worker::Worker;

/// The background environment worker, spawned on first use.
pub(crate) type EnvironmentWorker = Worker<EnvironmentRequest, EnvironmentResult>;

/// The captured environment for one located entry: celestial is computed
/// offline, weather and air quality come from Open-Meteo (either may be absent
/// when there's no data for that place/time).
#[derive(Default, Clone)]
pub(crate) struct Environment {
    pub(crate) celestial: Option<Celestial>,
    pub(crate) weather: Option<Weather>,
    pub(crate) air_quality: Option<AirQuality>,
}

/// Where a finished lookup's data belongs, so the drain step can route it.
#[derive(Clone)]
pub(crate) enum EnvironmentTarget {
    /// Attach to the open editor's draft, matched by the request id.
    Editor,
    /// Write back to this entry file (direct location-set or parse-time backfill).
    Entry(PathBuf),
}

/// A environment lookup handed to the worker, tagged with an id and its destination.
pub(crate) struct EnvironmentRequest {
    pub(crate) id: u64,
    pub(crate) lat: f64,
    pub(crate) lon: f64,
    pub(crate) datetime: DateTime<FixedOffset>,
    pub(crate) target: EnvironmentTarget,
}

/// A finished environment lookup coming back to the event loop.
pub(crate) struct EnvironmentResult {
    pub(crate) id: u64,
    pub(crate) target: EnvironmentTarget,
    pub(crate) environment: Environment,
}

/// Fetch the full environment for a place and time. Celestial is offline and always
/// present; weather/air quality are dropped to `None` on no-data or transport
/// failure (the caller can't do anything with the error mid-save).
pub(crate) fn fetch_entry_environment(
    lat: f64,
    lon: f64,
    datetime: DateTime<FixedOffset>,
) -> Environment {
    Environment {
        celestial: Some(compute_celestial(lat, lon, datetime)),
        weather: fetch_weather(lat, lon, datetime).ok().flatten(),
        air_quality: fetch_air_quality(lat, lon, datetime).ok().flatten(),
    }
}

/// Resolve one environment request. Runs on the worker thread.
pub(crate) fn resolve(request: EnvironmentRequest) -> EnvironmentResult {
    let environment = fetch_entry_environment(request.lat, request.lon, request.datetime);
    EnvironmentResult {
        id: request.id,
        target: request.target,
        environment,
    }
}

/// The metadata fields to persist for a fetched environment — only the parts that
/// came back present, so an absent weather/air reading isn't written as cleared.
pub(crate) fn environment_fields(environment: &Environment) -> Vec<MetadataField> {
    let mut fields = Vec::new();
    if let Some(celestial) = &environment.celestial {
        fields.push(MetadataField::Celestial(Some(Box::new(celestial.clone()))));
    }
    if let Some(weather) = &environment.weather {
        fields.push(MetadataField::Weather(Some(Box::new(weather.clone()))));
    }
    if let Some(air_quality) = &environment.air_quality {
        fields.push(MetadataField::AirQuality(Some(Box::new(
            air_quality.clone(),
        ))));
    }
    fields
}
