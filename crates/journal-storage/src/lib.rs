use std::{
    fs,
    path::{Path, PathBuf},
};

mod crypto;
mod error;
pub(crate) mod markdown;
mod migrate;
mod roster;
mod storage;

pub use age::secrecy::{ExposeSecret, SecretString};
pub use crypto::{DeviceIdentityInfo, PendingRequest, Recipient};
pub use error::StorageError;
pub use journal_core::{
    AppResult, Entry, EntryEncryptionState, EntryPath, MOOD_RANGE, Metadata, MetadataField,
    SearchHit, SearchScope, Timestamp, search_loaded_entries,
};
pub use migrate::{DecryptSummary, MigrationSummary};
pub use storage::{
    AssetFailure, AssetReport, Journal, entry_group_date, entry_id, entry_timestamp_label,
    is_entry_file, parse_entry_timestamp, sole_stored_image, stored_image_reference,
};

/// Decode image bytes to a displayable sRGB image with EXIF orientation baked
/// into the pixels. Both normalizations matter for terminal rendering:
/// orientation, because re-encoding drops EXIF; and Display P3 -> sRGB, because
/// `image` ignores the ICC profile and a terminal is not color-managed, so
/// wider-gamut pixels would render desaturated.
///
/// `max_dimensions` (pixels) downscales *before* the color transform and
/// encoding, bounding peak memory to the display size rather than the source
/// resolution. `None` keeps full resolution.
pub fn decode_image_with_orientation(
    bytes: &[u8],
    max_dimensions: Option<(u32, u32)>,
) -> AppResult<image::DynamicImage> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .into_decoder()?;
    let icc_profile = decoder.icc_profile().ok().flatten();
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);

    // Shrink to the display target before the per-pixel work; never upscale.
    if let Some((max_width, max_height)) = max_dimensions
        && (image.width() > max_width || image.height() > max_height)
    {
        image = image.resize(max_width, max_height, image::imageops::FilterType::Triangle);
    }

    if let Some(icc) = icc_profile.filter(|profile| !profile.is_empty())
        && let Some(converted) = convert_to_srgb(&image, &icc)
    {
        image = converted;
    }
    Ok(image)
}

/// Convert an image's pixels from its embedded ICC color space to sRGB.
/// Best-effort: returns `None` if the profile can't be parsed or the transform
/// fails, leaving the caller to use the un-converted image.
fn convert_to_srgb(image: &image::DynamicImage, icc: &[u8]) -> Option<image::DynamicImage> {
    use moxcms::{ColorProfile, Layout, TransformOptions};
    let source = ColorProfile::new_from_slice(icc).ok()?;
    let srgb = ColorProfile::new_srgb();
    let transform = source
        .create_transform_8bit(Layout::Rgb, &srgb, Layout::Rgb, TransformOptions::default())
        .ok()?;
    let rgb = image.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let mut out = vec![0u8; rgb.as_raw().len()];
    transform.transform(rgb.as_raw(), &mut out).ok()?;
    Some(image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(
        width, height, out,
    )?))
}

/// A unique hidden sibling temp path next to `target`, for atomic
/// write-then-rename. Named `.journal-<pid>-<rand>.<suffix>` in the target's
/// directory so it lands on the same filesystem as the eventual rename target.
pub(crate) fn sibling_temp_path(target: &Path, suffix: &str) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(
        ".journal-{}-{}.{suffix}",
        std::process::id(),
        nanoid::nanoid!(12),
    ))
}

#[derive(Clone)]
pub struct JournalStore {
    paths: JournalStorePaths,
    identity: Option<crypto::UnlockedIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalStorePaths {
    pub journal_root: PathBuf,
    /// The hidden, synced key folder holding the signed `devices.toml` roster and
    /// any `pending-<id>.toml` join requests.
    pub age_dir: PathBuf,
    /// The signed, append-only device roster (`<root>/.age/devices.toml`).
    pub devices_file: PathBuf,
    pub identity_file: PathBuf,
    /// This device's local trust pins for the roster (genesis + last-seen head).
    /// Sits next to the identity, never synced, so a sync-folder attacker can't
    /// reach it.
    pub trust_file: PathBuf,
}

impl JournalStorePaths {
    /// Derive the store's file locations from the journal root and the config
    /// directory: public key material lives in the synced `<root>/.age/` folder,
    /// the private age identity and roster trust pins next to the config (never
    /// synced).
    pub fn new(journal_root: impl Into<PathBuf>, config_dir: impl AsRef<Path>) -> Self {
        let journal_root = journal_root.into();
        let age_dir = journal_root.join(".age");
        let config_dir = config_dir.as_ref();
        Self {
            devices_file: age_dir.join("devices.toml"),
            identity_file: config_dir.join("identity.age"),
            trust_file: config_dir.join("devices-trust.toml"),
            age_dir,
            journal_root,
        }
    }

    /// Like [`new`](Self::new), taking the config *file* and reading its parent
    /// directory for the identity location.
    pub fn for_config(config_path: &Path, journal_root: &Path) -> AppResult<Self> {
        let config_dir = config_path
            .parent()
            .ok_or("config path has no parent directory")?;
        Ok(Self::new(journal_root, config_dir))
    }
}

impl JournalStore {
    pub fn new(journal_root: impl Into<PathBuf>, config_dir: impl AsRef<Path>) -> Self {
        Self {
            paths: JournalStorePaths::new(journal_root, config_dir),
            identity: None,
        }
    }

    pub fn for_config(config_path: &Path, journal_root: &Path) -> AppResult<Self> {
        Ok(Self {
            paths: JournalStorePaths::for_config(config_path, journal_root)?,
            identity: None,
        })
    }

    pub fn paths(&self) -> &JournalStorePaths {
        &self.paths
    }

    pub fn ensure(&self) -> AppResult<()> {
        storage::ensure_store(&self.paths.journal_root)
    }

    pub fn encryption_enabled(&self) -> bool {
        crypto::has_devices_file(&self.paths)
    }

    pub fn unlock_available(&self) -> bool {
        crypto::has_identity_file(&self.paths)
    }

    pub fn public_recipient(&self) -> AppResult<String> {
        crypto::public_recipient(&self.paths)
    }

    /// Every device the store is currently encrypted to.
    pub fn recipients(&self) -> AppResult<Vec<Recipient>> {
        crypto::read_recipients(&self.paths)
    }

    /// Join requests dropped into the shared `.age/` folder awaiting approval.
    pub fn pending_requests(&self) -> AppResult<Vec<PendingRequest>> {
        crypto::read_pending(&self.paths)
    }

    /// This device's stored identity label and passphrase state, or `None` if no
    /// identity has been generated here yet.
    pub fn this_device(&self) -> AppResult<Option<DeviceIdentityInfo>> {
        crypto::device_identity_info(&self.paths)
    }

    /// Whether unlocking this device's identity requires a passphrase. `false`
    /// for a plaintext identity (or when no identity exists), so the caller can
    /// skip the unlock prompt and auto-load.
    pub fn identity_needs_passphrase(&self) -> AppResult<bool> {
        Ok(
            crypto::device_identity_info(&self.paths)?
                .is_some_and(|info| info.passphrase_protected),
        )
    }

    pub fn has_encrypted_entries(&self) -> AppResult<bool> {
        migrate::store_has_encrypted_entry_files(self)
    }

    /// Create this store's encryption on the device that owns it: generate this
    /// device's identity (passphrase-protected when `passphrase` is `Some`) and
    /// record it as the store's first recipient. Returns its public key.
    pub fn initialize_encryption(
        &self,
        device_name: &str,
        passphrase: Option<&SecretString>,
    ) -> AppResult<String> {
        Ok(crypto::initialize_store_identity(&self.paths, device_name, passphrase)?.key)
    }

    /// Generate this device's identity for a store that already exists elsewhere
    /// and drop a join request into the shared folder for another device to
    /// approve. Returns this device's [`Recipient`].
    pub fn request_access(
        &self,
        device_name: &str,
        passphrase: Option<&SecretString>,
    ) -> AppResult<Recipient> {
        crypto::request_store_access(&self.paths, device_name, passphrase)
    }

    /// Whether this device's unlocked identity is one of the store's current
    /// recipients — a device that can already decrypt, and so may re-encrypt the
    /// store to approve or remove others. `false` when locked or not yet approved.
    pub fn is_current_recipient(&self) -> AppResult<bool> {
        match &self.identity {
            Some(identity) => crypto::identity_is_recipient(&self.paths, identity),
            None => Ok(false),
        }
    }

    /// This device's unlocked identity public key, or `None` when locked.
    pub fn identity_public_key(&self) -> Option<String> {
        self.identity.as_ref().map(|identity| identity.public_key())
    }

    /// Whether a join request for *this* device's key is still queued in the
    /// shared `.age/` folder. Lets a non-recipient device tell "waiting for
    /// approval" (its request is pending) apart from "not authorized and nothing
    /// queued" (denied, removed, or never synced). `false` when locked.
    pub fn self_request_pending(&self) -> AppResult<bool> {
        let Some(own_key) = self.identity_public_key() else {
            return Ok(false);
        };
        Ok(self
            .pending_requests()?
            .iter()
            .any(|request| request.recipient.key == own_key))
    }

    /// Add `recipient` and re-encrypt every entry (and asset) to the new set so
    /// the added device can read history. Requires an unlocked identity that is
    /// already a store recipient.
    pub fn add_recipient(
        &self,
        recipient: Recipient,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<MigrationSummary> {
        let identity = self.require_reencrypt_identity("add-recipient")?;
        let summary = migrate::atomic(self, || {
            crypto::add_recipient(&self.paths, identity, &recipient)?;
            migrate::reencrypt_store(self, identity, &mut progress)
        })?;
        crypto::advance_trust_pins(&self.paths)?;
        Ok(summary)
    }

    /// Remove the recipient named `name` and re-encrypt every entry to exclude
    /// it. Revocation is forward-only — entries the removed device already synced
    /// stay readable to it. Requires an unlocked identity.
    pub fn remove_recipient(
        &self,
        name: &str,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<MigrationSummary> {
        let identity = self.require_reencrypt_identity("remove-recipient")?;
        // Match on the key, not the name: a device that was renamed still stores
        // its old name locally, so a name check could be sidestepped by renaming
        // first and would then re-encrypt this device out of its own history.
        let own_key = identity.public_key();
        if self
            .recipients()?
            .iter()
            .any(|recipient| recipient.name == name && recipient.key == own_key)
        {
            return Err("refusing to remove this device's own recipient".into());
        }
        let summary = migrate::atomic(self, || {
            crypto::remove_recipient(&self.paths, identity, name)?;
            migrate::reencrypt_store(self, identity, &mut progress)
        })?;
        crypto::advance_trust_pins(&self.paths)?;
        Ok(summary)
    }

    /// Relabel a recipient by appending a signed `rename` op. No re-encryption
    /// needed, but it must be signed, so it requires an unlocked recipient
    /// identity — an unsigned relabel would be a tamper vector.
    pub fn rename_recipient(&self, old: &str, new: &str) -> AppResult<()> {
        let identity = self.require_reencrypt_identity("rename-recipient")?;
        crypto::rename_recipient(&self.paths, identity, old, new)?;
        crypto::advance_trust_pins(&self.paths)?;
        Ok(())
    }

    /// Add, remove, or change the passphrase on this device's identity. `current`
    /// unlocks it as stored now (required when passphrase-protected), `new`
    /// chooses how to store it (`Some` = passphrase, `None` = plaintext). Only the
    /// local identity file changes; entries are untouched.
    pub fn set_passphrase(
        &self,
        current: Option<&SecretString>,
        new: Option<&SecretString>,
    ) -> AppResult<()> {
        crypto::set_identity_passphrase(&self.paths, current, new)
    }

    /// Rotate this device's keypair: generate a new key, re-encrypt all entries
    /// so the old key can no longer read history, and retire it. Requires the
    /// store already unlocked with the current key; `passphrase` re-wraps the new
    /// key (pass the same one used to unlock, or `None` for a plaintext identity).
    ///
    /// Ordered so no intermediate state can lock this device out: the new key is
    /// added alongside the old and everything re-encrypted to both, the new
    /// identity is committed, then the old key is dropped and everything
    /// re-encrypted to the new key alone.
    ///
    /// The whole rotation is transactional: the journal root and this device's
    /// identity file (which lives outside the root) are snapshot up front, and
    /// any failure rolls both back — and restores the in-memory identity — so a
    /// botched rotation leaves the device exactly as it was.
    pub fn rotate_identity(
        &mut self,
        passphrase: Option<&SecretString>,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<MigrationSummary> {
        let old = self.require_reencrypt_identity("rotate")?.clone();
        let old_key = old.public_key();
        let root = self.paths.journal_root.clone();

        let identity_backup = crypto::read_identity_file_bytes(&self.paths)?;
        let trust_backup = fs::read(&self.paths.trust_file).ok();
        let backup = migrate::backup_store(&root)?;

        let result = (|| -> AppResult<MigrationSummary> {
            let (recipient, new_identity) = crypto::rotate_add_new_key(&self.paths, &old)?;
            let first = migrate::reencrypt_store(self, &old, &mut progress)?;

            crypto::commit_rotated_identity(&self.paths, &recipient, &new_identity, passphrase)?;
            self.identity = Some(new_identity);

            // Retire the old key, signed by the freshly rotated key (now trusted).
            let identity = self.identity.as_ref().expect("identity set above");
            crypto::drop_old_recipient(&self.paths, identity, &old_key)?;
            let second = migrate::reencrypt_store(self, identity, &mut progress)?;

            Ok(MigrationSummary {
                migrated_files: first.migrated_files + second.migrated_files,
            })
        })();

        match result {
            Ok(summary) => {
                fs::remove_dir_all(&backup)?;
                crypto::advance_trust_pins(&self.paths)?;
                Ok(summary)
            }
            Err(error) => {
                migrate::restore_store(&root, &backup)?;
                crypto::restore_identity_file(&self.paths, &identity_backup)?;
                self.restore_trust_file(trust_backup.as_deref())?;
                self.identity = Some(old);
                Err(error)
            }
        }
    }

    /// Put the roster trust pins back to a snapshot taken before a rotation:
    /// rewrite the captured bytes, or delete the file if there were none.
    fn restore_trust_file(&self, bytes: Option<&[u8]>) -> AppResult<()> {
        match bytes {
            Some(bytes) => crypto::atomic_write(&self.paths.trust_file, bytes),
            None => {
                if self.paths.trust_file.exists() {
                    fs::remove_file(&self.paths.trust_file)?;
                }
                Ok(())
            }
        }
    }

    /// Approve a pending join request: add its recipient, re-encrypt, and delete
    /// the request file — as one atomic unit, so a failure leaves the request
    /// pending and the store unchanged. Requires an unlocked identity.
    pub fn approve_pending(
        &self,
        request: &PendingRequest,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<MigrationSummary> {
        let identity = self.require_reencrypt_identity("approve")?;
        // Idempotent: if this key is already a recipient (a stale request that
        // synced back, or this device's own request), there's nothing to
        // re-encrypt — just clear the request rather than failing on the
        // duplicate-key check inside `add_recipient`.
        if self
            .recipients()?
            .iter()
            .any(|recipient| recipient.key == request.recipient.key)
        {
            crypto::remove_pending(&self.paths, &request.id)?;
            return Ok(MigrationSummary { migrated_files: 0 });
        }
        let summary = migrate::atomic(self, || {
            crypto::add_recipient(&self.paths, identity, &request.recipient)?;
            let summary = migrate::reencrypt_store(self, identity, &mut progress)?;
            crypto::remove_pending(&self.paths, &request.id)?;
            Ok(summary)
        })?;
        crypto::advance_trust_pins(&self.paths)?;
        Ok(summary)
    }

    /// Reject a pending join request without granting access.
    pub fn deny_pending(&self, request: &PendingRequest) -> AppResult<()> {
        crypto::remove_pending(&self.paths, &request.id)
    }

    fn require_identity(&self, context: &'static str) -> AppResult<&crypto::UnlockedIdentity> {
        self.identity
            .as_ref()
            .ok_or_else(|| StorageError::LockedIdentity { context }.into())
    }

    /// The unlocked identity, but only if it's a current store recipient — the
    /// precondition for re-encrypting. A device awaiting approval can't grant
    /// history it can't itself read, so this fails with a clear message.
    fn require_reencrypt_identity(
        &self,
        context: &'static str,
    ) -> AppResult<&crypto::UnlockedIdentity> {
        let identity = self.require_identity(context)?;
        if !crypto::identity_is_recipient(&self.paths, identity)? {
            return Err("this device's key is not a current store recipient, so it cannot re-encrypt; approve it from a device that can already read this journal".into());
        }
        Ok(identity)
    }

    /// Load the age identity into this store so encrypted entries can be read
    /// and written. Pass `Some(passphrase)` for a passphrase-protected identity
    /// and `None` for a plaintext one. After this succeeds, the store
    /// transparently handles both plaintext and encrypted entries.
    pub fn unlock(&mut self, passphrase: Option<&SecretString>) -> AppResult<()> {
        self.identity = Some(crypto::unlock_identity(&self.paths, passphrase)?);
        Ok(())
    }

    pub fn is_unlocked(&self) -> bool {
        self.identity.is_some()
    }

    pub fn list_journals(&self) -> AppResult<Vec<Journal>> {
        storage::list_journals(&self.paths.journal_root)
    }

    pub fn create_journal(&self, name: &str) -> AppResult<Journal> {
        storage::create_journal(&self.paths.journal_root, name)
    }

    pub fn validate_journal_name(name: &str) -> AppResult<String> {
        storage::validate_journal_name(name)
    }

    pub fn collect_entry_paths(&self) -> AppResult<Vec<EntryPath>> {
        storage::collect_entry_paths(&self.paths.journal_root)
    }

    pub fn read_entries(&self, paths: Vec<EntryPath>) -> AppResult<Vec<Entry>> {
        storage::read_entries(paths, self.identity.as_ref())
    }

    pub fn scan_entries(&self) -> AppResult<Vec<Entry>> {
        storage::scan_entries(&self.paths.journal_root, self.identity.as_ref())
    }

    pub fn read_entry(&self, journal: &str, path: &Path) -> AppResult<Entry> {
        storage::read_entry(journal, path, self.identity.as_ref())
    }

    pub fn read_entry_content(&self, path: &Path) -> AppResult<String> {
        storage::read_entry_content(path, self.identity.as_ref())
    }

    pub fn create_entry_with_body(
        &self,
        journal: &str,
        body: &str,
        metadata: &Metadata,
    ) -> AppResult<PathBuf> {
        storage::create_entry(
            &self.entry_codec(),
            &self.paths.journal_root,
            journal,
            body,
            metadata,
        )
    }

    /// Create an entry from an external import, preserving its original
    /// creation/modification dates and recording an `import_id` provenance
    /// marker in the front matter. Encryption follows the store's setting, like
    /// [`create_entry_with_body`].
    pub fn create_imported_entry(
        &self,
        journal: &str,
        body: &str,
        metadata: &Metadata,
        created_at: chrono::DateTime<chrono::Local>,
        updated_at: chrono::DateTime<chrono::Local>,
        import_id: &str,
    ) -> AppResult<PathBuf> {
        storage::create_imported_entry(
            &self.entry_codec(),
            &self.paths.journal_root,
            journal,
            body,
            metadata,
            created_at,
            updated_at,
            import_id,
        )
    }

    /// Open a new entry in the editor. The callback receives an empty string
    /// and returns the body text the user wrote, or `None` to cancel.
    pub fn create_entry_via_editor(
        &self,
        journal: &str,
        metadata: &Metadata,
        edit: impl FnOnce(&str) -> AppResult<Option<String>>,
    ) -> AppResult<Option<PathBuf>> {
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
        edit: impl FnOnce(&str) -> AppResult<Option<String>>,
    ) -> AppResult<bool> {
        storage::edit_entry_body(&self.entry_codec(), path, remove_if_empty, edit)
    }

    /// The codec for reading and writing this store's entry files, carrying the
    /// recipients/identity and whether new entries are encrypted.
    fn entry_codec(&self) -> storage::EntryCodec {
        storage::EntryCodec::new(self.paths.clone(), self.identity.clone())
    }

    pub fn delete_journal(
        &self,
        journal_name: &str,
        journal_path: &Path,
        entries: &[(PathBuf, bool)],
    ) -> AppResult<()> {
        storage::delete_journal(
            &self.paths.journal_root,
            journal_name,
            journal_path,
            entries,
        )
    }

    pub fn move_entry_to_trash(&self, entry_path: &Path) -> AppResult<PathBuf> {
        storage::move_entry_to_trash(&self.paths.journal_root, entry_path)
    }

    pub fn delete_empty_entry(&self, path: &Path) -> AppResult<()> {
        storage::delete_empty_entry(path)
    }

    /// Replace one metadata field of an entry's front matter (and refresh
    /// `updated_at`), leaving the body untouched. A no-op if the file has no
    /// front matter.
    pub fn set_entry_metadata_field(&self, path: &Path, field: MetadataField) -> AppResult<()> {
        let codec = self.entry_codec();
        let content = codec.read(path)?;
        let Some(new_content) = markdown::with_metadata_field(&content, &field) else {
            return Ok(());
        };
        codec.write_existing(path, &new_content)
    }

    /// Ingest external image references in the entry (copy/download them into
    /// the entry's `<stem>.assets/` folder, encrypting when the entry is
    /// encrypted, and rewrite the references) and delete orphaned assets. Runs
    /// after create and edit; a no-op when the body has no external references
    /// and no orphaned assets.
    pub fn process_entry_assets(
        &self,
        path: &Path,
        download_remote: bool,
        replace_offline: bool,
    ) -> AppResult<storage::AssetReport> {
        let encrypted = storage::is_encrypted_entry_file(path);
        if encrypted && self.identity.is_none() {
            return Ok(storage::AssetReport::default());
        }

        let codec = self.entry_codec();
        let entry = codec.open(path)?;

        let encryption = encrypted.then(|| codec.encryption_paths().clone());
        let (new_body, report) = storage::ingest_and_cleanup_opts(
            path,
            &entry.body,
            encryption.as_ref(),
            download_remote,
            replace_offline,
        )?;

        if let Some(new_body) = new_body {
            codec.write_body(
                path,
                entry.front_matter.as_deref(),
                new_body.trim_start_matches('\n'),
            )?;
        }

        Ok(report)
    }

    /// Read an entry-owned image asset into memory, decrypting `.age` assets
    /// with the unlocked identity. Never writes a plaintext copy to disk, and
    /// refuses paths outside the entry's own `<stem>.assets` folder.
    pub fn read_entry_asset_bytes(
        &self,
        entry_path: &Path,
        file_name: &str,
    ) -> AppResult<Option<Vec<u8>>> {
        let Some(path) = storage::resolve_entry_asset_path(entry_path, file_name)? else {
            return Ok(None);
        };
        if path.extension().is_some_and(|ext| ext == "age") {
            let identity = self
                .identity
                .as_ref()
                .ok_or(StorageError::LockedIdentity { context: "asset" })?;
            Ok(Some(crypto::decrypt_file_bytes(identity, &path)?))
        } else {
            Ok(Some(fs::read(path)?))
        }
    }

    pub fn decrypt_store(
        &self,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<migrate::DecryptSummary> {
        let identity = self
            .identity
            .as_ref()
            .ok_or(StorageError::LockedIdentity { context: "store" })?;
        migrate::decrypt_store(self, identity, &mut progress)
    }

    pub fn encrypt_store(
        &self,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<migrate::MigrationSummary> {
        if !self.encryption_enabled() && migrate::store_has_encrypted_entry_files(self)? {
            return Err(StorageError::RecipientsMissing {
                path: self.paths.devices_file.clone(),
            }
            .into());
        }
        migrate::encrypt_store(self, &mut progress)
    }
}
