use crate::{
    AppResult,
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
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub title: String,
    pub preview: String,
    pub content: String,
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

pub fn create_entry(root: &Path, journal: &str, editor: &str) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_template(now, now);
    let path = create_entry_file(root, journal, now, &content, || nanoid!(ENTRY_ID_LEN))?;
    open_editor(editor, &path)?;
    set_updated_at_now(&path)?;
    Ok(path)
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

fn write_new_file(path: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(content.as_bytes())
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

pub fn scan_entries(root: &Path) -> AppResult<Vec<Entry>> {
    let mut entries = Vec::new();
    for journal in list_journals(root)? {
        collect_entries(&journal.name, &journal.path, &mut entries)?;
    }
    entries.sort_by(|a, b| b.path.cmp(&a.path));
    Ok(entries)
}

fn collect_entries(journal: &str, dir: &Path, entries: &mut Vec<Entry>) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if item.file_type()?.is_dir() {
            if name != ".trash" {
                collect_entries(journal, &path, entries)?;
            }
            continue;
        }

        if path.extension() == Some(OsStr::new("md")) {
            entries.push(read_entry(journal, &path)?);
        }
    }

    Ok(())
}

pub fn read_entry(journal: &str, path: &Path) -> AppResult<Entry> {
    let content = fs::read_to_string(path)?;
    let (front_matter, body) = split_front_matter(&content);
    let created_at = front_matter.and_then(|yaml| front_matter_value(yaml, "created_at"));
    let updated_at = front_matter.and_then(|yaml| front_matter_value(yaml, "updated_at"));
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("entry file has no UTF-8 stem")?
        .to_string();
    let (title, preview) = display_title_and_preview(body, created_at.as_deref().unwrap_or(""));

    Ok(Entry {
        id,
        journal: journal.to_string(),
        path: path.to_path_buf(),
        created_at,
        updated_at,
        title,
        preview,
        content,
    })
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
