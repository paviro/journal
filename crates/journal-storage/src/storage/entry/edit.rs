use super::paths::ENTRY_ID_LEN;
use crate::{AppResult, crypto, markdown};
use nanoid::nanoid;
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
    path: &Path,
    encryption: Option<(&crypto::EncryptionPaths, &crypto::UnlockedIdentity)>,
    remove_if_empty: bool,
    edit: impl FnOnce(&str) -> AppResult<Option<String>>,
) -> AppResult<bool> {
    let content = match encryption {
        Some((_, identity)) => crypto::decrypt_to_string(identity, path)?,
        None => fs::read_to_string(path)?,
    };

    let (front_matter, body) = markdown::split_front_matter(&content);
    let body = body.trim_start_matches('\n');

    let Some(new_body) = edit(body)? else {
        return Ok(true);
    };

    if remove_if_empty && new_body.trim().is_empty() {
        fs::remove_file(path)?;
        return Ok(false);
    }

    let new_content = if let Some(fm) = front_matter {
        let reassembled = format!("+++\n{fm}\n+++\n\n{}", new_body.trim_start_matches('\n'));
        markdown::set_updated_at_now_in_content(&reassembled)
    } else {
        new_body
    };

    write_entry_content(path, encryption, &new_content)?;
    Ok(true)
}

pub fn delete_empty_entry(path: &Path) -> AppResult<()> {
    Ok(fs::remove_file(path)?)
}

fn write_entry_content(
    path: &Path,
    encryption: Option<(&crypto::EncryptionPaths, &crypto::UnlockedIdentity)>,
    content: &str,
) -> AppResult<()> {
    if let Some((paths, _)) = encryption {
        write_encrypted_entry_content(paths, path, content)
    } else {
        let temp = unique_temp_path(
            path.parent().unwrap_or_else(|| Path::new(".")),
            "edit.md",
        );
        let result = (|| {
            fs::write(&temp, content)?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

pub fn write_encrypted_entry_content(
    paths: &crypto::EncryptionPaths,
    path: &Path,
    content: &str,
) -> AppResult<()> {
    let encrypted = encrypted_replacement_temp_path(path);
    let result = (|| {
        crypto::encrypt_to_file(paths, content.as_bytes(), &encrypted)?;
        fs::rename(&encrypted, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&encrypted);
    result
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
    fs::rename(entry_path, &trash_path)?;
    Ok(trash_path)
}

pub(super) fn encrypted_replacement_temp_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("entry.md.age");
    parent.join(format!(".{name}.tmp"))
}

fn unique_temp_path(dir: &Path, suffix: &str) -> PathBuf {
    dir.join(format!(
        ".journal-{}-{}.{}",
        std::process::id(),
        nanoid!(ENTRY_ID_LEN),
        suffix
    ))
}
