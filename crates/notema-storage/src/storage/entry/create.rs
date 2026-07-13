use super::Metadata;
use super::assets::{AssetReport, ingest_and_cleanup_opts};
use super::codec::EntryCodec;
use super::paths::{
    ENTRY_ID_LEN, encrypted_entry_path_with_id, entry_assets_dir, entry_path_with_id, random_id,
};
use crate::AppResult;
use anyhow::bail;
use chrono::{DateTime, FixedOffset, Local};
use notema_domain::{AirQuality, Celestial, ImportSource, Location, Weather};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const ENTRY_CREATE_ATTEMPTS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryAssetOptions {
    pub download_remote: bool,
    pub replace_offline: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct EntryCreateOutcome {
    pub path: PathBuf,
    pub assets: AssetReport,
}

pub struct EntryDraft<'a> {
    pub journal: &'a str,
    pub body: &'a str,
    pub metadata: &'a Metadata,
    pub created_at: Option<DateTime<FixedOffset>>,
    pub edited_at: Option<DateTime<FixedOffset>>,
    pub timezone: Option<&'a str>,
    pub location: Option<&'a Location>,
    pub weather: Option<&'a Weather>,
    pub celestial: Option<&'a Celestial>,
    pub air_quality: Option<&'a AirQuality>,
    pub writing_seconds: Option<u64>,
    pub import: Option<&'a ImportSource>,
}

impl<'a> EntryDraft<'a> {
    pub fn new(journal: &'a str, body: &'a str, metadata: &'a Metadata) -> Self {
        Self {
            journal,
            body,
            metadata,
            created_at: None,
            edited_at: None,
            timezone: None,
            location: metadata.location.as_ref(),
            weather: None,
            celestial: None,
            air_quality: None,
            writing_seconds: None,
            import: None,
        }
    }
}

/// Create a new entry from an in-memory draft. Asset references are rewritten
/// before the final entry content is encoded and written, so encrypted stores do
/// not need a read/decrypt/write pass after creation.
pub(crate) fn create_entry(
    codec: &EntryCodec<'_>,
    root: &Path,
    draft: EntryDraft<'_>,
    assets: EntryAssetOptions,
) -> AppResult<EntryCreateOutcome> {
    create_entry_inner(codec, root, None, draft, assets)
}

pub(crate) fn create_entry_copy(
    codec: &EntryCodec<'_>,
    root: &Path,
    source_path: &Path,
    draft: EntryDraft<'_>,
    assets: EntryAssetOptions,
) -> AppResult<EntryCreateOutcome> {
    create_entry_inner(codec, root, Some(source_path), draft, assets)
}

fn create_entry_inner(
    codec: &EntryCodec<'_>,
    root: &Path,
    source_path: Option<&Path>,
    draft: EntryDraft<'_>,
    assets: EntryAssetOptions,
) -> AppResult<EntryCreateOutcome> {
    let native_timestamp = draft.created_at.is_none();
    let created_at = draft
        .created_at
        .unwrap_or_else(|| Local::now().fixed_offset());
    let edited_at = draft.edited_at.unwrap_or(created_at);
    let local_timezone = native_timestamp.then(|| iana_time_zone::get_timezone().ok());
    let timezone = draft
        .timezone
        .or_else(|| local_timezone.as_ref().and_then(Option::as_deref));

    for _ in 0..ENTRY_CREATE_ATTEMPTS {
        let id = random_id(ENTRY_ID_LEN);
        let path = if codec.encrypts_new_entries() {
            encrypted_entry_path_with_id(root, draft.journal, created_at, &id)
        } else {
            entry_path_with_id(root, draft.journal, created_at, &id)
        };
        if path.exists() || entry_assets_dir(&path).is_some_and(|assets| assets.exists()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let source_body = if let Some(source_path) = source_path {
            match clone_entry_assets(source_path, &path, draft.body) {
                Ok(body) => body,
                Err(error) => {
                    remove_assets_dir(&path);
                    return Err(error);
                }
            }
        } else {
            draft.body.to_string()
        };

        let encryption = codec
            .encrypts_new_entries()
            .then(|| codec.encryption_paths());
        let (rewritten_body, report) = ingest_and_cleanup_opts(
            &path,
            &source_body,
            encryption,
            assets.download_remote,
            assets.replace_offline,
        )?;
        let body = rewritten_body.as_deref().unwrap_or(&source_body);
        let content = entry_content(
            created_at,
            edited_at,
            body,
            draft.metadata,
            timezone,
            draft.location,
            draft.weather,
            draft.celestial,
            draft.air_quality,
            draft.writing_seconds,
            draft.import,
        );
        let bytes = match codec.encode_new(&content) {
            Ok(bytes) => bytes,
            Err(error) => {
                remove_assets_dir(&path);
                return Err(error);
            }
        };

        match write_new_file(&path, &bytes) {
            Ok(()) => {
                return Ok(EntryCreateOutcome {
                    path,
                    assets: report,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                remove_assets_dir(&path);
                continue;
            }
            Err(error) => {
                remove_assets_dir(&path);
                return Err(error.into());
            }
        }
    }

    bail!("could not create a unique entry path after {ENTRY_CREATE_ATTEMPTS} attempts")
}

/// Copy raw stored assets (ciphertext stays ciphertext) and retarget canonical
/// body links to the new entry's asset directory. The normal ingestion pass
/// then removes unreferenced copies and handles any newly added images.
fn clone_entry_assets(source_path: &Path, target_path: &Path, body: &str) -> AppResult<String> {
    let (Some(source_dir), Some(target_dir), Some(source_name), Some(target_name)) = (
        entry_assets_dir(source_path),
        entry_assets_dir(target_path),
        super::paths::entry_assets_dir_name(source_path),
        super::paths::entry_assets_dir_name(target_path),
    ) else {
        return Ok(body.to_string());
    };
    if source_dir.exists() {
        fs::create_dir_all(&target_dir)?;
        for item in fs::read_dir(&source_dir)? {
            let item = item?;
            if item.file_type()?.is_file() {
                fs::copy(item.path(), target_dir.join(item.file_name()))?;
            }
        }
    }
    Ok(super::assets::retarget_stored_image_links(
        body,
        &source_name,
        &target_name,
    ))
}

#[allow(clippy::too_many_arguments)]
fn entry_content(
    created_at: DateTime<FixedOffset>,
    edited_at: DateTime<FixedOffset>,
    body: &str,
    metadata: &Metadata,
    timezone: Option<&str>,
    location: Option<&Location>,
    weather: Option<&Weather>,
    celestial: Option<&Celestial>,
    air_quality: Option<&AirQuality>,
    writing_seconds: Option<u64>,
    import: Option<&ImportSource>,
) -> String {
    let front_matter = crate::markdown::FrontMatter {
        schema_version: crate::markdown::ENTRY_SCHEMA_VERSION,
        metadata: metadata.clone(),
        datetime: crate::markdown::EntryTimestamps {
            created_at: Some(created_at.to_rfc3339()),
            edited_at: Some(edited_at.to_rfc3339()),
            timezone: timezone.map(str::to_string),
            writing_seconds,
        },
        import: import.cloned(),
        location: location.cloned(),
        weather: weather.cloned(),
        celestial: celestial.cloned(),
        air_quality: air_quality.cloned(),
    };
    let mut content = crate::markdown::render_entry(&front_matter, body);
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content
}

/// Create a fresh entry file, retrying with new ids until a free path is found.
///
/// For encrypted entries the ciphertext is computed once up front — it does not
/// depend on the path — and the retry loop only re-attempts the atomic
/// `create_new` write, so a rare id collision never re-encrypts.
#[cfg(test)]
pub(crate) fn create_entry_file(
    codec: &EntryCodec<'_>,
    root: &Path,
    journal: &str,
    now: DateTime<FixedOffset>,
    content: &str,
    mut id_generator: impl FnMut() -> String,
) -> AppResult<PathBuf> {
    let encrypted = codec.encrypts_new_entries();
    let bytes = codec.encode_new(content)?;

    for _ in 0..ENTRY_CREATE_ATTEMPTS {
        let id = id_generator();
        let path = if encrypted {
            encrypted_entry_path_with_id(root, journal, now, &id)
        } else {
            entry_path_with_id(root, journal, now, &id)
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        match write_new_file(&path, &bytes) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    bail!("could not create a unique entry path after {ENTRY_CREATE_ATTEMPTS} attempts")
}

fn remove_assets_dir(path: &Path) {
    if let Some(assets_dir) = entry_assets_dir(path) {
        let _ = fs::remove_dir_all(assets_dir);
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}
