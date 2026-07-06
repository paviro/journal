use super::edit::{write_encrypted_entry_content, write_plain_atomic};
use super::paths::is_encrypted_entry_file;
use super::read::read_entry_content;
use crate::AppResult;
use crate::crypto::{self, EncryptionPaths, UnlockedIdentity};
use crate::markdown;
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

/// An entry read and split into its raw front matter and trimmed body, ready to
/// edit and write back via [`EntryCodec::write_body`].
pub(crate) struct OpenEntry {
    pub(crate) front_matter: Option<String>,
    pub(crate) body: String,
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
        crypto::has_recipients_file(&self.paths)
    }

    /// The recipient/identity file locations, for the encrypt side.
    pub(crate) fn encryption_paths(&self) -> &EncryptionPaths {
        &self.paths
    }

    /// Encode `content` for a freshly created entry: age ciphertext when this
    /// store encrypts new entries, plain UTF-8 bytes otherwise. Pairs with the
    /// `.md`/`.md.age` path choice keyed off [`encrypts_new_entries`].
    ///
    /// [`encrypts_new_entries`]: Self::encrypts_new_entries
    pub(crate) fn encode_new(&self, content: &str) -> AppResult<Vec<u8>> {
        if self.encrypts_new_entries() {
            crypto::encrypt_bytes(&self.paths, content.as_bytes())
        } else {
            Ok(content.as_bytes().to_vec())
        }
    }

    /// Read an entry file to text, decrypting when it is a `.age` entry. Errors
    /// if the entry is encrypted but this codec has no identity.
    pub(crate) fn read(&self, path: &Path) -> AppResult<String> {
        read_entry_content(path, self.identity.as_ref())
    }

    /// Read an entry and split it into its raw front matter (if any) and its
    /// body with leading blank lines trimmed — the shared opening step of every
    /// in-place body edit. Pair with [`write_body`](Self::write_body).
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

    /// Reassemble an entry from its (still-unparsed) `front_matter` and a new
    /// `body`, refresh `updated_at`, and write it back in place. With no front
    /// matter the body is written verbatim. Front matter that fails to parse is
    /// preserved verbatim (only the body changes) rather than being overwritten
    /// with defaults, so a body-only rewrite never silently drops metadata.
    pub(crate) fn write_body(
        &self,
        path: &Path,
        front_matter: Option<&str>,
        body: &str,
    ) -> AppResult<()> {
        let content = match front_matter {
            Some(front_matter) => match markdown::parse_front_matter(front_matter) {
                Some(mut parsed) => {
                    parsed.updated_at = Some(chrono::Local::now().to_rfc3339());
                    markdown::render_entry(&parsed, body)
                }
                None => format!(
                    "+++\n{front_matter}\n+++\n\n{}",
                    body.trim_start_matches('\n')
                ),
            },
            None => body.to_string(),
        };
        self.write_existing(path, &content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn write_body_preserves_unparseable_front_matter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-07-06T10-00-00.md");
        // Unterminated array: malformed TOML that a body-only rewrite must not
        // silently replace with default (empty) metadata.
        let original = "+++\ntags = [unterminated\n+++\n\nold body\n";
        fs::write(&path, original).unwrap();

        let (front_matter, _) = markdown::split_front_matter(original);
        EntryCodec::plain()
            .write_body(&path, front_matter, "new body\n")
            .unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("tags = [unterminated"),
            "metadata preserved"
        );
        assert!(written.contains("new body"), "body updated");
    }
}
