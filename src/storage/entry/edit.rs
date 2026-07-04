use super::paths::ENTRY_ID_LEN;
use crate::{
    AppResult, crypto,
    markdown::{entry_has_body, set_front_matter_value, split_front_matter},
};
use chrono::Local;
use nanoid::nanoid;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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

pub fn open_editor_body_only(editor: &str, path: &Path) -> AppResult<()> {
    let content = fs::read_to_string(path)?;
    let (Some(front_matter), body) = split_front_matter(&content) else {
        return open_editor(editor, path);
    };
    let front_matter = front_matter.to_string();
    let temp_dir = std::env::temp_dir();
    let temp_path = unique_temp_path(&temp_dir, "body.md");
    fs::write(&temp_path, body.trim_start_matches('\n'))?;
    let result = open_editor(editor, &temp_path);
    if result.is_ok() {
        let new_body = fs::read_to_string(&temp_path)?;
        let new_content = format!(
            "+++\n{}\n+++\n\n{}",
            front_matter,
            new_body.trim_start_matches('\n')
        );
        fs::write(path, new_content)?;
    }
    let _ = fs::remove_file(&temp_path);
    result
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
    remove_if_empty: bool,
) -> AppResult<()> {
    let temp_dir = std::env::temp_dir();
    let plaintext = unique_temp_path(&temp_dir, "edit.md");
    let encrypted = encrypted_replacement_temp_path(path);
    let result = (|| {
        crypto::decrypt_file(identity, path, &plaintext)?;
        open_editor_body_only(editor, &plaintext)?;
        if remove_if_empty && !entry_has_body(&fs::read_to_string(&plaintext)?) {
            let _ = fs::remove_file(path);
            return Ok(());
        }
        set_updated_at_now(&plaintext)?;
        crypto::encrypt_file(paths, &plaintext, &encrypted)?;
        fs::rename(&encrypted, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&plaintext);
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

    let trash_path = root.join(journal).join(".trash").join(entry_relative_path);
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

pub(super) fn unique_temp_path(dir: &Path, suffix: &str) -> PathBuf {
    dir.join(format!(
        ".journal-{}-{}.{}",
        std::process::id(),
        nanoid!(ENTRY_ID_LEN),
        suffix
    ))
}
