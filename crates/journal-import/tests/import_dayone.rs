//! End-to-end coverage for the Day One import orchestration: entry creation and
//! provenance, duplicate skipping on re-run, and per-entry failure handling.
//! (The body-transform details are unit-tested inside the crate.)

use std::fs;

use journal_import::import_dayone;
use journal_storage::{ImportSource, JournalStore};
use tempfile::TempDir;

/// The provenance a Day One import records for the entry with this uuid.
fn dayone(id: &str) -> ImportSource {
    ImportSource {
        source: "dayone".to_string(),
        id: id.to_string(),
    }
}

/// A plaintext store rooted in a fresh temp dir, plus the dir (kept alive).
fn plaintext_store() -> (TempDir, JournalStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = JournalStore::new(dir.path().join("journals"), dir.path());
    store.ensure().unwrap();
    (dir, store)
}

/// Write a Day One export JSON next to where media would live and return its path.
fn write_export(dir: &TempDir, json: &str) -> std::path::PathBuf {
    let path = dir.path().join("export.json");
    fs::write(&path, json).unwrap();
    path
}

#[test]
fn imports_entry_with_body_tags_and_provenance() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "ABC123",
                "text": "Hello from Day One",
                "creationDate": "2026-07-01T10:00:00Z",
                "tags": ["travel", "notes"]
            }
        ]
    }"#;

    let report = import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();

    assert_eq!(report.imported, 1);
    assert_eq!(report.skipped_duplicate, 0);
    assert!(report.failures.is_empty());

    let entries = store.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.import.as_ref(), Some(&dayone("ABC123")));
    assert!(entry.content.contains("Hello from Day One"));
    assert_eq!(entry.metadata.tags, vec!["travel", "notes"]);
    // The on-disk date folder comes from the creationDate, not today.
    assert!(entry.path.to_string_lossy().contains("2026"));
}

#[test]
fn imports_activity_lowercased_dropping_step_count_and_stationary() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "WALK",
                "text": "A walk",
                "creationDate": "2026-07-01T10:00:00Z",
                "userActivity": { "activityName": "Walking", "stepCount": 1000 }
            },
            {
                "uuid": "SIT",
                "text": "Sitting",
                "creationDate": "2026-07-01T11:00:00Z",
                "userActivity": { "activityName": "Stationary", "stepCount": 50 }
            },
            {
                "uuid": "PLAIN",
                "text": "No activity",
                "creationDate": "2026-07-01T12:00:00Z"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();
    let entries = store.scan_entries().unwrap();
    let activities = |uuid: &str| -> Vec<String> {
        let entry = entries
            .iter()
            .find(|e| e.import.as_ref() == Some(&dayone(uuid)))
            .unwrap();
        entry.metadata.activities.clone()
    };

    // Mapped and lowercased; the step count is never read.
    assert_eq!(activities("WALK"), vec!["walking".to_string()]);
    // "Stationary" is filtered out, and a missing activity yields none.
    assert!(activities("SIT").is_empty());
    assert!(activities("PLAIN").is_empty());
}

#[test]
fn imports_starred_flag() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "STAR1",
                "text": "A favorite",
                "creationDate": "2026-07-01T10:00:00Z",
                "starred": true
            },
            {
                "uuid": "PLAIN1",
                "text": "Not a favorite",
                "creationDate": "2026-07-01T11:00:00Z"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();

    let entries = store.scan_entries().unwrap();
    let starred = entries
        .iter()
        .find(|e| e.import.as_ref() == Some(&dayone("STAR1")))
        .unwrap();
    let plain = entries
        .iter()
        .find(|e| e.import.as_ref() == Some(&dayone("PLAIN1")))
        .unwrap();
    assert!(starred.metadata.starred);
    assert!(!plain.metadata.starred);
}

#[test]
fn imports_zone_into_offset_and_keeps_iana_name() {
    let (dir, store) = plaintext_store();
    // 06:30:05 UTC in Europe/Berlin (April = CEST, +02:00) is 08:30:05 local.
    let json = r#"{
        "entries": [
            {
                "uuid": "BERLIN1",
                "text": "Guten Morgen",
                "creationDate": "2021-04-03T06:30:05Z",
                "timeZone": "Europe/Berlin"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();

    let entries = store.scan_entries().unwrap();
    let entry = &entries[0];
    // The entry files under its own local date, not the UTC date or the importer's.
    assert!(entry.path.to_string_lossy().contains("2021/04/03"));

    let raw = std::fs::read_to_string(&entry.path).unwrap();
    // Offset folded into the timestamp; IANA name kept alongside for fidelity.
    assert!(raw.contains("created_at = \"2021-04-03T08:30:05+02:00\""));
    assert!(raw.contains("timezone = \"Europe/Berlin\""));
}

#[test]
fn imports_without_zone_fall_back_to_utc_offset() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "NOZONE1",
                "text": "No zone",
                "creationDate": "2026-07-01T12:30:00Z"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();

    let entry = &store.scan_entries().unwrap()[0];
    let raw = std::fs::read_to_string(&entry.path).unwrap();
    assert!(raw.contains("created_at = \"2026-07-01T12:30:00+00:00\""));
    assert!(!raw.contains("timezone"));
}

#[test]
fn imports_editing_time_as_writing_seconds() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "TIMED",
                "text": "Took a while",
                "creationDate": "2026-07-01T10:00:00Z",
                "editingTime": 45.7
            },
            {
                "uuid": "UNTIMED",
                "text": "No timing",
                "creationDate": "2026-07-01T11:00:00Z"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();
    let entries = store.scan_entries().unwrap();
    let raw = |uuid: &str| {
        let entry = entries
            .iter()
            .find(|e| e.import.as_ref() == Some(&dayone(uuid)))
            .unwrap();
        std::fs::read_to_string(&entry.path).unwrap()
    };

    // Fractional seconds truncate to whole seconds.
    assert!(raw("TIMED").contains("writing_seconds = 45"));
    assert!(!raw("UNTIMED").contains("writing_seconds"));
}

#[test]
fn imports_location_storing_only_present_fields() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "FULL",
                "text": "Full location",
                "creationDate": "2021-04-03T06:30:05Z",
                "location": {
                    "placeName": "1 Example Plaza",
                    "localityName": "Testville",
                    "administrativeArea": "Test Province",
                    "country": "Testland",
                    "latitude": 10.0,
                    "longitude": 20.0,
                    "region": { "radius": 75 }
                }
            },
            {
                "uuid": "PARTIAL",
                "text": "City + country only",
                "creationDate": "2021-04-03T07:00:00Z",
                "location": { "localityName": "Testville", "country": "Testland" }
            },
            {
                "uuid": "NONE",
                "text": "No location",
                "creationDate": "2021-04-03T08:00:00Z"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();
    let entries = store.scan_entries().unwrap();
    let raw = |uuid: &str| {
        let e = entries
            .iter()
            .find(|e| e.import.as_ref() == Some(&dayone(uuid)))
            .unwrap();
        std::fs::read_to_string(&e.path).unwrap()
    };

    let full = raw("FULL");
    assert!(full.contains("[location]"));
    assert!(full.contains("name = \"1 Example Plaza\""));
    assert!(full.contains("city = \"Testville\""));
    assert!(full.contains("state = \"Test Province\""));
    assert!(full.contains("country = \"Testland\""));
    assert!(full.contains("latitude = 10.0"));
    // The geofence radius is dropped.
    assert!(!full.contains("radius"));

    // Only the two present fields are written — no name/coords lines.
    let partial = raw("PARTIAL");
    assert!(partial.contains("[location]"));
    assert!(partial.contains("city = \"Testville\""));
    assert!(partial.contains("country = \"Testland\""));
    assert!(!partial.contains("name"));
    assert!(!partial.contains("latitude"));

    // No location object → no table.
    assert!(!raw("NONE").contains("[location]"));
}

#[test]
fn imports_weather_and_celestial_tables() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            {
                "uuid": "FULL",
                "text": "Full weather",
                "creationDate": "2021-04-03T06:30:05Z",
                "weather": {
                    "weatherCode": "partly-cloudy",
                    "conditionsDescription": "Partly Cloudy",
                    "temperatureCelsius": 19.9,
                    "windChillCelsius": 19.5,
                    "relativeHumidity": 0.62,
                    "pressureMB": 1013.2,
                    "visibilityKM": 12.5,
                    "windSpeedKPH": 12.0,
                    "windBearing": 210.0,
                    "moonPhase": 0.5,
                    "moonPhaseCode": "full",
                    "sunriseDate": "2021-04-03T04:45:39Z",
                    "sunsetDate": "2021-04-03T18:12:00Z",
                    "weatherServiceName": "TestWeather"
                }
            },
            {
                "uuid": "PARTIAL",
                "text": "Condition + temp only",
                "creationDate": "2021-04-03T07:00:00Z",
                "weather": { "weatherCode": "clear", "temperatureCelsius": 25.0 }
            },
            {
                "uuid": "NONE",
                "text": "No weather",
                "creationDate": "2021-04-03T08:00:00Z"
            }
        ]
    }"#;

    import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();
    let entries = store.scan_entries().unwrap();
    let raw = |uuid: &str| {
        let entry = entries
            .iter()
            .find(|e| e.import.as_ref() == Some(&dayone(uuid)))
            .unwrap();
        std::fs::read_to_string(&entry.path).unwrap()
    };

    let full = raw("FULL");
    // condition holds the slug, not the human description; the provider is kept
    // as `source` for attribution.
    assert!(full.contains("condition = \"partly-cloudy\""));
    assert!(!full.contains("Partly Cloudy"));
    assert!(full.contains("source = \"TestWeather\""));
    assert!(full.contains("temperature_celsius = 19.9"));
    assert!(full.contains("feels_like_celsius = 19.5"));
    // Wind is flat on `[weather]`, not a sub-table.
    assert!(full.contains("wind_speed_kph = 12.0"));
    assert!(full.contains("wind_direction = 210"));
    assert!(full.contains("[celestial]"));
    assert!(full.contains("moon_phase = 0.5"));
    assert!(full.contains("moon_phase_name = \"full\""));
    assert!(full.contains("sunrise = \"2021-04-03T04:45:39Z\""));

    // Partial: just the two scalars, no wind and no celestial.
    let partial = raw("PARTIAL");
    assert!(partial.contains("condition = \"clear\""));
    assert!(partial.contains("temperature_celsius = 25.0"));
    assert!(!partial.contains("wind_speed_kph"));
    assert!(!partial.contains("[celestial]"));

    // No weather object → none of the tables.
    let none = raw("NONE");
    assert!(!none.contains("[weather]"));
    assert!(!none.contains("[celestial]"));
}

#[test]
fn re_running_the_same_export_skips_already_imported_entries() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            { "uuid": "DUP1", "text": "Once", "creationDate": "2026-07-01T10:00:00Z" }
        ]
    }"#;
    let path = write_export(&dir, json);

    let first = import_dayone(&store, "diary", &path, false).unwrap();
    let second = import_dayone(&store, "diary", &path, false).unwrap();

    assert_eq!(first.imported, 1);
    assert_eq!(second.imported, 0);
    assert_eq!(second.skipped_duplicate, 1);
    assert_eq!(store.scan_entries().unwrap().len(), 1);
}

#[test]
fn entry_with_invalid_creation_date_is_recorded_as_a_failure() {
    let (dir, store) = plaintext_store();
    let json = r#"{
        "entries": [
            { "uuid": "GOOD", "text": "Kept", "creationDate": "2026-07-01T10:00:00Z" },
            { "uuid": "BAD", "text": "Dropped", "creationDate": "not a date" }
        ]
    }"#;

    let report = import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();

    assert_eq!(report.imported, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("BAD"));
    let entries = store.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].import.as_ref(), Some(&dayone("GOOD")));
}
