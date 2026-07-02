use super::create::{WriteTarget, create_entry_file};
use super::paths::entry_path_with_id;
use super::*;
use crate::crypto;
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
        dir.path(),
        "work",
        now,
        "new content",
        WriteTarget::Plain,
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

    let created = create_entry_with_body(dir.path(), "work", "Some text").unwrap();
    let text = fs::read_to_string(created).unwrap();

    assert!(text.starts_with("---\ncreated_at: \""));
    assert!(text.contains("\nupdated_at: \""));
    assert!(text.contains("\ntags: []\n...\n\nSome text\n"));
}

#[test]
fn create_entry_with_body_preserves_multiline_body_and_trailing_newline() {
    let dir = tempdir().unwrap();

    let created = create_entry_with_body(dir.path(), "work", "Line one\n\nLine three\n").unwrap();
    let text = fs::read_to_string(created).unwrap();

    assert!(text.ends_with("\n\nLine three\n"));
    assert!(!text.ends_with("\n\nLine three\n\n"));
}

#[test]
fn entry_template_has_expected_front_matter() {
    let now = local_time(2026, 7, 1, 23, 30);

    let template = entry_template(now, now);

    assert_eq!(
        template,
        format!(
            "---\ncreated_at: \"{}\"\nupdated_at: \"{}\"\ntags: []\n...\n\n",
            now.to_rfc3339(),
            now.to_rfc3339()
        )
    );
    assert!(!template.contains("journal:"));
    assert!(!template.contains("kind:"));
}

#[test]
fn entry_id_is_filename_stem_not_front_matter() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("id-from-file.md");
    fs::write(
        &path,
        "---\nid: \"wrong\"\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\n# Title\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path).unwrap();

    assert_eq!(entry.id, "id-from-file");
}

#[test]
fn entry_journal_is_read_context_not_front_matter() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "---\njournal: \"wrong\"\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\n# Title\n",
    )
    .unwrap();

    let entry = read_entry("folder-name", &path).unwrap();

    assert_eq!(entry.journal, "folder-name");
}

#[test]
fn entry_title_uses_first_markdown_line_and_preview_uses_next_line() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\n# Hi how is it going?\nThis is a test entry\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path).unwrap();

    assert_eq!(entry.title, "Hi how is it going?");
    assert_eq!(entry.preview, "This is a test entry");
}

#[test]
fn entry_tags_read_yaml_block_list() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "---\ntags:\n  - work\n  - deep focus\n...\n\n# Tagged\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path).unwrap();

    assert_eq!(entry.tags, vec!["work", "deep focus"]);
}

#[test]
fn plain_entry_title_and_preview_use_first_two_markdown_lines() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\nPlain title\nPlain preview\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path).unwrap();

    assert_eq!(entry.title, "Plain title");
    assert_eq!(entry.preview, "Plain preview");
}

#[test]
fn empty_entry_uses_timestamp_title_and_empty_preview() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("entry.md");
    fs::write(
        &path,
        "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\n",
    )
    .unwrap();

    let entry = read_entry("journal", &path).unwrap();

    assert_eq!(entry.title, "2026-07-01T10:00:00+02:00");
    assert_eq!(entry.preview, "");
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
        "---\ntags: []\n...\n\n# Active\n",
    )
    .unwrap();
    fs::write(
        trash_dir.join("trashed.md"),
        "---\ntags: []\n...\n\n# Trashed\n",
    )
    .unwrap();

    let entries = scan_entries(dir.path()).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "Active");
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

    let entries = scan_entries(dir.path()).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].encryption_state,
        EntryEncryptionState::EncryptedLocked
    );
    assert_eq!(entries[0].title, "[locked] Encrypted entry");
    assert_eq!(entries[0].preview, "Encryption identity not available");
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
    let paths = crypto::EncryptionPaths::for_config(&config, &root).unwrap();
    crypto::generate_identity_store(&paths, "secret").unwrap();
    let encrypted =
        create_encrypted_entry_with_body(&root, "work", "# Secret\nBody", &paths).unwrap();
    let identity = crypto::unlock_identity(&paths, "secret").unwrap();

    let entries = scan_entries_with_identity(&root, Some(&identity)).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, encrypted);
    assert_eq!(
        entries[0].encryption_state,
        EntryEncryptionState::EncryptedUnlocked
    );
    assert_eq!(entries[0].title, "Secret");
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
            .join("work")
            .join(".trash")
            .join("2026")
            .join("07")
            .join("01")
            .join("id.md")
    );
    assert!(trash.exists());
    assert!(!path.exists());
}
