use super::paths::{entry_id, is_encrypted_entry_file, is_entry_file};
use super::{Entry, EntryEncryptionState, EntryPath};
use crate::storage::{journals::is_hidden_name, list_journals};
use crate::{
    AppResult, crypto,
    markdown::{
        display_title_and_preview, front_matter_activities, front_matter_feelings,
        front_matter_mood, front_matter_people, front_matter_tags, front_matter_value,
        split_front_matter,
    },
};
use journal_core::feelings::normalize_feelings;
use rayon::prelude::*;
use std::{
    fs,
    path::Path,
};

#[cfg(test)]
pub fn scan_entries(root: &Path) -> AppResult<Vec<Entry>> {
    scan_entries_with_identity(root, None)
}

pub fn scan_entries_with_identity(
    root: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
) -> AppResult<Vec<Entry>> {
    read_entries(collect_entry_paths(root)?, identity)
}

/// Walk the journal tree once and collect every entry file path without reading
/// any file contents. Skips hidden directories.
pub fn collect_entry_paths(root: &Path) -> AppResult<Vec<EntryPath>> {
    let mut paths = Vec::new();
    for journal in list_journals(root)? {
        collect_paths(&journal.name, &journal.path, &mut paths)?;
    }
    Ok(paths)
}

fn collect_paths(journal: &str, dir: &Path, paths: &mut Vec<EntryPath>) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if item.file_type()?.is_dir() {
            if !is_hidden_name(&name) {
                collect_paths(journal, &path, paths)?;
            }
            continue;
        }

        if is_entry_file(&path) {
            paths.push(EntryPath {
                journal: journal.to_string(),
                path,
            });
        }
    }

    Ok(())
}

/// Read and parse (and, when encrypted, decrypt) the given entry paths in
/// parallel, returning them sorted newest-first.
pub fn read_entries(
    paths: Vec<EntryPath>,
    identity: Option<&crypto::UnlockedIdentity>,
) -> AppResult<Vec<Entry>> {
    let mut entries = paths
        .par_iter()
        .map(|entry| read_entry_with_identity(&entry.journal, &entry.path, identity))
        .collect::<AppResult<Vec<Entry>>>()?;
    entries.sort_by(|a, b| b.path.cmp(&a.path));
    Ok(entries)
}

#[cfg(test)]
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
    let tags = front_matter.map(front_matter_tags).unwrap_or_default();
    let people = front_matter.map(front_matter_people).unwrap_or_default();
    let activities = front_matter
        .map(front_matter_activities)
        .unwrap_or_default();
    let feelings = front_matter
        .map(front_matter_feelings)
        .map(|feelings| normalize_feelings(feelings.iter().map(String::as_str)))
        .unwrap_or_default();
    let mood = front_matter.and_then(front_matter_mood);
    let id = entry_id(path).ok_or("entry file has no UTF-8 stem")?;
    let (title, preview) = display_title_and_preview(body, created_at.as_deref().unwrap_or(""));
    let body = body.trim_start_matches('\n').to_string();

    Ok(Entry {
        id,
        journal: journal.to_string(),
        path: path.to_path_buf(),
        encryption_state,
        created_at,
        updated_at,
        title,
        preview,
        tags,
        people,
        activities,
        feelings,
        mood,
        content: body,
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
        tags: Vec::new(),
        people: Vec::new(),
        activities: Vec::new(),
        feelings: Vec::new(),
        mood: None,
        content: "Encryption identity not available".to_string(),
    })
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
