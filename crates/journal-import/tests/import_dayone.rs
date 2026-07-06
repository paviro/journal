//! End-to-end coverage for the Day One import orchestration: entry creation and
//! provenance, duplicate skipping on re-run, and per-entry failure handling.
//! (The body-transform details are unit-tested inside the crate.)

use std::fs;

use journal_import::import_dayone;
use journal_storage::JournalStore;
use tempfile::TempDir;

/// A plaintext store rooted in a fresh temp dir, plus the dir (kept alive).
fn plaintext_store() -> (TempDir, JournalStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = JournalStore::new(
        dir.path().join("journals"),
        dir.path().join("recipients.txt"),
        dir.path().join("identity.age"),
    );
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
    assert_eq!(entry.import_id.as_deref(), Some("dayone:ABC123"));
    assert!(entry.content.contains("Hello from Day One"));
    assert_eq!(entry.metadata.tags, vec!["travel", "notes"]);
    // The on-disk date folder comes from the creationDate, not today.
    assert!(entry.path.to_string_lossy().contains("2026"));
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
    assert_eq!(entries[0].import_id.as_deref(), Some("dayone:GOOD"));
}
