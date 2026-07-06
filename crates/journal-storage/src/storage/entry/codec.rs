use super::edit::{write_encrypted_entry_content, write_plain_atomic};
use super::paths::is_encrypted_entry_file;
use super::read::read_entry_content_with_identity;
use crate::AppResult;
use crate::crypto::{self, EncryptionPaths, UnlockedIdentity};
use std::path::Path;

/// The crypto material for reading and writing a store's entry files, plus the
/// policy for whether new entries are encrypted. Reading decrypts `.age`
/// entries; writing an existing entry preserves its on-disk encryption; creating
/// a new entry follows [`encrypts_new_entries`](Self::encrypts_new_entries).
#[derive(Clone)]
pub(crate) struct EntryCodec {
    paths: EncryptionPaths,
    identity: Option<UnlockedIdentity>,
}

impl EntryCodec {
    pub(crate) fn new(paths: EncryptionPaths, identity: Option<UnlockedIdentity>) -> Self {
        Self { paths, identity }
    }

    /// A codec that never encrypts and holds no identity.
    #[cfg(test)]
    pub(crate) fn plain() -> Self {
        Self {
            paths: EncryptionPaths {
                config_dir: std::path::PathBuf::new(),
                recipients_file: std::path::PathBuf::new(),
                identity_file: std::path::PathBuf::new(),
            },
            identity: None,
        }
    }

    /// Whether newly created entries are written encrypted (i.e. a recipients
    /// file exists for this store). Independent of whether the store is unlocked:
    /// encryption only needs the recipient, decryption needs the identity.
    pub(crate) fn encrypts_new_entries(&self) -> bool {
        crypto::should_encrypt(&self.paths)
    }

    /// The recipient/identity file locations, for the encrypt side.
    pub(crate) fn recipients(&self) -> &EncryptionPaths {
        &self.paths
    }

    /// Read an entry file to text, decrypting when it is a `.age` entry. Errors
    /// if the entry is encrypted but this codec has no identity.
    pub(crate) fn read(&self, path: &Path) -> AppResult<String> {
        read_entry_content_with_identity(path, self.identity.as_ref())
    }

    /// Overwrite an existing entry file in place, preserving its on-disk
    /// encryption. Atomic via temp + rename.
    pub(crate) fn write_existing(&self, path: &Path, content: &str) -> AppResult<()> {
        if is_encrypted_entry_file(path) {
            write_encrypted_entry_content(&self.paths, path, content)
        } else {
            write_plain_atomic(path, content)
        }
    }
}
