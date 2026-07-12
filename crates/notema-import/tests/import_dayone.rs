//! End-to-end coverage for the Day One import orchestration: entry creation and
//! provenance, duplicate skipping on re-run, and per-entry failure handling.
//! (The body-transform details are unit-tested inside the crate.)

use std::{collections::HashSet, fs, path::Path};

use notema_domain::ImportSource;
use notema_storage::{AssetFailure, EntryAssetOptions, EntryDraft, JournalStore};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct ImportReport {
    imported: usize,
    skipped_duplicate: usize,
    images_stored: usize,
    images_failed: usize,
    attachments_skipped: usize,
    failures: Vec<String>,
}

fn import_dayone(
    store: &JournalStore,
    journal: &str,
    path: &Path,
    download_remote: bool,
) -> anyhow::Result<ImportReport> {
    if store.encrypts_new_files() && !store.is_unlocked() {
        anyhow::bail!("the journal store is encrypted; unlock it before importing");
    }
    if !store
        .list_journals()?
        .iter()
        .any(|item| item.name == journal)
    {
        store.create_journal(journal)?;
    }
    let mut seen: HashSet<_> = store.scan_import_sources()?.into_iter().collect();
    let batch = notema_import::parse_dayone(path)?;
    let mut report = ImportReport::default();
    for warning in batch.warnings {
        report
            .failures
            .push(format!("{}: {}", warning.entry_id, warning.message));
    }
    for entry in batch.entries {
        if !seen.insert(entry.provenance.clone()) {
            report.skipped_duplicate += 1;
            continue;
        }
        let created = store.create_entry(
            EntryDraft {
                journal,
                body: &entry.body,
                metadata: &entry.metadata,
                created_at: Some(entry.created_at),
                edited_at: Some(entry.edited_at),
                timezone: entry.timezone.as_deref(),
                location: entry.location.as_ref(),
                weather: entry.weather.as_ref(),
                celestial: entry.celestial.as_ref(),
                air_quality: None,
                writing_seconds: entry.writing_seconds,
                import: Some(&entry.provenance),
            },
            EntryAssetOptions {
                download_remote,
                replace_offline: download_remote,
            },
        )?;
        report.imported += 1;
        report.attachments_skipped += entry.attachments_skipped;
        report.images_stored += created.assets.stored;
        for failure in created.assets.failed {
            if let AssetFailure::Ingest { source, error } = failure {
                report.images_failed += 1;
                report
                    .failures
                    .push(format!("{}: {source}: {error}", entry.provenance.id));
            }
        }
    }
    Ok(report)
}

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
    assert!(entry.body.contains("Hello from Day One"));
    assert_eq!(entry.tags, vec!["travel", "notes"]);
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
        entry.activities.clone()
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
    assert!(starred.starred);
    assert!(!plain.starred);
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

/// An encrypted store rooted in a fresh temp dir, initialized and unlocked.
fn encrypted_store() -> (TempDir, JournalStore) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = JournalStore::new(dir.path().join("journals"), dir.path());
    store.ensure().unwrap();
    store.initialize_encryption("dev", None).unwrap();
    store.unlock(None).unwrap();
    (dir, store)
}

#[test]
fn imports_photo_into_encrypted_entry_assets() {
    let (dir, store) = encrypted_store();
    let photos = dir.path().join("photos");
    fs::create_dir_all(&photos).unwrap();
    fs::write(photos.join("aabbcc.jpeg"), b"fake jpeg bytes").unwrap();
    let json = r#"{
        "entries": [
            {
                "uuid": "PIC1",
                "text": "Look\n\n![](dayone-moment://PHOTOID)",
                "creationDate": "2026-07-01T10:00:00Z",
                "photos": [{ "identifier": "PHOTOID", "md5": "aabbcc", "type": "jpeg" }]
            }
        ]
    }"#;

    let report = import_dayone(&store, "diary", &write_export(&dir, json), false).unwrap();

    assert_eq!(report.imported, 1);
    assert_eq!(report.images_stored, 1);
    assert_eq!(report.images_failed, 0);
    assert!(report.failures.is_empty());

    let entries = store.scan_entries().unwrap();
    let entry = &entries[0];
    assert!(entry.path.to_string_lossy().ends_with(".md.age"));
    // The body references the entry's own asset folder, not the export archive.
    assert!(entry.body.contains(".assets/"), "{}", entry.body);
    assert!(!entry.body.contains("photos/aabbcc.jpeg"), "{}", entry.body);
    // The asset landed next to the entry, encrypted.
    let assets_dir = fs::read_dir(entry.path.parent().unwrap())
        .unwrap()
        .map(|item| item.unwrap().path())
        .find(|path| path.is_dir() && path.to_string_lossy().ends_with(".assets"))
        .expect("entry assets dir");
    let assets: Vec<_> = fs::read_dir(&assets_dir)
        .unwrap()
        .map(|item| item.unwrap().path())
        .collect();
    assert_eq!(assets.len(), 1);
    assert!(assets[0].to_string_lossy().ends_with(".age"));
}

#[test]
fn locked_encrypted_store_refuses_to_import() {
    let (dir, _unlocked) = encrypted_store();
    // A fresh store handle that was never unlocked.
    let locked = JournalStore::new(dir.path().join("journals"), dir.path());
    let json = r#"{
        "entries": [
            { "uuid": "L1", "text": "Nope", "creationDate": "2026-07-01T10:00:00Z" }
        ]
    }"#;

    let error = import_dayone(&locked, "diary", &write_export(&dir, json), false).unwrap_err();

    assert!(error.to_string().contains("unlock"), "{error}");
    // Refused before creating anything.
    assert!(_unlocked.scan_entries().unwrap().is_empty());
}
