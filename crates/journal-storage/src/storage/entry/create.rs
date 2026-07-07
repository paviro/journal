use super::Metadata;
use super::codec::EntryCodec;
use super::paths::{ENTRY_ID_LEN, encrypted_entry_path_with_id, entry_path_with_id};
use crate::AppResult;
use chrono::{DateTime, Local};
use nanoid::nanoid;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const ENTRY_CREATE_ATTEMPTS: usize = 32;

/// Create a new entry dated now. Whether it is encrypted follows the `codec`.
pub fn create_entry(
    codec: &EntryCodec<'_>,
    root: &Path,
    journal: &str,
    body: &str,
    metadata: &Metadata,
) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_content(now, now, body, metadata, None);
    create_entry_file(codec, root, journal, now, &content, || {
        nanoid!(ENTRY_ID_LEN)
    })
}

/// Create an entry that carries an explicit creation/modification date and an
/// `import_id` provenance marker (used by importers). The on-disk path and
/// filename are derived from `created_at`, so imported entries land in their
/// original date folder rather than today's. Encryption follows the `codec`.
#[allow(clippy::too_many_arguments)]
pub fn create_imported_entry(
    codec: &EntryCodec<'_>,
    root: &Path,
    journal: &str,
    body: &str,
    metadata: &Metadata,
    created_at: DateTime<Local>,
    updated_at: DateTime<Local>,
    import_id: &str,
) -> AppResult<PathBuf> {
    let content = entry_content(created_at, updated_at, body, metadata, Some(import_id));
    create_entry_file(codec, root, journal, created_at, &content, || {
        nanoid!(ENTRY_ID_LEN)
    })
}

fn entry_content(
    created_at: DateTime<Local>,
    updated_at: DateTime<Local>,
    body: &str,
    metadata: &Metadata,
    import_id: Option<&str>,
) -> String {
    let front_matter = crate::markdown::FrontMatter {
        created_at: Some(created_at.to_rfc3339()),
        updated_at: Some(updated_at.to_rfc3339()),
        metadata: metadata.clone(),
        import_id: import_id.map(str::to_string),
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
pub(crate) fn create_entry_file(
    codec: &EntryCodec<'_>,
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
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

    Err(
        format!("could not create a unique entry path after {ENTRY_CREATE_ATTEMPTS} attempts")
            .into(),
    )
}

fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}
