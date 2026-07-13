#![forbid(unsafe_code)]

//! Import external journals into the store.
//!
//! Currently supports [Day One](https://dayoneapp.com/) JSON exports via
//! [`parse_dayone`]. Each importer maps an external format onto the store's
//! entry model, records provenance (`[import]`) so re-runs skip already-imported
//! entries, and preserves original timestamps.

mod dayone;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use notema_domain::{Celestial, ImportSource, Location, Metadata, Weather};
use thiserror::Error;

/// Map Day One's parsed location onto the store's [`Location`], keeping only the
/// place hierarchy and coordinates (the geofence `region` and `timeZoneName` are
/// dropped — the latter is already captured as the entry's `timezone`).
fn map_location(location: &dayone::model::Location) -> Location {
    // Day One exposes only a coarse placemark, so map each part to the OSM key
    // that best fits: `placeName` → `name`, `localityName` → `city` (Day One
    // doesn't distinguish city/town/village), `administrativeArea` → `state`.
    Location {
        name: location.place_name.clone(),
        city: location.locality_name.clone(),
        state: location.administrative_area.clone(),
        country: location.country.clone(),
        latitude: location.latitude,
        longitude: location.longitude,
        ..Location::default()
    }
}

/// Map Day One's `weather` onto the store's `[weather]` table. `condition` takes
/// the machine `weatherCode` slug (not the human description); `weatherServiceName`
/// is kept as `source` for attribution. The astronomy fields go to
/// [`map_celestial`] instead.
fn map_weather(weather: &dayone::model::Weather) -> Weather {
    Weather {
        condition: weather.weather_code.clone(),
        temperature_celsius: weather.temperature_celsius,
        feels_like_celsius: weather.wind_chill_celsius,
        humidity: weather.relative_humidity,
        // Dew point, cloud cover, precipitation, and wind gusts aren't in the Day
        // One export.
        dew_point_celsius: None,
        pressure_mb: weather.pressure_mb,
        visibility_km: weather.visibility_km,
        cloud_cover: None,
        precipitation_mm: None,
        wind_speed_kph: weather.wind_speed_kph,
        wind_gust_kph: None,
        wind_direction: weather.wind_bearing,
        source: weather.weather_service_name.clone(),
    }
}

/// Map Day One's `weather` onto the store's `[celestial]` table (sun/moon).
/// Day One carries no daylight duration, so derive it from sunrise/sunset when
/// both parse — matching how the local compute path fills the field.
fn map_celestial(weather: &dayone::model::Weather) -> Celestial {
    let day_length_seconds = match (&weather.sunrise_date, &weather.sunset_date) {
        (Some(sunrise), Some(sunset)) => day_length_seconds(sunrise, sunset),
        _ => None,
    };
    Celestial {
        moon_phase: weather.moon_phase,
        moon_phase_name: weather.moon_phase_code.clone(),
        sunrise: weather.sunrise_date.clone(),
        sunset: weather.sunset_date.clone(),
        day_length_seconds,
    }
}

/// Seconds between two RFC3339 instants (sunset − sunrise), or `None` when either
/// fails to parse or sunset precedes sunrise.
fn day_length_seconds(sunrise: &str, sunset: &str) -> Option<u64> {
    let rise = DateTime::parse_from_rfc3339(sunrise).ok()?;
    let set = DateTime::parse_from_rfc3339(sunset).ok()?;
    u64::try_from(set.signed_duration_since(rise).num_seconds()).ok()
}

use dayone::model::DayOneExport;
use dayone::moments::{MediaIndex, rewrite_moments};
use dayone::richtext;
use dayone::text::{
    merge_code_fences, normalize_whitespace, recover_html_embeds, unescape_markdown,
};

/// Re-zone Day One's UTC timestamp into the entry's IANA zone so the stored
/// RFC3339 carries the offset it was written at (e.g. `+02:00` for a summer
/// `Europe/Berlin` entry). Falls back to UTC when the zone is missing/unknown.
fn zoned_timestamp(rfc3339_utc: &str, tz: Option<&str>) -> Option<DateTime<FixedOffset>> {
    let instant = DateTime::parse_from_rfc3339(rfc3339_utc).ok()?;
    match tz.and_then(|name| name.parse::<chrono_tz::Tz>().ok()) {
        Some(zone) => Some(instant.with_timezone(&zone).fixed_offset()),
        None => Some(instant.fixed_offset()),
    }
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse Day One export {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Default, PartialEq)]
pub struct ImportBatch {
    pub entries: Vec<ImportedEntry>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportWarning {
    pub entry_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedEntry {
    pub provenance: ImportSource,
    pub body: String,
    pub metadata: Metadata,
    pub created_at: DateTime<FixedOffset>,
    pub edited_at: DateTime<FixedOffset>,
    pub timezone: Option<String>,
    pub location: Option<Location>,
    pub weather: Option<Weather>,
    pub celestial: Option<Celestial>,
    pub writing_seconds: Option<u64>,
    /// Unique local attachment files linked into the normalized body. Storage
    /// decides how many are ultimately copied.
    pub attachments_linked: usize,
}

/// Parse and normalize a Day One export without reading or mutating a Notema
/// store. Local photo references are resolved relative to the export file.
pub fn parse_dayone(json_path: &Path) -> Result<ImportBatch, ImportError> {
    let file = File::open(json_path).map_err(|source| ImportError::Read {
        path: json_path.to_path_buf(),
        source,
    })?;
    let export: DayOneExport =
        serde_json::from_reader(BufReader::new(file)).map_err(|source| ImportError::Parse {
            path: json_path.to_path_buf(),
            source,
        })?;
    let media_root = json_path.parent().unwrap_or_else(|| Path::new("."));
    let mut batch = ImportBatch::default();

    for entry in &export.entries {
        let provenance = ImportSource {
            source: "dayone".to_string(),
            id: entry.uuid.clone(),
        };

        let tz = entry.time_zone.as_deref();
        let Some(created_at) = entry
            .creation_date
            .as_deref()
            .and_then(|value| zoned_timestamp(value, tz))
        else {
            batch.warnings.push(ImportWarning {
                entry_id: entry.uuid.clone(),
                message: "missing or invalid creationDate".to_string(),
            });
            continue;
        };
        let edited_at = entry
            .modified_date
            .as_deref()
            .and_then(|value| zoned_timestamp(value, tz))
            .unwrap_or(created_at);

        let media = MediaIndex::build(entry, media_root);
        // Prefer Day One's structured `richText` (clean, faithful) when present;
        // otherwise clean up its lossy `text`. `richtext::render` yields `None`
        // when `richText` is absent *or* parses to empty, so either way we fall
        // through to the `text` path. Both leave images as `dayone-moment://`
        // references for `rewrite_moments` below.
        let body = entry
            .rich_text
            .as_deref()
            .and_then(richtext::render)
            .unwrap_or_else(|| {
                let text = entry.text.as_deref().unwrap_or_default();
                let cleaned = normalize_whitespace(&unescape_markdown(text));
                recover_html_embeds(&merge_code_fences(&cleaned))
            });
        let rewrite = rewrite_moments(&body, &media);

        // Day One's Core Motion activity (dropping stepCount). "Stationary" means
        // the device wasn't moving — not a real activity — so it's skipped.
        let activities: Vec<String> = entry
            .user_activity
            .as_ref()
            .and_then(|ua| ua.activity_name.as_deref())
            .map(|name| name.trim().to_lowercase())
            .filter(|name| !name.is_empty() && name != "stationary")
            .into_iter()
            .collect();

        let metadata = Metadata {
            tags: entry.tags.clone(),
            activities,
            starred: entry.starred,
            ..Metadata::default()
        };
        let location = entry
            .location
            .as_ref()
            .map(map_location)
            .filter(|l| !l.is_empty());
        // Both the weather and the celestial tables read from Day One's single
        // `weather` object.
        let weather = entry
            .weather
            .as_ref()
            .map(map_weather)
            .filter(|w| !w.is_empty());
        let celestial = entry
            .weather
            .as_ref()
            .map(map_celestial)
            .filter(|c| !c.is_empty());
        let writing_seconds = entry
            .editing_time
            .and_then(|seconds| std::time::Duration::try_from_secs_f64(seconds).ok())
            .map(|duration| duration.as_secs());

        for id in &rewrite.unresolved {
            batch.warnings.push(ImportWarning {
                entry_id: entry.uuid.clone(),
                message: format!("unresolved moment {id}"),
            });
        }
        let attachments_linked = rewrite.linked_attachments();
        batch.entries.push(ImportedEntry {
            provenance,
            body: rewrite.body,
            metadata,
            created_at,
            edited_at,
            timezone: entry.time_zone.clone(),
            location,
            weather,
            celestial,
            writing_seconds,
            attachments_linked,
        });
    }

    Ok(batch)
}
