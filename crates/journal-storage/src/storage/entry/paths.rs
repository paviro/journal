use chrono::{DateTime, Local, NaiveDate};
#[cfg(test)]
use nanoid::nanoid;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(crate) const ENTRY_ID_LEN: usize = 12;

#[cfg(test)]
pub fn entry_path(root: &Path, journal: &str, now: DateTime<Local>) -> PathBuf {
    entry_path_with_id(root, journal, now, &nanoid!(ENTRY_ID_LEN))
}

pub(crate) fn entry_path_with_id(
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
    id: &str,
) -> PathBuf {
    root.join(journal)
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string())
        .join(format!("{}-{id}.md", now.format("%Y-%m-%dT%H-%M-%S")))
}

pub(crate) fn encrypted_entry_path_with_id(
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
    id: &str,
) -> PathBuf {
    entry_path_with_id(root, journal, now, id).with_extension("md.age")
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

pub fn entry_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".md.age")
        .or_else(|| name.strip_suffix(".md"))
        .map(str::to_string)
}

pub(crate) fn entry_date_from_path(path: &Path) -> Option<NaiveDate> {
    let stem = path.file_stem()?.to_str()?;
    let date = stem.get(..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}
