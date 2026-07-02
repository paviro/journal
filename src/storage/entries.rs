use crate::{
    AppResult, crypto,
    markdown::{
        display_title_and_preview, front_matter_value, set_front_matter_value, split_front_matter,
    },
};
use chrono::{DateTime, Local};
use nanoid::nanoid;
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use super::list_journals;

const ENTRY_ID_LEN: usize = 12;
const ENTRY_CREATE_ATTEMPTS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub journal: String,
    pub path: PathBuf,
    pub encryption_state: EntryEncryptionState,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub title: String,
    pub preview: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEncryptionState {
    Plain,
    EncryptedUnlocked,
    EncryptedLocked,
}

pub fn entry_path(root: &Path, journal: &str, now: DateTime<Local>) -> PathBuf {
    entry_path_with_id(root, journal, now, &nanoid!(ENTRY_ID_LEN))
}

fn entry_path_with_id(root: &Path, journal: &str, now: DateTime<Local>, id: &str) -> PathBuf {
    root.join(journal)
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string())
        .join(format!("{}-{id}.md", now.format("%Y-%m-%dT%H-%M-%S")))
}

fn encrypted_entry_path_with_id(
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
    id: &str,
) -> PathBuf {
    entry_path_with_id(root, journal, now, id).with_extension("md.age")
}

pub fn create_entry(root: &Path, journal: &str, editor: &str) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_template(now, now);
    let path = create_entry_file(root, journal, now, &content, || nanoid!(ENTRY_ID_LEN))?;
    open_editor(editor, &path)?;
    set_updated_at_now(&path)?;
    Ok(path)
}

pub fn create_encrypted_entry(
    root: &Path,
    journal: &str,
    editor: &str,
    paths: &crypto::EncryptionPaths,
    identity: &crypto::UnlockedIdentity,
) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_template(now, now);
    let path = create_encrypted_entry_file(root, journal, now, &content, paths, || {
        nanoid!(ENTRY_ID_LEN)
    })?;
    edit_encrypted_entry(&path, editor, paths, identity)?;
    Ok(path)
}

pub fn create_entry_with_body(root: &Path, journal: &str, body: &str) -> AppResult<PathBuf> {
    let now = Local::now();
    let mut content = entry_template(now, now);
    content.push_str(body);
    if !content.ends_with('\n') {
        content.push('\n');
    }

    create_entry_file(root, journal, now, &content, || nanoid!(ENTRY_ID_LEN))
}

pub fn create_encrypted_entry_with_body(
    root: &Path,
    journal: &str,
    body: &str,
    paths: &crypto::EncryptionPaths,
) -> AppResult<PathBuf> {
    let now = Local::now();
    let mut content = entry_template(now, now);
    content.push_str(body);
    if !content.ends_with('\n') {
        content.push('\n');
    }

    create_encrypted_entry_file(root, journal, now, &content, paths, || {
        nanoid!(ENTRY_ID_LEN)
    })
}

fn create_entry_file(
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
    content: &str,
    mut id_generator: impl FnMut() -> String,
) -> AppResult<PathBuf> {
    for _ in 0..ENTRY_CREATE_ATTEMPTS {
        let path = entry_path_with_id(root, journal, now, &id_generator());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        match write_new_file(&path, content) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(
        format!("could not create a unique entry path after {ENTRY_CREATE_ATTEMPTS} attempts")
            .into(),
    )
}

fn create_encrypted_entry_file(
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
    content: &str,
    paths: &crypto::EncryptionPaths,
    mut id_generator: impl FnMut() -> String,
) -> AppResult<PathBuf> {
    for _ in 0..ENTRY_CREATE_ATTEMPTS {
        let path = encrypted_entry_path_with_id(root, journal, now, &id_generator());
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        match write_encrypted_new_file(&path, content, paths) {
            Ok(()) => return Ok(path),
            Err(error) if is_already_exists_error(error.as_ref()) => continue,
            Err(error) => return Err(error),
        }
    }

    Err(
        format!("could not create a unique entry path after {ENTRY_CREATE_ATTEMPTS} attempts")
            .into(),
    )
}

fn write_new_file(path: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(content.as_bytes())
}

fn write_encrypted_new_file(
    path: &Path,
    content: &str,
    paths: &crypto::EncryptionPaths,
) -> AppResult<()> {
    let encrypted = unique_temp_path(&std::env::temp_dir(), "encrypted.age");
    let result = (|| {
        crypto::encrypt_to_file(paths, content.as_bytes(), &encrypted)?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| {
                let bytes = fs::read(&encrypted)?;
                file.write_all(&bytes)
            })?;
        Ok(())
    })();
    let _ = fs::remove_file(&encrypted);
    result
}

fn is_already_exists_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::AlreadyExists)
}

pub fn open_editor(editor: &str, path: &Path) -> AppResult<()> {
    let mut parts = shell_words::split(editor)?;
    if parts.is_empty() {
        return Err("editor command is empty".into());
    }

    let program = parts.remove(0);
    let status = Command::new(program).args(parts).arg(path).status()?;
    if !status.success() {
        return Err(format!("editor exited with status {status}").into());
    }
    Ok(())
}

pub fn set_updated_at_now(path: &Path) -> AppResult<()> {
    let content = fs::read_to_string(path)?;
    let updated = set_front_matter_value(&content, "updated_at", &Local::now().to_rfc3339());
    fs::write(path, updated)?;
    Ok(())
}

pub fn edit_encrypted_entry(
    path: &Path,
    editor: &str,
    paths: &crypto::EncryptionPaths,
    identity: &crypto::UnlockedIdentity,
) -> AppResult<()> {
    let temp_dir = std::env::temp_dir();
    let plaintext = unique_temp_path(&temp_dir, "edit.md");
    let encrypted = unique_temp_path(&temp_dir, "edit.age");
    let result = (|| {
        crypto::decrypt_file(identity, path, &plaintext)?;
        open_editor(editor, &plaintext)?;
        set_updated_at_now(&plaintext)?;
        crypto::encrypt_file(paths, &plaintext, &encrypted)?;
        fs::rename(&encrypted, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&plaintext);
    let _ = fs::remove_file(&encrypted);
    result
}

pub fn scan_entries(root: &Path) -> AppResult<Vec<Entry>> {
    scan_entries_with_identity(root, None)
}

pub fn scan_entries_with_identity(
    root: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
) -> AppResult<Vec<Entry>> {
    let mut entries = Vec::new();
    for journal in list_journals(root)? {
        collect_entries(&journal.name, &journal.path, identity, &mut entries)?;
    }
    entries.sort_by(|a, b| b.path.cmp(&a.path));
    Ok(entries)
}

fn collect_entries(
    journal: &str,
    dir: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
    entries: &mut Vec<Entry>,
) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if item.file_type()?.is_dir() {
            if name != ".trash" {
                collect_entries(journal, &path, identity, entries)?;
            }
            continue;
        }

        if is_entry_file(&path) {
            entries.push(read_entry_with_identity(journal, &path, identity)?);
        }
    }

    Ok(())
}

pub fn read_entry(journal: &str, path: &Path) -> AppResult<Entry> {
    read_entry_with_identity(journal, path, None)
}

pub fn read_entry_with_identity(
    journal: &str,
    path: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
) -> AppResult<Entry> {
    let encryption_state = if is_encrypted_entry_file(path) {
        if identity.is_none() {
            return locked_entry(journal, path);
        }
        EntryEncryptionState::EncryptedUnlocked
    } else {
        EntryEncryptionState::Plain
    };
    let content = read_entry_content_with_identity(path, identity)?;
    let (front_matter, body) = split_front_matter(&content);
    let created_at = front_matter.and_then(|yaml| front_matter_value(yaml, "created_at"));
    let updated_at = front_matter.and_then(|yaml| front_matter_value(yaml, "updated_at"));
    let id = entry_id(path).ok_or("entry file has no UTF-8 stem")?;
    let (title, preview) = display_title_and_preview(body, created_at.as_deref().unwrap_or(""));

    Ok(Entry {
        id,
        journal: journal.to_string(),
        path: path.to_path_buf(),
        encryption_state,
        created_at,
        updated_at,
        title,
        preview,
        content,
    })
}

fn locked_entry(journal: &str, path: &Path) -> AppResult<Entry> {
    let id = entry_id(path).ok_or("entry file has no UTF-8 stem")?;
    Ok(Entry {
        id,
        journal: journal.to_string(),
        path: path.to_path_buf(),
        encryption_state: EntryEncryptionState::EncryptedLocked,
        created_at: None,
        updated_at: None,
        title: "[locked] Encrypted entry".to_string(),
        preview: "Encryption identity not available".to_string(),
        content: "Encryption identity not available".to_string(),
    })
}

pub fn read_entry_content(path: &Path) -> AppResult<String> {
    read_entry_content_with_identity(path, None)
}

pub fn read_entry_content_with_identity(
    path: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
) -> AppResult<String> {
    if is_encrypted_entry_file(path) {
        let identity =
            identity.ok_or("encrypted entry requires unlocked journal encryption identity")?;
        crypto::decrypt_to_string(identity, path)
    } else {
        Ok(fs::read_to_string(path)?)
    }
}

pub fn is_encrypted_entry_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".md.age"))
}

pub fn is_plain_entry_file(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("md"))
}

pub fn is_entry_file(path: &Path) -> bool {
    is_plain_entry_file(path) || is_encrypted_entry_file(path)
}

pub fn has_encrypted_entries(root: &Path) -> AppResult<bool> {
    has_matching_entry(root, is_encrypted_entry_file)
}

fn has_matching_entry(root: &Path, predicate: fn(&Path) -> bool) -> AppResult<bool> {
    if !root.exists() {
        return Ok(false);
    }

    for item in fs::read_dir(root)? {
        let item = item?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if item.file_type()?.is_dir() {
            if name != ".trash" && has_matching_entry(&path, predicate)? {
                return Ok(true);
            }
            continue;
        }
        if predicate(&path) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn entry_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".md.age")
        .or_else(|| name.strip_suffix(".md"))
        .map(str::to_string)
}

pub fn move_entry_to_trash(root: &Path, entry_path: &Path) -> AppResult<PathBuf> {
    let relative = entry_path.strip_prefix(root)?;
    let mut components = relative.components();
    let journal = components
        .next()
        .ok_or("entry path is missing journal component")?
        .as_os_str();
    let mut entry_relative_path = PathBuf::new();
    for component in components {
        entry_relative_path.push(component.as_os_str());
    }
    if entry_relative_path.as_os_str().is_empty() {
        return Err("entry path is missing file path after journal component".into());
    }

    let trash_path = root.join(journal).join(".trash").join(entry_relative_path);
    if let Some(parent) = trash_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(entry_path, &trash_path)?;
    Ok(trash_path)
}

pub fn entry_template(created_at: DateTime<Local>, updated_at: DateTime<Local>) -> String {
    format!(
        "---\ncreated_at: \"{}\"\nupdated_at: \"{}\"\ntags: []\n---\n\n",
        created_at.to_rfc3339(),
        updated_at.to_rfc3339()
    )
}

fn unique_temp_path(dir: &Path, suffix: &str) -> PathBuf {
    dir.join(format!(
        ".journal-{}-{}.{}",
        std::process::id(),
        nanoid!(ENTRY_ID_LEN),
        suffix
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{LocalResult, TimeZone};
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

        let created = create_entry_file(dir.path(), "work", now, "new content", || {
            ids.next().unwrap().to_string()
        })
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
        assert!(text.contains("\ntags: []\n---\n\nSome text\n"));
    }

    #[test]
    fn create_entry_with_body_preserves_multiline_body_and_trailing_newline() {
        let dir = tempdir().unwrap();

        let created =
            create_entry_with_body(dir.path(), "work", "Line one\n\nLine three\n").unwrap();
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
                "---\ncreated_at: \"{}\"\nupdated_at: \"{}\"\ntags: []\n---\n\n",
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
            "---\nid: \"wrong\"\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# Title\n",
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
            "---\njournal: \"wrong\"\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# Title\n",
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
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# Hi how is it going?\nThis is a test entry\n",
        )
        .unwrap();

        let entry = read_entry("journal", &path).unwrap();

        assert_eq!(entry.title, "Hi how is it going?");
        assert_eq!(entry.preview, "This is a test entry");
    }

    #[test]
    fn plain_entry_title_and_preview_use_first_two_markdown_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("entry.md");
        fs::write(
            &path,
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\nPlain title\nPlain preview\n",
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
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n",
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
            "---\ntags: []\n---\n\n# Active\n",
        )
        .unwrap();
        fs::write(
            trash_dir.join("trashed.md"),
            "---\ntags: []\n---\n\n# Trashed\n",
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
}
