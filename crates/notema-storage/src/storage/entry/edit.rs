use super::codec::EntryCodec;
use super::create::EntryAssetOptions;
use super::paths::entry_assets_dir;
use crate::{AppResult, EntryRevision, StorageError};
use anyhow::{Context, bail};
use notema_domain::{Metadata, MetadataField};
use notema_encryption::{self as crypto, KeyPaths};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn delete_journal(
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

/// The result of an edit-via-editor session, so callers can tell a real edit
/// from a no-op open (e.g. to record editing time only when the body changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutcome {
    /// The body was changed and written.
    Changed,
    /// Kept as-is — the editor failed/cancelled, or was closed without changes.
    Unchanged,
    /// The entry was deleted for being emptied.
    Deleted,
}

#[derive(Debug, PartialEq, Eq)]
pub struct EntryEditOutcome {
    pub outcome: EditOutcome,
    pub assets: super::assets::AssetReport,
}

pub struct EntryEdit<'a> {
    pub body: &'a str,
    pub metadata: &'a Metadata,
    pub original_metadata: &'a Metadata,
    pub writing_seconds: Option<u64>,
    pub remove_if_empty: bool,
    pub extra_fields: &'a [MetadataField],
}

impl EditOutcome {
    /// Whether the entry still exists after the session.
    pub fn kept(self) -> bool {
        !matches!(self, EditOutcome::Deleted)
    }
}

/// Save an edited entry from the plaintext body and metadata already in memory.
/// The entry is opened once to preserve front matter, then assets, metadata,
/// writing time, and extra save-time fields are applied before a single final
/// write.
pub(crate) fn save_entry_edit(
    codec: &EntryCodec<'_>,
    path: &Path,
    edit: EntryEdit<'_>,
    assets: EntryAssetOptions,
) -> AppResult<EntryEditOutcome> {
    save_entry_edit_inner(codec, path, None, edit, assets)
}

pub(crate) fn save_entry_edit_if_revision(
    codec: &EntryCodec<'_>,
    path: &Path,
    revision: EntryRevision,
    edit: EntryEdit<'_>,
    assets: EntryAssetOptions,
) -> AppResult<EntryEditOutcome> {
    save_entry_edit_inner(codec, path, Some(revision), edit, assets)
}

fn save_entry_edit_inner(
    codec: &EntryCodec<'_>,
    path: &Path,
    revision: Option<EntryRevision>,
    edit: EntryEdit<'_>,
    assets: EntryAssetOptions,
) -> AppResult<EntryEditOutcome> {
    ensure_revision(path, revision)?;
    let entry = codec.open(path)?;
    ensure_revision(path, revision)?;

    if edit.remove_if_empty && edit.body.trim().is_empty() {
        ensure_revision(path, revision)?;
        fs::remove_file(path)?;
        remove_entry_assets(path);
        return Ok(EntryEditOutcome {
            outcome: EditOutcome::Deleted,
            assets: super::assets::AssetReport::default(),
        });
    }

    let body = edit.body.trim_start_matches('\n');
    let body_changed = body != entry.body;
    let metadata_fields = changed_metadata_fields(edit.original_metadata, edit.metadata);
    let has_metadata_changes = !metadata_fields.is_empty() || !edit.extra_fields.is_empty();

    if !body_changed && !has_metadata_changes {
        return Ok(EntryEditOutcome {
            outcome: EditOutcome::Unchanged,
            assets: super::assets::AssetReport::default(),
        });
    }

    let encryption = super::paths::is_encrypted_entry_file(path).then(|| codec.encryption_paths());
    let (rewritten_body, report) = super::assets::ingest_and_cleanup_opts(
        path,
        body,
        encryption,
        assets.download_remote,
        assets.replace_offline,
    )?;
    let final_body = rewritten_body.as_deref().unwrap_or(body);
    let content = render_edited_content(
        entry.front_matter.as_deref(),
        final_body,
        &metadata_fields,
        edit.extra_fields,
        edit.writing_seconds,
    )?;
    ensure_revision(path, revision)?;
    codec.write_existing(path, &content)?;

    Ok(EntryEditOutcome {
        outcome: EditOutcome::Changed,
        assets: report,
    })
}

fn ensure_revision(path: &Path, expected: Option<EntryRevision>) -> AppResult<()> {
    if expected.is_some_and(|expected| EntryRevision::read(path).ok() != Some(expected)) {
        return Err(StorageError::EntryRevisionConflict {
            path: path.to_path_buf(),
        }
        .into());
    }
    Ok(())
}

fn changed_metadata_fields(original: &Metadata, current: &Metadata) -> Vec<MetadataField> {
    let mut fields = Vec::new();
    if current.tags != original.tags {
        fields.push(MetadataField::Tags(current.tags.clone()));
    }
    if current.people != original.people {
        fields.push(MetadataField::People(current.people.clone()));
    }
    if current.activities != original.activities {
        fields.push(MetadataField::Activities(current.activities.clone()));
    }
    if current.feelings != original.feelings {
        fields.push(MetadataField::Feelings(current.feelings.clone()));
    }
    if current.mood != original.mood {
        fields.push(MetadataField::Mood(current.mood));
    }
    if current.location != original.location {
        fields.push(MetadataField::Location(
            current.location.clone().map(Box::new),
        ));
    }
    fields
}

fn render_edited_content(
    front_matter: Option<&str>,
    body: &str,
    metadata_fields: &[MetadataField],
    extra_fields: &[MetadataField],
    writing_seconds: Option<u64>,
) -> AppResult<String> {
    let Some(front_matter) = front_matter else {
        return Ok(body.to_string());
    };
    let mut parsed = match crate::markdown::parse_front_matter(front_matter) {
        Ok(parsed) => parsed,
        Err(error) => {
            if !metadata_fields.is_empty() || !extra_fields.is_empty() {
                bail!("cannot edit entry metadata until its front matter is repaired: {error}");
            }
            // A body-only edit can preserve malformed front matter byte-for-byte.
            // Capture fields such as writing time cannot be updated safely here.
            return Ok(format!(
                "+++\n{front_matter}\n+++\n\n{}",
                body.trim_start_matches('\n')
            ));
        }
    };

    for field in metadata_fields.iter().chain(extra_fields) {
        crate::markdown::apply_metadata_field(&mut parsed, field);
    }
    if let Some(secs) = writing_seconds
        && secs > 0
    {
        parsed.datetime.writing_seconds = Some(
            parsed
                .datetime
                .writing_seconds
                .unwrap_or(0)
                .saturating_add(secs),
        );
    }
    parsed.datetime.edited_at = Some(chrono::Local::now().to_rfc3339());
    Ok(crate::markdown::render_entry(&parsed, body))
}

pub(crate) fn delete_empty_entry(path: &Path) -> AppResult<()> {
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

pub(crate) fn write_plain_atomic(path: &Path, content: &str) -> AppResult<()> {
    Ok(crypto::atomic_write(path, content.as_bytes())?)
}

pub(crate) fn write_encrypted_entry_content(
    paths: &KeyPaths,
    path: &Path,
    content: &str,
) -> AppResult<()> {
    let plaintext = crypto::PlaintextBytes::copy_from_slice(content.as_bytes());
    Ok(crypto::encrypt_to_file(paths, &plaintext, path)?)
}

pub(crate) fn move_entry_to_trash(root: &Path, entry_path: &Path) -> AppResult<PathBuf> {
    let relative = entry_path.strip_prefix(root)?;
    let mut components = relative.components();
    let journal = components
        .next()
        .context("entry path is missing journal component")?
        .as_os_str();
    let mut entry_relative_path = PathBuf::new();
    for component in components {
        entry_relative_path.push(component.as_os_str());
    }
    if entry_relative_path.as_os_str().is_empty() {
        bail!("entry path is missing file path after journal component");
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
