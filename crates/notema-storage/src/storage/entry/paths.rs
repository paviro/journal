use chrono::{DateTime, FixedOffset};
#[cfg(test)]
use nanoid::nanoid;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(crate) const ENTRY_ID_LEN: usize = 12;

#[cfg(test)]
pub(crate) fn entry_path(root: &Path, journal: &str, now: DateTime<FixedOffset>) -> PathBuf {
    entry_path_with_id(root, journal, now, &nanoid!(ENTRY_ID_LEN))
}

pub(crate) fn entry_path_with_id(
    root: &Path,
    journal: &str,
    now: DateTime<FixedOffset>,
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
    now: DateTime<FixedOffset>,
    id: &str,
) -> PathBuf {
    entry_path_with_id(root, journal, now, id).with_extension("md.age")
}

pub(crate) fn is_encrypted_entry_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".md.age"))
}

pub(crate) fn is_plain_entry_file(path: &Path) -> bool {
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

/// The sibling asset directory for an entry: `<parent>/<stem>.assets`, where
/// `<stem>` is the entry id (the file name without the `.md`/`.md.age` suffix).
/// Images referenced by the entry are stored (and encrypted) inside it.
pub(crate) fn entry_assets_dir(entry_path: &Path) -> Option<PathBuf> {
    let stem = entry_id(entry_path)?;
    let parent = entry_path.parent()?;
    Some(parent.join(format!("{stem}.assets")))
}

/// The directory name used for an entry's asset folder inside the body links,
/// e.g. `2026-07-05T14-30-00-abc123.assets`.
pub(crate) fn entry_assets_dir_name(entry_path: &Path) -> Option<String> {
    Some(format!("{}.assets", entry_id(entry_path)?))
}

/// True when `path` names a per-entry asset directory (`<stem>.assets`).
pub(crate) fn is_assets_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".assets"))
}
