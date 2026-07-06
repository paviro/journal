use super::codec::EntryCodec;
use super::paths::entry_assets_dir;
use crate::{AppResult, crypto};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn delete_journal(
    root: &Path,
    journal_name: &str,
    journal_path: &Path,
    entries: &[(PathBuf, bool)],
) -> AppResult<()> {
    let has_any_with_body = entries.iter().any(|(_, has_body)| *has_body);

    if !has_any_with_body {
        fs::remove_dir_all(journal_path)?;
        return Ok(());
    }

    let has_any_without_body = entries.iter().any(|(_, has_body)| !*has_body);
    let trash_journal_path = root.join(".trash").join(journal_name);

    if !has_any_without_body && !trash_journal_path.exists() {
        if let Some(parent) = trash_journal_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(journal_path, &trash_journal_path)?;
    } else {
        for (path, has_body) in entries {
            if *has_body {
                move_entry_to_trash(root, path)?;
            } else if path.exists() {
                fs::remove_file(path)?;
            }
        }
        fs::remove_dir_all(journal_path)?;
    }

    Ok(())
}

/// Read, extract body, call `edit`, then reassemble and write back.
///
/// The callback receives the body text (without front matter) and returns the
/// new body, or `None` to cancel without making any changes.
/// Returns `true` if the entry was kept, `false` if it was deleted.
pub fn edit_entry_body(
    codec: &EntryCodec,
    path: &Path,
    remove_if_empty: bool,
    edit: impl FnOnce(&str) -> AppResult<Option<String>>,
) -> AppResult<bool> {
    let entry = codec.open(path)?;

    let Some(new_body) = edit(&entry.body)? else {
        return Ok(true);
    };

    if remove_if_empty && new_body.trim().is_empty() {
        fs::remove_file(path)?;
        remove_entry_assets(path);
        return Ok(false);
    }

    codec.write_body(
        path,
        entry.front_matter.as_deref(),
        new_body.trim_start_matches('\n'),
    )?;
    Ok(true)
}

pub fn delete_empty_entry(path: &Path) -> AppResult<()> {
    fs::remove_file(path)?;
    remove_entry_assets(path);
    Ok(())
}

/// Remove an entry's sibling `<stem>.assets` folder, if present. Best-effort:
/// failures are ignored since the entry itself is already gone.
fn remove_entry_assets(entry_path: &Path) {
    if let Some(assets) = entry_assets_dir(entry_path)
        && assets.exists()
    {
        let _ = fs::remove_dir_all(assets);
    }
}

/// Replace `path` atomically: write the new content to a sibling temp file via
/// `fill`, then rename it over `path`. The temp file is cleaned up if either
/// step fails. `fill` receives the temp path and writes the (plain or encrypted)
/// bytes to it.
fn replace_atomically(path: &Path, fill: impl FnOnce(&Path) -> AppResult<()>) -> AppResult<()> {
    let temp = crate::sibling_temp_path(path, "tmp");
    let result = fill(&temp).and_then(|()| Ok(fs::rename(&temp, path)?));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(crate) fn write_plain_atomic(path: &Path, content: &str) -> AppResult<()> {
    replace_atomically(path, |temp| Ok(fs::write(temp, content)?))
}

pub(crate) fn write_encrypted_entry_content(
    paths: &crypto::EncryptionPaths,
    path: &Path,
    content: &str,
) -> AppResult<()> {
    replace_atomically(path, |temp| {
        crypto::encrypt_to_file(paths, content.as_bytes(), temp)
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

    let trash_path = root.join(".trash").join(journal).join(entry_relative_path);
    if let Some(parent) = trash_path.parent() {
        fs::create_dir_all(parent)?;
    }
    preflight_entry_assets_trash(entry_path, &trash_path)?;
    fs::rename(entry_path, &trash_path)?;
    move_entry_assets_to_trash(entry_path, &trash_path)?;
    Ok(trash_path)
}

fn preflight_entry_assets_trash(entry_path: &Path, trash_path: &Path) -> AppResult<()> {
    let (Some(source), Some(dest)) = (entry_assets_dir(entry_path), entry_assets_dir(trash_path))
    else {
        return Ok(());
    };
    if source.exists() && dest.exists() {
        return Err(crate::StorageError::TargetExists {
            what: "asset trash destination",
            path: dest,
        }
        .into());
    }
    Ok(())
}

/// Move an entry's sibling `<stem>.assets` folder next to its trashed entry
/// file so images are trashed together with the entry.
fn move_entry_assets_to_trash(entry_path: &Path, trash_path: &Path) -> AppResult<()> {
    let (Some(source), Some(dest)) = (entry_assets_dir(entry_path), entry_assets_dir(trash_path))
    else {
        return Ok(());
    };
    if !source.exists() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&source, &dest)?;
    Ok(())
}
