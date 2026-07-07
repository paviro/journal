use super::codec::EntryCodec;
use super::create::create_entry_file;
use super::paths::entry_path_with_id;
use super::*;
use crate::{JournalStorePaths, crypto};
use chrono::{DateTime, Local, LocalResult, TimeZone};
use std::fs;
use tempfile::tempdir;

fn local_time(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
    match Local.with_ymd_and_hms(y, m, d, h, min, 0) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(dt, _) => dt,
        LocalResult::None => panic!("invalid local test time"),
    }
}

#[test]
fn entry_path_uses_year_month_day_folder_and_datetime_short_id_filename() {
    let dir = tempdir().unwrap();
    let now = local_time(2026, 7, 1, 23, 30);

    let path = entry_path(dir.path(), "work", now);

    assert!(path.starts_with(dir.path().join("work").join("2026").join("07").join("01")));
    let stem = path.file_stem().unwrap().to_str().unwrap();
    let short_id = stem.strip_prefix("2026-07-01T23-30-00-").unwrap();
    assert_eq!(short_id.len(), 12);
    assert!(
        short_id
            .chars()
            .all(|ch| nanoid::alphabet::SAFE.contains(&ch))
    );
}

#[test]
fn create_entry_file_retries_without_overwriting_existing_path() {
    let dir = tempdir().unwrap();
    let now = local_time(2026, 7, 1, 23, 30);
    let existing = entry_path_with_id(dir.path(), "work", now, "existing");
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(&existing, "keep me").unwrap();
    let mut ids = ["existing", "fresh"].into_iter();

    let created = create_entry_file(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        now,
        "new content",
        || ids.next().unwrap().to_string(),
    )
    .unwrap();

    assert_eq!(
        created,
        entry_path_with_id(dir.path(), "work", now, "fresh")
    );
    assert_eq!(fs::read_to_string(existing).unwrap(), "keep me");
    assert_eq!(fs::read_to_string(created).unwrap(), "new content");
}

#[test]
fn create_entry_with_body_writes_body_after_front_matter() {
    let dir = tempdir().unwrap();

    let created = create_entry(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        "Some text",
        &Metadata::default(),
    )
    .unwrap();
    let text = fs::read_to_string(created).unwrap();
    let (front_matter, body) = crate::markdown::split_front_matter(&text);
    let fields = crate::markdown::front_matter_fields(front_matter.unwrap());

    assert!(fields.created_at.is_some());
    assert!(fields.updated_at.is_some());
    assert!(fields.metadata.tags.is_empty());
    assert_eq!(body.trim_start_matches('\n'), "Some text\n");
}

#[test]
fn create_entry_with_body_preserves_multiline_body_and_trailing_newline() {
    let dir = tempdir().unwrap();

    let created = create_entry(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        "Line one\n\nLine three\n",
        &Metadata::default(),
    )
    .unwrap();
    let text = fs::read_to_string(created).unwrap();

    assert!(text.ends_with("\n\nLine three\n"));
    assert!(!text.ends_with("\n\nLine three\n\n"));
}

#[test]
fn entry_id_and_journal_come_from_path_not_front_matter() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("id-from-file.md");
    fs::write(
        &path,
        "+++\nid = \"wrong\"\njournal = \"wrong\"\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Title\n",
    )
    .unwrap();

    let entry = read_entry("folder-name", &path, None).unwrap();

    assert_eq!(entry.id, "id-from-file");
    assert_eq!(entry.journal, "folder-name");
}

#[test]
fn entry_preview_collapses_body_with_markdown_stripped() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Hi how is it going?\nThis is a test entry\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.preview, "Hi how is it going? This is a test entry");
}

#[test]
fn entry_tags_read_toml_list() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\ntags = [\"work\", \"deep focus\"]\n+++\n\n# Tagged\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.metadata.tags, vec!["work", "deep focus"]);
}

#[test]
fn entry_feelings_read_known_values_only() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\nfeelings = [\"Calm\", \"nope\", \"focused\"]\n+++\n\n# Feeling\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.metadata.feelings, vec!["calm", "focused"]);
}

#[test]
fn create_entry_with_body_and_metadata_writes_metadata() {
    let dir = tempdir().unwrap();
    let tags = vec!["rust".to_string()];
    let people = vec!["alex".to_string()];
    let activities = vec!["programming".to_string(), "cycling".to_string()];
    let feelings = vec!["calm".to_string(), "focused".to_string()];

    let created = create_entry(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        "Some text",
        &Metadata {
            tags: tags.clone(),
            people: people.clone(),
            activities: activities.clone(),
            feelings: feelings.clone(),
            mood: None,
        },
    )
    .unwrap();
    let text = fs::read_to_string(created).unwrap();
    let (front_matter, _) = crate::markdown::split_front_matter(&text);

    let fields = front_matter.map(crate::markdown::front_matter_fields);
    assert_eq!(fields.as_ref().map(|f| f.metadata.tags.clone()), Some(tags));
    assert_eq!(
        fields.as_ref().map(|f| f.metadata.people.clone()),
        Some(people)
    );
    assert_eq!(
        fields.as_ref().map(|f| f.metadata.activities.clone()),
        Some(activities)
    );
    assert_eq!(
        fields.as_ref().map(|f| f.metadata.feelings.clone()),
        Some(feelings)
    );
    assert!(text.ends_with("\nSome text\n"));
}

#[test]
fn plain_entry_preview_is_the_whole_body() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\nPlain title\nPlain preview\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.preview, "Plain title Plain preview");
    assert_eq!(entry.display_label(), "Plain title Plain preview");
}

#[test]
fn empty_entry_preview_is_empty_and_label_falls_back_to_timestamp() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.preview, "");
    assert_eq!(entry.display_label(), "2026-07-01T10:00:00+02:00");
}

#[test]
fn scan_entries_skips_trash() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026").join("07").join("01");
    let trash_dir = dir
        .path()
        .join("work")
        .join(".trash")
        .join("2026")
        .join("07")
        .join("01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::create_dir_all(&trash_dir).unwrap();
    fs::write(
        entry_dir.join("entry.md"),
        "+++\ntags = []\n+++\n\n# Active\n",
    )
    .unwrap();
    fs::write(
        trash_dir.join("trashed.md"),
        "+++\ntags = []\n+++\n\n# Trashed\n",
    )
    .unwrap();

    let entries = scan_entries(dir.path(), None).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].preview, "Active");
}

#[test]
fn scan_entries_returns_locked_placeholder_for_encrypted_entry_without_key() {
    let dir = tempdir().unwrap();
    let path = dir
        .path()
        .join("work")
        .join("2026")
        .join("07")
        .join("01")
        .join("2026-07-01T10-23-00-secret.md.age");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not decrypted during locked scans").unwrap();

    let entries = scan_entries(dir.path(), None).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].encryption_state,
        EntryEncryptionState::EncryptedLocked
    );
    assert_eq!(entries[0].preview, "[locked] Encrypted entry");
    assert_eq!(entries[0].content, "Encryption identity not available");
    assert_eq!(
        crate::storage::entry_group_date(&entries[0]),
        Some(chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
    );
}

#[test]
fn scan_entries_marks_encrypted_entry_unlocked_with_identity() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let root = dir.path().join("journals");
    let paths = JournalStorePaths::for_config(&config, &root).unwrap();
    crypto::initialize_store_identity(&paths, "laptop", Some(&crate::SecretString::from("secret")))
        .unwrap();
    let encrypted = create_entry(
        &EntryCodec::new(paths.clone(), None),
        &root,
        "work",
        "# Secret\nBody",
        &Metadata::default(),
    )
    .unwrap();
    let identity =
        crypto::unlock_identity(&paths, Some(&crate::SecretString::from("secret"))).unwrap();

    let entries = scan_entries(&root, Some(&identity)).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, encrypted);
    assert_eq!(
        entries[0].encryption_state,
        EntryEncryptionState::EncryptedUnlocked
    );
    assert_eq!(entries[0].preview, "Secret Body");
    assert!(entries[0].content.contains("Body"));
}

#[test]
fn delete_moves_entry_to_journal_trash() {
    let dir = tempdir().unwrap();
    let path = dir
        .path()
        .join("work")
        .join("2026")
        .join("07")
        .join("01")
        .join("id.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "body").unwrap();

    let trash = move_entry_to_trash(dir.path(), &path).unwrap();

    assert_eq!(
        trash,
        dir.path()
            .join(".trash")
            .join("work")
            .join("2026")
            .join("07")
            .join("01")
            .join("id.md")
    );
    assert!(trash.exists());
    assert!(!path.exists());
}

#[test]
fn delete_relocates_entry_asset_folder_to_trash() {
    let dir = tempdir().unwrap();
    let day = dir.path().join("work").join("2026").join("07").join("01");
    fs::create_dir_all(&day).unwrap();
    let path = day.join("id.md");
    fs::write(&path, "body").unwrap();
    let assets = day.join("id.assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("x9.png"), b"img").unwrap();

    move_entry_to_trash(dir.path(), &path).unwrap();

    let trashed_assets = dir
        .path()
        .join(".trash")
        .join("work")
        .join("2026")
        .join("07")
        .join("01")
        .join("id.assets");
    assert!(trashed_assets.join("x9.png").exists());
    assert!(!assets.exists());
}

#[test]
fn delete_does_not_move_entry_when_asset_trash_destination_exists() {
    let dir = tempdir().unwrap();
    let day = dir.path().join("work").join("2026").join("07").join("01");
    fs::create_dir_all(&day).unwrap();
    let path = day.join("id.md");
    fs::write(&path, "body").unwrap();
    let assets = day.join("id.assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("x9.png"), b"img").unwrap();
    let trashed_assets = dir
        .path()
        .join(".trash")
        .join("work")
        .join("2026")
        .join("07")
        .join("01")
        .join("id.assets");
    fs::create_dir_all(&trashed_assets).unwrap();

    let error = move_entry_to_trash(dir.path(), &path).unwrap_err();

    assert!(matches!(
        error.downcast_ref::<crate::StorageError>(),
        Some(crate::StorageError::TargetExists {
            what: "asset trash destination",
            ..
        })
    ));
    assert!(path.exists());
    assert!(assets.join("x9.png").exists());
}
