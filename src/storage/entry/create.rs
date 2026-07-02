use super::edit::{edit_encrypted_entry, open_editor, set_updated_at_now};
use super::paths::{ENTRY_ID_LEN, encrypted_entry_path_with_id, entry_path_with_id};
use crate::{AppResult, crypto, markdown::entry_has_body};
use chrono::{DateTime, Local};
use nanoid::nanoid;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const ENTRY_CREATE_ATTEMPTS: usize = 32;

/// How a new entry file's bytes should be produced on disk.
pub(crate) enum WriteTarget<'a> {
    Plain,
    Encrypted(&'a crypto::EncryptionPaths),
}

pub fn create_entry(root: &Path, journal: &str, editor: &str) -> AppResult<Option<PathBuf>> {
    let now = Local::now();
    let content = entry_template(now, now);
    let path = create_entry_file(root, journal, now, &content, WriteTarget::Plain, || {
        nanoid!(ENTRY_ID_LEN)
    })?;
    open_editor(editor, &path)?;
    if !entry_has_body(&fs::read_to_string(&path)?) {
        fs::remove_file(&path)?;
        return Ok(None);
    }
    set_updated_at_now(&path)?;
    Ok(Some(path))
}

pub fn create_encrypted_entry(
    root: &Path,
    journal: &str,
    editor: &str,
    paths: &crypto::EncryptionPaths,
    identity: &crypto::UnlockedIdentity,
) -> AppResult<Option<PathBuf>> {
    let now = Local::now();
    let content = entry_template(now, now);
    let path = create_entry_file(
        root,
        journal,
        now,
        &content,
        WriteTarget::Encrypted(paths),
        || nanoid!(ENTRY_ID_LEN),
    )?;
    edit_encrypted_entry(&path, editor, paths, identity, true)?;
    if path.exists() {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

pub fn create_entry_with_body(root: &Path, journal: &str, body: &str) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_with_body(now, body, &[]);
    create_entry_file(root, journal, now, &content, WriteTarget::Plain, || {
        nanoid!(ENTRY_ID_LEN)
    })
}

pub fn create_entry_with_body_and_feelings(
    root: &Path,
    journal: &str,
    body: &str,
    feelings: &[String],
) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_with_body(now, body, feelings);
    create_entry_file(root, journal, now, &content, WriteTarget::Plain, || {
        nanoid!(ENTRY_ID_LEN)
    })
}

pub fn create_encrypted_entry_with_body(
    root: &Path,
    journal: &str,
    body: &str,
    paths: &crypto::EncryptionPaths,
) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_with_body(now, body, &[]);
    create_entry_file(
        root,
        journal,
        now,
        &content,
        WriteTarget::Encrypted(paths),
        || nanoid!(ENTRY_ID_LEN),
    )
}

pub fn create_encrypted_entry_with_body_and_feelings(
    root: &Path,
    journal: &str,
    body: &str,
    feelings: &[String],
    paths: &crypto::EncryptionPaths,
) -> AppResult<PathBuf> {
    let now = Local::now();
    let content = entry_with_body(now, body, feelings);
    create_entry_file(
        root,
        journal,
        now,
        &content,
        WriteTarget::Encrypted(paths),
        || nanoid!(ENTRY_ID_LEN),
    )
}

fn entry_with_body(now: DateTime<Local>, body: &str, feelings: &[String]) -> String {
    let mut content = entry_template(now, now);
    if !feelings.is_empty() {
        content =
            crate::markdown::set_feelings_in_front_matter(&content, feelings).unwrap_or(content);
    }
    content.push_str(body);
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
    root: &Path,
    journal: &str,
    now: DateTime<Local>,
    content: &str,
    target: WriteTarget<'_>,
    mut id_generator: impl FnMut() -> String,
) -> AppResult<PathBuf> {
    let encrypted = matches!(target, WriteTarget::Encrypted(_));
    let bytes = match target {
        WriteTarget::Plain => content.as_bytes().to_vec(),
        WriteTarget::Encrypted(paths) => crypto::encrypt_bytes(paths, content.as_bytes())?,
    };

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

pub fn entry_template(created_at: DateTime<Local>, updated_at: DateTime<Local>) -> String {
    format!(
        "---\ncreated_at: \"{}\"\nupdated_at: \"{}\"\ntags: []\nfeelings: []\n...\n\n",
        created_at.to_rfc3339(),
        updated_at.to_rfc3339()
    )
}
