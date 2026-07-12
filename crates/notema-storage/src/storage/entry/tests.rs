use super::codec::EntryCodec;
use super::create::create_entry_file;
use super::paths::entry_path_with_id;
use super::*;
use chrono::{DateTime, FixedOffset, Local, LocalResult, TimeZone};
use notema_encryption::{self as crypto, KeyPaths};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn local_time(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<FixedOffset> {
    let local = match Local.with_ymd_and_hms(y, m, d, h, min, 0) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(dt, _) => dt,
        LocalResult::None => panic!("invalid local test time"),
    };
    local.fixed_offset()
}

fn create_test_entry(
    codec: &EntryCodec<'_>,
    root: &Path,
    journal: &str,
    body: &str,
    metadata: &Metadata,
) -> PathBuf {
    create_entry(
        codec,
        root,
        EntryDraft::new(journal, body, metadata),
        EntryAssetOptions::default(),
    )
    .unwrap()
    .path
}

#[test]
fn entry_path_uses_year_month_day_folder_and_datetime_short_id_filename() {
    let dir = tempdir().unwrap();
    let now = local_time(2026, 7, 1, 23, 30);

    let path = entry_path(dir.path(), "work", now);

    assert!(path.starts_with(dir.path().join("work").join("2026").join("07").join("01")));
    let stem = path.file_stem().unwrap().to_str().unwrap();
    let short_id = stem.strip_prefix("2026-07-01T23-30-00-").unwrap();
    assert_eq!(short_id.len(), 4);
    assert!(short_id.chars().all(|ch| ch.is_ascii_alphanumeric()));
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
fn create_entry_writes_body_after_front_matter() {
    let dir = tempdir().unwrap();

    let created = create_test_entry(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        "Some text",
        &Metadata::default(),
    );
    let text = fs::read_to_string(created).unwrap();
    let (front_matter, body) = crate::markdown::split_front_matter(&text);
    let fields = crate::markdown::front_matter_fields(front_matter.unwrap());

    assert!(fields.datetime.created_at.is_some());
    assert!(fields.datetime.edited_at.is_some());
    assert!(fields.metadata.tags.is_empty());
    // A native entry captures this machine's IANA zone name, when resolvable.
    assert_eq!(
        fields.datetime.timezone,
        iana_time_zone::get_timezone().ok()
    );
    assert_eq!(body.trim_start_matches('\n'), "Some text\n");
}

#[test]
fn create_entry_preserves_multiline_body_and_trailing_newline() {
    let dir = tempdir().unwrap();

    let created = create_test_entry(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        "Line one\n\nLine three\n",
        &Metadata::default(),
    );
    let text = fs::read_to_string(created).unwrap();

    assert!(text.ends_with("\n\nLine three\n"));
    assert!(!text.ends_with("\n\nLine three\n\n"));
}

#[test]
fn save_entry_edit_reports_changed_unchanged_and_deleted() {
    let dir = tempdir().unwrap();
    let codec = EntryCodec::plain();
    let path = create_test_entry(
        &codec,
        dir.path(),
        "work",
        "original body\n",
        &Metadata::default(),
    );
    let metadata = Metadata::default();

    // Saving the same body is not a change and does not rewrite timestamps.
    let before = fs::read_to_string(&path).unwrap();
    let saved = save_entry_edit(
        &codec,
        &path,
        EntryEdit {
            body: "original body\n",
            metadata: &metadata,
            original_metadata: &metadata,
            writing_seconds: None,
            remove_if_empty: true,
            extra_fields: &[],
        },
        EntryAssetOptions::default(),
    )
    .unwrap();
    assert_eq!(saved.outcome, EditOutcome::Unchanged);
    assert_eq!(fs::read_to_string(&path).unwrap(), before);

    // A different body is a change.
    let saved = save_entry_edit(
        &codec,
        &path,
        EntryEdit {
            body: "new body\n",
            metadata: &metadata,
            original_metadata: &metadata,
            writing_seconds: Some(30),
            remove_if_empty: true,
            extra_fields: &[],
        },
        EntryAssetOptions::default(),
    )
    .unwrap();
    assert_eq!(saved.outcome, EditOutcome::Changed);
    let text = fs::read_to_string(&path).unwrap();
    let front_matter = crate::markdown::split_front_matter(&text).0.unwrap();
    assert_eq!(
        crate::markdown::front_matter_fields(front_matter)
            .datetime
            .writing_seconds,
        Some(30)
    );

    // Emptying it deletes the entry.
    let saved = save_entry_edit(
        &codec,
        &path,
        EntryEdit {
            body: "   ",
            metadata: &metadata,
            original_metadata: &metadata,
            writing_seconds: None,
            remove_if_empty: true,
            extra_fields: &[],
        },
        EntryAssetOptions::default(),
    )
    .unwrap();
    assert_eq!(saved.outcome, EditOutcome::Deleted);
    assert!(!path.exists());
}

#[test]
fn save_entry_edit_preserves_unparseable_front_matter() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("2026-07-06T10-00-00.md");
    let original = "+++\nschema_version = 1\ntags = [unterminated\n+++\n\nold body\n";
    fs::write(&path, original).unwrap();
    let metadata = Metadata::default();

    save_entry_edit(
        &EntryCodec::plain(),
        &path,
        EntryEdit {
            body: "new body\n",
            metadata: &metadata,
            original_metadata: &metadata,
            writing_seconds: Some(12),
            remove_if_empty: true,
            extra_fields: &[],
        },
        EntryAssetOptions::default(),
    )
    .unwrap();

    let written = fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("tags = [unterminated"),
        "metadata preserved"
    );
    assert!(written.contains("new body"), "body updated");
}

#[test]
fn entry_id_and_journal_come_from_path_not_front_matter() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("id-from-file.md");
    fs::write(
        &path,
        "+++\nschema_version = 1\nid = \"wrong\"\njournal = \"wrong\"\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Title\n",
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
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Hi how is it going?\nThis is a test entry\n",
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
        "+++\nschema_version = 1\ntags = [\"work\", \"deep focus\"]\n+++\n\n# Tagged\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.tags, vec!["work", "deep focus"]);
}

#[test]
fn entry_feelings_read_known_values_only() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\nschema_version = 1\nfeelings = [\"Calm\", \"nope\", \"focused\"]\n+++\n\n# Feeling\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path, None).unwrap();

    assert_eq!(entry.feelings, vec!["calm", "focused"]);
}

#[test]
fn create_entry_writes_metadata() {
    let dir = tempdir().unwrap();
    let tags = vec!["rust".to_string()];
    let people = vec!["alex".to_string()];
    let activities = vec!["programming".to_string(), "cycling".to_string()];
    let feelings = vec!["calm".to_string(), "focused".to_string()];

    let created = create_test_entry(
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
            starred: false,
            location: None,
        },
    );
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
fn create_entry_writes_metadata_location() {
    let dir = tempdir().unwrap();
    let created = create_test_entry(
        &EntryCodec::plain(),
        dir.path(),
        "work",
        "Some text",
        &Metadata {
            location: Some(notema_domain::Location {
                name: Some("Cafe".to_string()),
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..notema_domain::Location::default()
            }),
            ..Metadata::default()
        },
    );

    let text = fs::read_to_string(created).unwrap();
    let (front_matter, _) = crate::markdown::split_front_matter(&text);
    let fields = front_matter.map(crate::markdown::front_matter_fields);

    assert_eq!(
        fields
            .as_ref()
            .and_then(|fields| fields.location.as_ref())
            .and_then(|location| location.name.as_deref()),
        Some("Cafe")
    );
}

#[test]
fn plain_entry_preview_is_the_whole_body() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\nPlain title\nPlain preview\n",
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
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n",
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
        "+++\nschema_version = 1\ntags = []\n+++\n\n# Active\n",
    )
    .unwrap();
    fs::write(
        trash_dir.join("trashed.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# Trashed\n",
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
    assert_eq!(entries[0].body, "Encryption identity not available");
    assert_eq!(
        notema_domain::entry_group_date(&entries[0]),
        Some(chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
    );
}

#[test]
fn scan_entries_marks_encrypted_entry_unlocked_with_identity() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let root = dir.path().join("journals");
    let paths = KeyPaths::for_config(&config, &root).unwrap();
    crypto::initialize_store_identity(&paths, "laptop", Some(&crate::SecretString::from("secret")))
        .unwrap();
    let encrypted = create_test_entry(
        &EntryCodec::new(paths.clone(), None),
        &root,
        "work",
        "# Secret\nBody",
        &Metadata::default(),
    );
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
    assert!(entries[0].body.contains("Body"));
}

#[test]
fn scan_entries_marks_corrupt_encrypted_entry_unreadable_with_identity() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let root = dir.path().join("journals");
    let paths = KeyPaths::for_config(&config, &root).unwrap();
    crypto::initialize_store_identity(&paths, "laptop", Some(&crate::SecretString::from("secret")))
        .unwrap();
    let encrypted = create_test_entry(
        &EntryCodec::new(paths.clone(), None),
        &root,
        "work",
        "# Secret\nBody",
        &Metadata::default(),
    );
    fs::write(&encrypted, "not an age file").unwrap();
    let identity =
        crypto::unlock_identity(&paths, Some(&crate::SecretString::from("secret"))).unwrap();

    let entries = scan_entries(&root, Some(&identity)).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].encryption_state,
        EntryEncryptionState::EncryptedUnreadable
    );
    assert_eq!(entries[0].preview, "[unreadable] Encrypted entry");
    assert_eq!(entries[0].body, "Encrypted entry could not be decrypted");
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
