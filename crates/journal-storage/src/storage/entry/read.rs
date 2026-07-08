use super::paths::{entry_id, is_assets_dir, is_encrypted_entry_file, is_entry_file};
use super::{Entry, EntryEncryptionState, EntryPath, Metadata, Timestamp};
use crate::storage::{journals::is_hidden_name, list_journals};
use crate::{
    AppResult,
    markdown::{FrontMatter, display_preview, front_matter_fields, split_front_matter},
};
use anyhow::Context;
use journal_core::entry::build_search_haystack;
use journal_core::feelings::normalize_feelings;
use journal_encryption as crypto;
use rayon::prelude::*;
use std::{fs, path::Path};

pub fn scan_entries(
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
            if !is_hidden_name(&name) && !is_assets_dir(&path) {
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
        .map(|entry| read_entry(&entry.journal, &entry.path, identity))
        .collect::<AppResult<Vec<Entry>>>()?;
    entries.sort_by(|a, b| b.path.cmp(&a.path));
    Ok(entries)
}

pub fn read_entry(
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
    let content = match read_entry_content(path, identity) {
        Ok(content) => content,
        // An encrypted entry the loaded identity can't decrypt (e.g. a device
        // not yet approved as a recipient, or a partially re-encrypted store)
        // degrades to a locked placeholder rather than failing the whole scan.
        Err(_) if matches!(encryption_state, EntryEncryptionState::EncryptedUnlocked) => {
            return locked_entry(journal, path);
        }
        Err(error) => return Err(error),
    };
    let (front_matter, body) = split_front_matter(&content);
    // One TOML parse per entry instead of one per field.
    let FrontMatter {
        mut metadata,
        datetime,
        import,
        location,
        // Capture-only: preserved on disk, not surfaced on the in-memory entry.
        weather: _,
        celestial: _,
        air_quality: _,
    } = front_matter.map(front_matter_fields).unwrap_or_default();
    metadata.feelings = normalize_feelings(metadata.feelings.iter().map(String::as_str));
    let created_at = datetime.created_at.map(Timestamp::parse);
    // `datetime.timezone`/`writing_seconds` are capture-only: preserved on disk,
    // not surfaced here.
    let edited_at = datetime.edited_at;
    let id = entry_id(path).context("entry file has no UTF-8 stem")?;
    let preview = display_preview(body);
    let body = body.trim_start_matches('\n').to_string();
    let word_count = body.split_whitespace().count();
    let search_haystack = build_search_haystack(&body, &metadata);

    Ok(Entry {
        id,
        journal: journal.to_string(),
        path: path.to_path_buf(),
        encryption_state,
        created_at,
        edited_at,
        preview,
        metadata,
        location,
        import,
        content: body,
        word_count,
        search_haystack,
    })
}

fn locked_entry(journal: &str, path: &Path) -> AppResult<Entry> {
    let id = entry_id(path).context("entry file has no UTF-8 stem")?;
    Ok(Entry {
        id,
        journal: journal.to_string(),
        path: path.to_path_buf(),
        encryption_state: EntryEncryptionState::EncryptedLocked,
        created_at: None,
        edited_at: None,
        preview: "[locked] Encrypted entry".to_string(),
        metadata: Metadata::default(),
        location: None,
        import: None,
        content: "Encryption identity not available".to_string(),
        word_count: 0,
        search_haystack: String::new(),
    })
}

pub fn read_entry_content(
    path: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
) -> AppResult<String> {
    if is_encrypted_entry_file(path) {
        let identity = identity.ok_or(crate::EncryptionError::Locked { context: "entry" })?;
        Ok(String::from_utf8(crypto::decrypt_file_bytes(
            identity, path,
        )?)?)
    } else {
        Ok(fs::read_to_string(path)?)
    }
}
