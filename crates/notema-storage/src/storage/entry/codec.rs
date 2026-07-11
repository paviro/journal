use super::edit::{write_encrypted_entry_content, write_plain_atomic};
use super::paths::is_encrypted_entry_file;
use super::read::read_entry_content;
use crate::markdown;
use notema_core::AppResult;
use notema_encryption::{self as crypto, KeyPaths, UnlockedIdentity};
use std::path::Path;

/// The crypto material for reading and writing a store's entry files, plus the
/// policy for whether new entries are encrypted. Reading decrypts `.age`
/// entries; writing an existing entry preserves its on-disk encryption; creating
/// a new entry follows [`encrypts_new_entries`](Self::encrypts_new_entries).
///
/// The identity is borrowed rather than owned so a codec — rebuilt for every
/// entry operation — never clones the store's private key material.
#[derive(Clone)]
pub(crate) struct EntryCodec<'a> {
    paths: KeyPaths,
    identity: Option<&'a UnlockedIdentity>,
}

/// An entry read and split into its raw front matter and trimmed body, ready for
/// the single-pass edit save path.
pub(crate) struct OpenEntry {
    pub(crate) front_matter: Option<String>,
    pub(crate) body: String,
}

impl<'a> EntryCodec<'a> {
    pub(crate) fn new(paths: KeyPaths, identity: Option<&'a UnlockedIdentity>) -> Self {
        Self { paths, identity }
    }

    /// A codec that never encrypts and holds no identity.
    #[cfg(test)]
    pub(crate) fn plain() -> EntryCodec<'static> {
        EntryCodec {
            paths: KeyPaths {
                age_dir: std::path::PathBuf::new(),
                devices_file: std::path::PathBuf::new(),
                identity_file: std::path::PathBuf::new(),
                trust_file: std::path::PathBuf::new(),
            },
            identity: None,
        }
    }

    /// Whether newly created entries are written encrypted (i.e. a recipients
    /// file exists for this store). Independent of whether the store is unlocked:
    /// encryption only needs the recipient, decryption needs the identity.
    pub(crate) fn encrypts_new_entries(&self) -> bool {
        self.paths.has_roster()
    }

    /// The store's key-material locations, for the asset-encryption side.
    pub(crate) fn encryption_paths(&self) -> &KeyPaths {
        &self.paths
    }

    /// Encode `content` for a freshly created entry: age ciphertext when this
    /// store encrypts new entries, plain UTF-8 bytes otherwise. Pairs with the
    /// `.md`/`.md.age` path choice keyed off [`encrypts_new_entries`].
    ///
    /// [`encrypts_new_entries`]: Self::encrypts_new_entries
    pub(crate) fn encode_new(&self, content: &str) -> AppResult<Vec<u8>> {
        if self.encrypts_new_entries() {
            let plaintext = crypto::PlaintextBytes::copy_from_slice(content.as_bytes());
            Ok(crypto::encrypt_new_entry(&self.paths, &plaintext, self.identity)?.into_vec())
        } else {
            Ok(content.as_bytes().to_vec())
        }
    }

    /// Read an entry file to text, decrypting when it is a `.age` entry. Errors
    /// if the entry is encrypted but this codec has no identity.
    pub(crate) fn read(&self, path: &Path) -> AppResult<String> {
        read_entry_content(path, self.identity)
    }

    /// Read an entry and split it into its raw front matter (if any) and its
    /// body with leading blank lines trimmed.
    pub(crate) fn open(&self, path: &Path) -> AppResult<OpenEntry> {
        let content = self.read(path)?;
        let (front_matter, body) = markdown::split_front_matter(&content);
        Ok(OpenEntry {
            front_matter: front_matter.map(str::to_string),
            body: body.trim_start_matches('\n').to_string(),
        })
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
