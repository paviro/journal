//! Import external journals into the store.
//!
//! Currently supports [Day One](https://dayoneapp.com/) JSON exports via
//! [`import_dayone`]. Each importer maps an external format onto the store's
//! entry model, records provenance (`[import]`) so re-runs skip already-imported
//! entries, and preserves original timestamps.

mod dayone;

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, FixedOffset};
use notema_core::{AppResult, Celestial, ImportSource, Location, Metadata, Weather};
use notema_storage::{AssetFailure, EntryAssetOptions, EntryDraft, JournalStore};

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

/// Summary of a Day One import, printed to the user.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Entries created.
    pub imported: usize,
    /// Entries skipped because their `[import]` provenance was already present.
    pub skipped_duplicate: usize,
    /// Photos copied into entry asset folders.
    pub images_stored: usize,
    /// Photos that could not be ingested (missing file, decode failure, …).
    pub images_failed: usize,
    /// Remote `http(s)` images that were not fetched. When downloading was on,
    /// these were unreachable and are replaced in the body with `[Offline
    /// Image]`; when off, they are left as links to fetch later. Not failures.
    pub remote_images_skipped: usize,
    /// Non-image attachments (audio/video/pdf) referenced but not imported.
    pub attachments_skipped: usize,
    /// Human-readable per-entry problems that did not abort the import.
    pub failures: Vec<String>,
}

/// Import every entry from a Day One JSON export at `json_path` into `journal`,
/// creating the journal if it does not exist. Media folders (e.g. `photos/`) are
/// resolved relative to the JSON file. Entries whose Day One UUID was already
/// imported are skipped.
///
/// `download_remote` gates fetching `http(s)` image links found in entry bodies
/// (Day One entries can embed remote images, distinct from local `photos`);
/// pass the store's configured preference, mirroring `notema log`.
pub fn import_dayone(
    store: &JournalStore,
    journal: &str,
    json_path: &Path,
    download_remote: bool,
) -> AppResult<ImportReport> {
    // Asset ingestion only needs the recipients roster, but duplicate detection
    // must decrypt existing entries' `[import]` provenance — a locked import
    // would silently re-import everything.
    if store.encrypts_new_files() && !store.is_unlocked() {
        anyhow::bail!("the journal store is encrypted; unlock it before importing");
    }

    let file =
        File::open(json_path).with_context(|| format!("could not read {}", json_path.display()))?;
    let export: DayOneExport =
        serde_json::from_reader(BufReader::new(file)).context("could not parse Day One export")?;
    let media_root = json_path.parent().unwrap_or_else(|| Path::new("."));

    if !store.list_journals()?.iter().any(|j| j.name == journal) {
        store.create_journal(journal)?;
    }

    let mut seen: HashSet<ImportSource> = store.scan_import_sources()?.into_iter().collect();

    let mut report = ImportReport::default();

    for entry in &export.entries {
        let import = ImportSource {
            source: "dayone".to_string(),
            id: entry.uuid.clone(),
        };
        if seen.contains(&import) {
            report.skipped_duplicate += 1;
            continue;
        }

        let tz = entry.time_zone.as_deref();
        let Some(created_at) = entry
            .creation_date
            .as_deref()
            .and_then(|value| zoned_timestamp(value, tz))
        else {
            report
                .failures
                .push(format!("{}: missing or invalid creationDate", entry.uuid));
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
        // Day One records fractional seconds; whole seconds are plenty.
        let editing_seconds = entry.editing_time.map(|secs| secs as u64);

        // Replace un-fetchable images with a placeholder only when we actually
        // tried to download — otherwise remote links are kept so they can be
        // fetched by a later `--download-images` run.
        let created = store.create_entry(
            EntryDraft {
                journal,
                body: &rewrite.body,
                metadata: &metadata,
                created_at: Some(created_at),
                edited_at: Some(edited_at),
                timezone: tz,
                location: location.as_ref(),
                weather: weather.as_ref(),
                celestial: celestial.as_ref(),
                air_quality: None,
                writing_seconds: editing_seconds,
                import: Some(&import),
            },
            EntryAssetOptions {
                download_remote,
                replace_offline: download_remote,
            },
        )?;

        report.images_stored += created.assets.stored;
        for failure in created.assets.failed {
            match failure {
                // A remote link we chose not to (or couldn't) fetch — download
                // off, or the host is gone — is left in the body as a link, not
                // a failure.
                AssetFailure::RemoteUnavailable { .. } => report.remote_images_skipped += 1,
                AssetFailure::Ingest { source, error } => {
                    report.images_failed += 1;
                    report
                        .failures
                        .push(format!("{}: {source}: {error}", entry.uuid));
                }
            }
        }
        for id in &rewrite.unresolved {
            report
                .failures
                .push(format!("{}: unresolved photo moment {id}", entry.uuid));
        }
        report.attachments_skipped += rewrite.skipped_attachments();
        report.imported += 1;
        seen.insert(import);
    }

    Ok(report)
}
