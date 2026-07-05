use std::{
    fs,
    path::{Path, PathBuf},
};

mod crypto;
pub(crate) mod markdown;
mod migrate;
mod storage;

pub use journal_core::{
    AppResult, JournalResult, Entry, EntryEncryptionState, EntryMetadata, EntryPath, SearchHit,
    SearchScopeFilter, search_loaded_entries,
};
pub use migrate::{DecryptSummary, MigrationSummary};
pub use storage::{
    Journal, entry_group_date, entry_id, entry_timestamp_label, parse_entry_timestamp,
};

pub struct JournalStore {
    paths: JournalStorePaths,
    identity: Option<crypto::UnlockedIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalStorePaths {
    pub journal_root: PathBuf,
    pub recipients_file: PathBuf,
    pub identity_file: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncryptionStatus {
    pub enabled: bool,
    pub unlock_available: bool,
}

impl JournalStore {
    pub fn new(
        journal_root: impl Into<PathBuf>,
        recipients_file: impl Into<PathBuf>,
        identity_file: impl Into<PathBuf>,
    ) -> Self {
        Self {
            paths: JournalStorePaths {
                journal_root: journal_root.into(),
                recipients_file: recipients_file.into(),
                identity_file: identity_file.into(),
            },
            identity: None,
        }
    }

    pub fn for_config(config_path: &Path, journal_root: &Path) -> JournalResult<Self> {
        let paths = crypto::EncryptionPaths::for_config(config_path, journal_root)?;
        Ok(Self::new(
            journal_root,
            paths.recipients_file,
            paths.identity_file,
        ))
    }

    pub fn paths(&self) -> &JournalStorePaths {
        &self.paths
    }

    pub fn ensure(&self) -> JournalResult<()> {
        storage::ensure_store(&self.paths.journal_root)
    }

    pub fn encryption_status(&self) -> EncryptionStatus {
        let paths = self.encryption_paths();
        EncryptionStatus {
            enabled: crypto::should_encrypt(&paths),
            unlock_available: crypto::can_decrypt(&paths),
        }
    }

    pub fn encryption_enabled(&self) -> bool {
        self.encryption_status().enabled
    }

    pub fn unlock_available(&self) -> bool {
        self.encryption_status().unlock_available
    }

    pub fn public_recipient(&self) -> JournalResult<String> {
        crypto::public_recipient(&self.encryption_paths())
    }

    pub fn has_encrypted_entries(&self) -> JournalResult<bool> {
        migrate::store_has_encrypted_entry_files(self)
    }

    pub fn initialize_encryption(&self, passphrase: &str) -> JournalResult<String> {
        crypto::generate_identity_store(&self.encryption_paths(), passphrase)
    }

    /// Load the age identity into this store so encrypted entries can be read
    /// and written. After this succeeds, the store transparently handles both
    /// plaintext and encrypted entries.
    pub fn unlock(&mut self, passphrase: &str) -> JournalResult<()> {
        self.identity = Some(crypto::unlock_identity(&self.encryption_paths(), passphrase)?);
        Ok(())
    }

    pub fn is_unlocked(&self) -> bool {
        self.identity.is_some()
    }

    pub fn list_journals(&self) -> JournalResult<Vec<Journal>> {
        storage::list_journals(&self.paths.journal_root)
    }

    pub fn create_journal(&self, name: &str) -> JournalResult<Journal> {
        storage::create_journal(&self.paths.journal_root, name)
    }

    pub fn validate_journal_name(name: &str) -> JournalResult<String> {
        storage::validate_journal_name(name)
    }

    pub fn collect_entry_paths(&self) -> JournalResult<Vec<EntryPath>> {
        storage::collect_entry_paths(&self.paths.journal_root)
    }

    pub fn read_entries(&self, paths: Vec<EntryPath>) -> JournalResult<Vec<Entry>> {
        storage::read_entries(paths, self.identity.as_ref())
    }

    pub fn scan_entries(&self) -> JournalResult<Vec<Entry>> {
        storage::scan_entries_with_identity(&self.paths.journal_root, self.identity.as_ref())
    }

    pub fn read_entry(&self, journal: &str, path: &Path) -> JournalResult<Entry> {
        storage::read_entry_with_identity(journal, path, self.identity.as_ref())
    }

    pub fn read_entry_content(&self, path: &Path) -> JournalResult<String> {
        storage::read_entry_content_with_identity(path, self.identity.as_ref())
    }

    pub fn create_entry_with_body(
        &self,
        journal: &str,
        body: &str,
        metadata: EntryMetadata<'_>,
    ) -> JournalResult<PathBuf> {
        if self.encryption_enabled() {
            storage::create_encrypted_entry_with_body_and_metadata(
                &self.paths.journal_root,
                journal,
                body,
                metadata,
                &self.encryption_paths(),
            )
        } else {
            storage::create_entry_with_body_and_metadata(
                &self.paths.journal_root,
                journal,
                body,
                metadata,
            )
        }
    }

    /// Open a new entry in the editor. The callback receives an empty string
    /// and returns the body text the user wrote, or `None` to cancel.
    pub fn create_entry_via_editor(
        &self,
        journal: &str,
        metadata: EntryMetadata<'_>,
        edit: impl FnOnce(&str) -> JournalResult<Option<String>>,
    ) -> JournalResult<Option<PathBuf>> {
        let Some(body) = edit("")? else {
            return Ok(None);
        };
        if body.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(self.create_entry_with_body(journal, &body, metadata)?))
    }

    /// Open an existing entry in the editor. The callback receives the body
    /// text and returns the edited body, or `None` to leave unchanged.
    /// Returns `true` if the entry was kept, `false` if deleted for being empty.
    pub fn edit_entry_via_editor(
        &self,
        path: &Path,
        remove_if_empty: bool,
        edit: impl FnOnce(&str) -> JournalResult<Option<String>>,
    ) -> JournalResult<bool> {
        let paths = self.encryption_paths();
        storage::edit_entry_body(
            path,
            self.entry_encryption(path, &paths)?,
            remove_if_empty,
            edit,
        )
    }

    /// Returns the encryption context to use for an entry file: `Some` for
    /// encrypted files (requires the store to be unlocked), `None` for plain
    /// files. Errors if the entry is encrypted but the store is locked.
    fn entry_encryption<'a>(
        &'a self,
        path: &Path,
        paths: &'a crypto::EncryptionPaths,
    ) -> JournalResult<Option<(&'a crypto::EncryptionPaths, &'a crypto::UnlockedIdentity)>> {
        if storage::is_encrypted_entry_file(path) {
            let identity = self
                .identity
                .as_ref()
                .ok_or("encrypted entry requires an unlocked journal encryption identity")?;
            Ok(Some((paths, identity)))
        } else {
            Ok(None)
        }
    }

    pub fn delete_journal(
        &self,
        journal_name: &str,
        journal_path: &Path,
        entries: &[(PathBuf, bool)],
    ) -> JournalResult<()> {
        storage::delete_journal(
            &self.paths.journal_root,
            journal_name,
            journal_path,
            entries,
        )
    }

    pub fn move_entry_to_trash(&self, entry_path: &Path) -> JournalResult<PathBuf> {
        storage::move_entry_to_trash(&self.paths.journal_root, entry_path)
    }

    pub fn delete_empty_entry(&self, path: &Path) -> JournalResult<()> {
        storage::delete_empty_entry(path)
    }

    pub fn set_entry_tags(&self, path: &Path, tags: &[String]) -> JournalResult<()> {
        self.update_entry_metadata(path, |content| markdown::set_tags_in_front_matter(content, tags))
    }

    pub fn set_entry_people(&self, path: &Path, people: &[String]) -> JournalResult<()> {
        self.update_entry_metadata(path, |content| {
            markdown::set_people_in_front_matter(content, people)
        })
    }

    pub fn set_entry_activities(&self, path: &Path, activities: &[String]) -> JournalResult<()> {
        self.update_entry_metadata(path, |content| {
            markdown::set_activities_in_front_matter(content, activities)
        })
    }

    pub fn set_entry_feelings(&self, path: &Path, feelings: &[String]) -> JournalResult<()> {
        self.update_entry_metadata(path, |content| {
            markdown::set_feelings_in_front_matter(content, feelings)
        })
    }

    pub fn set_entry_mood(&self, path: &Path, mood: Option<i8>) -> JournalResult<()> {
        self.update_entry_metadata(path, |content| {
            markdown::set_mood_in_front_matter(content, mood)
        })
    }

    pub(crate) fn update_entry_metadata(
        &self,
        path: &Path,
        update: impl FnOnce(&str) -> Option<String>,
    ) -> JournalResult<()> {
        if storage::is_encrypted_entry_file(path) {
            let identity = self
                .identity
                .as_ref()
                .ok_or("encrypted entry requires an unlocked journal encryption identity")?;
            let content = storage::read_entry_content_with_identity(path, Some(identity))?;
            let Some(new_content) = update(&content) else {
                return Ok(());
            };
            storage::write_encrypted_entry_content(
                &self.encryption_paths(),
                path,
                &markdown::set_updated_at_now_in_content(&new_content),
            )
        } else {
            let content = fs::read_to_string(path)?;
            let Some(new_content) = update(&content) else {
                return Ok(());
            };
            fs::write(path, markdown::set_updated_at_now_in_content(&new_content))?;
            Ok(())
        }
    }

    pub fn decrypt_store(&self) -> JournalResult<migrate::DecryptSummary> {
        let identity = self
            .identity
            .as_ref()
            .ok_or("decrypting the store requires an unlocked journal encryption identity")?;
        migrate::decrypt_store(self, identity)
    }

    pub fn encrypt_store(&self) -> JournalResult<migrate::MigrationSummary> {
        if !self.encryption_enabled() && migrate::store_has_encrypted_entry_files(self)? {
            return Err(format!(
                "encrypted entries already exist but recipients file is missing at {}; cannot safely continue encryption",
                self.paths.recipients_file.display()
            )
            .into());
        }
        migrate::encrypt_store(self)
    }

    fn encryption_paths(&self) -> crypto::EncryptionPaths {
        crypto::EncryptionPaths {
            config_dir: self
                .paths
                .identity_file
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf(),
            recipients_file: self.paths.recipients_file.clone(),
            identity_file: self.paths.identity_file.clone(),
        }
    }
}
