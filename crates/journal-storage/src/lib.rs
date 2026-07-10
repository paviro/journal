use anyhow::{Context, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod error;
pub(crate) mod markdown;
mod migrate;
mod storage;

use journal_core::{AppResult, Entry, EntryPath, ImportSource, Metadata, MetadataField};
use journal_encryption as crypto;

pub use error::StorageError;
pub use journal_encryption::{
    DeviceIdentityInfo, EncryptionError, ExposeSecret, PendingRequest, Recipient, SecretString,
};
pub use migrate::{DecryptSummary, MigrationSummary};
pub use storage::{
    ARCHIVED_SUFFIX, AssetFailure, AssetReport, EditOutcome, ImportedEntryDraft, Journal, entry_id,
    entry_timestamp_label, is_archived_name, is_entry_file, journal_display_name,
    parse_entry_timestamp, sole_stored_image, stored_image_reference,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFileEncoding {
    Plain,
    Encrypted,
}

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

#[derive(Clone)]
pub struct JournalStore {
    paths: JournalStorePaths,
    identity: Option<crypto::UnlockedIdentity>,
}

/// Whether this device can open the store once the unlock phase is done, and why
/// not when it can't. Resolved in one call so the caller matches a single
/// outcome instead of threading several encryption predicates together.
pub enum StoreAccess {
    /// Plaintext store, or this device's unlocked identity is a current
    /// recipient — ready to load.
    Ready,
    /// This device's join request is still queued; it keeps its identity while
    /// waiting for another device to approve it.
    AwaitingApproval { device_name: String },
    /// This device has no usable key. `retired_key` is true when a now-dead
    /// (revoked) key was just renamed aside during this call.
    NeedsEnroll {
        device_name: String,
        retired_key: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnableEncryptionSummary {
    pub recipient: String,
    pub migrated_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalStorePaths {
    pub journal_root: PathBuf,
    /// This store's key-material locations (roster, identity, trust pins). Owned
    /// by the encryption layer; storage only threads it through to `crypto`.
    pub keys: crypto::KeyPaths,
}

impl JournalStorePaths {
    /// Derive the store's file locations from the journal root and the config
    /// directory: public key material lives in the synced `<root>/.age/` folder,
    /// the private age identity and roster trust pins next to the config (never
    /// synced).
    pub fn new(journal_root: impl Into<PathBuf>, config_dir: impl AsRef<Path>) -> Self {
        let journal_root = journal_root.into();
        let keys = crypto::KeyPaths::new(&journal_root, config_dir);
        Self { journal_root, keys }
    }

    /// Like [`new`](Self::new), taking the config *file* and reading its parent
    /// directory for the identity location.
    pub fn for_config(config_path: &Path, journal_root: &Path) -> AppResult<Self> {
        let config_dir = config_path
            .parent()
            .context("config path has no parent directory")?;
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

    /// Pick up an encryption *disable* performed on another device: when the
    /// synced roster is gone but this device still holds the key and pins it used
    /// while encrypted, retire them locally (renamed aside, recoverable) and fall
    /// back to plaintext. Returns `true` when it just did so, so the caller can
    /// tell the user. A no-op (`false`) on a store still encrypted, never
    /// encrypted here, or with encrypted entries still to sync. Call once per open,
    /// right after [`ensure`](Self::ensure).
    pub fn reconcile_disabled_encryption(&self) -> AppResult<bool> {
        Ok(migrate::reconcile_disabled_encryption(self)?.is_some())
    }

    /// Retire this device's identity after its access was revoked; see
    /// [`migrate::retire_revoked_identity`]. Returns the renamed-aside path, or
    /// `None` when there was no identity to retire.
    pub fn retire_revoked_identity(&self) -> AppResult<Option<PathBuf>> {
        migrate::retire_revoked_identity(self)
    }

    pub fn encryption_enabled(&self) -> bool {
        self.paths.keys.has_roster()
    }

    pub fn unlock_available(&self) -> bool {
        self.paths.keys.has_identity()
    }

    /// The first recipient's public key, for display after enabling encryption.
    pub fn public_recipient(&self) -> AppResult<String> {
        crypto::read_recipients(&self.paths.keys)?
            .into_iter()
            .next()
            .map(|recipient| recipient.enc_key)
            .ok_or_else(|| crypto::EncryptionError::NoRecipients.into())
    }

    /// Every device the store is currently encrypted to.
    pub fn recipients(&self) -> AppResult<Vec<Recipient>> {
        Ok(crypto::read_recipients(&self.paths.keys)?)
    }

    /// Join requests dropped into the shared `.age/` folder awaiting approval.
    pub fn pending_requests(&self) -> AppResult<Vec<PendingRequest>> {
        Ok(crypto::read_pending(&self.paths.keys)?)
    }

    /// This device's stored identity label and passphrase state, or `None` if no
    /// identity has been generated here yet.
    pub fn this_device(&self) -> AppResult<Option<DeviceIdentityInfo>> {
        Ok(crypto::device_identity_info(&self.paths.keys)?)
    }

    /// Whether unlocking this device's identity requires a passphrase. `false`
    /// for a plaintext identity (or when no identity exists), so the caller can
    /// skip the unlock prompt and auto-load.
    pub fn identity_needs_passphrase(&self) -> AppResult<bool> {
        Ok(crypto::device_identity_info(&self.paths.keys)?
            .is_some_and(|info| info.passphrase_protected))
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
        Ok(crypto::initialize_store_identity(&self.paths.keys, device_name, passphrase)?.enc_key)
    }

    /// Create this device's identity, write the initial roster, and encrypt the
    /// store as one recoverable operation. On any failure the journal root and
    /// local key/trust files are restored to their pre-enable state.
    pub fn enable_encryption(
        &self,
        device_name: &str,
        passphrase: Option<&SecretString>,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<EnableEncryptionSummary> {
        if self.encryption_enabled() {
            return Err(crypto::EncryptionError::RosterExists.into());
        }
        if migrate::store_has_encrypted_entry_files(self)? {
            return Err(EncryptionError::RecipientsMissing {
                path: self.paths.keys.devices_file.clone(),
            }
            .into());
        }

        let root_backup = migrate::backup_store(&self.paths.journal_root)?;
        let identity_backup = read_optional_file(&self.paths.keys.identity_file)?;
        let trust_backup = read_optional_file(&self.paths.keys.trust_file)?;

        let result = (|| -> AppResult<EnableEncryptionSummary> {
            let recipient = self.initialize_encryption(device_name, passphrase)?;
            let summary = migrate::encrypt_store_without_backup(self, &mut progress)?;
            Ok(EnableEncryptionSummary {
                recipient,
                migrated_files: summary.migrated_files,
            })
        })();

        match result {
            Ok(summary) => {
                fs::remove_dir_all(&root_backup)?;
                Ok(summary)
            }
            Err(error) => {
                if let Err(restore_error) =
                    migrate::restore_store(&self.paths.journal_root, &root_backup)
                {
                    bail!(
                        "{error}; ALSO failed to roll back the journal root: {restore_error}. \
                         A backup of the pre-encryption store remains at {}",
                        root_backup.display()
                    );
                }
                if let Err(restore_error) = self
                    .restore_enable_local_state(identity_backup.as_deref(), trust_backup.as_deref())
                {
                    bail!(
                        "{error}; the journal root was restored, but local encryption state could not be restored: {restore_error}"
                    );
                }
                Err(anyhow::anyhow!(
                    "{error}; encryption failed and the store was restored unchanged"
                ))
            }
        }
    }

    /// Generate this device's identity for a store that already exists elsewhere
    /// and drop a join request into the shared folder for another device to
    /// approve. Returns this device's [`Recipient`].
    pub fn request_access(
        &self,
        device_name: &str,
        passphrase: Option<&SecretString>,
    ) -> AppResult<Recipient> {
        Ok(crypto::request_store_access(
            &self.paths.keys,
            device_name,
            passphrase,
        )?)
    }

    /// Whether this device's unlocked identity is one of the store's current
    /// recipients — a device that can already decrypt, and so may re-encrypt the
    /// store to approve or remove others. `false` when locked or not yet approved.
    pub fn is_current_recipient(&self) -> AppResult<bool> {
        match &self.identity {
            Some(identity) => Ok(crypto::identity_is_recipient(&self.paths.keys, identity)?),
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
            .any(|request| request.recipient.enc_key == own_key))
    }

    /// Resolve whether this device can open the store, after its identity has
    /// been unlocked (or found not to need unlocking). On an encrypted store this
    /// device isn't a recipient of, a now-dead (revoked) key is retired aside as
    /// a side effect so the user only has to re-enroll — reported via
    /// [`StoreAccess::NeedsEnroll`]'s `retired_key`.
    pub fn resolve_access(&self) -> AppResult<StoreAccess> {
        if !self.encryption_enabled() || self.is_current_recipient()? {
            return Ok(StoreAccess::Ready);
        }
        let device_name = self
            .this_device()?
            .map(|device| device.name)
            .unwrap_or_default();
        if self.self_request_pending()? {
            return Ok(StoreAccess::AwaitingApproval { device_name });
        }
        let retired_key = self.retire_revoked_identity()?.is_some();
        Ok(StoreAccess::NeedsEnroll {
            device_name,
            retired_key,
        })
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
            crypto::add_recipient(&self.paths.keys, identity, &recipient)?;
            migrate::reencrypt_store(self, identity, &mut progress)
        })?;
        crypto::advance_trust_pins(&self.paths.keys)?;
        Ok(summary)
    }

    /// Revoke the recipient named `name` and re-encrypt every entry to exclude
    /// it. Revocation is forward-only — entries the revoked device already synced
    /// stay readable to it. Requires an unlocked identity.
    pub fn revoke_recipient(
        &self,
        name: &str,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<MigrationSummary> {
        let identity = self.require_reencrypt_identity("revoke-recipient")?;
        // Match on the key, not the name: a device that was renamed still stores
        // its old name locally, so a name check could be sidestepped by renaming
        // first and would then re-encrypt this device out of its own history.
        let own_key = identity.public_key();
        if self
            .recipients()?
            .iter()
            .any(|recipient| recipient.name == name && recipient.enc_key == own_key)
        {
            bail!("refusing to revoke this device's own recipient");
        }
        let summary = migrate::atomic(self, || {
            crypto::revoke_recipient(&self.paths.keys, identity, name)?;
            migrate::reencrypt_store(self, identity, &mut progress)
        })?;
        crypto::advance_trust_pins(&self.paths.keys)?;
        Ok(summary)
    }

    /// Relabel a recipient by appending a signed `rename` op. No re-encryption
    /// needed, but it must be signed, so it requires an unlocked recipient
    /// identity — an unsigned relabel would be a tamper vector.
    pub fn rename_recipient(&self, old: &str, new: &str) -> AppResult<()> {
        let identity = self.require_reencrypt_identity("rename-recipient")?;
        crypto::rename_recipient(&self.paths.keys, identity, old, new)?;
        crypto::advance_trust_pins(&self.paths.keys)?;
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
        Ok(crypto::set_identity_passphrase(
            &self.paths.keys,
            current,
            new,
        )?)
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

        let identity_backup = crypto::read_identity_file_bytes(&self.paths.keys)?;
        let trust_backup = fs::read(&self.paths.keys.trust_file).ok();
        let backup = migrate::backup_store(&root)?;

        let result = (|| -> AppResult<MigrationSummary> {
            let (recipient, new_identity) = crypto::rotate_add_new_key(&self.paths.keys, &old)?;
            let first = migrate::reencrypt_store(self, &old, &mut progress)?;

            crypto::commit_rotated_identity(
                &self.paths.keys,
                &recipient,
                &new_identity,
                passphrase,
            )?;
            self.identity = Some(new_identity);

            // Retire the old key, signed by the freshly rotated key (now trusted).
            let identity = self.identity.as_ref().expect("identity set above");
            crypto::drop_old_recipient(&self.paths.keys, identity, &old_key)?;
            let second = migrate::reencrypt_store(self, identity, &mut progress)?;

            Ok(MigrationSummary {
                migrated_files: first.migrated_files + second.migrated_files,
            })
        })();

        match result {
            Ok(summary) => {
                fs::remove_dir_all(&backup)?;
                crypto::advance_trust_pins(&self.paths.keys)?;
                Ok(summary)
            }
            Err(error) => {
                migrate::restore_store(&root, &backup)?;
                crypto::restore_identity_file(&self.paths.keys, &identity_backup)?;
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
            Some(bytes) => Ok(crypto::atomic_write(&self.paths.keys.trust_file, bytes)?),
            None => {
                if self.paths.keys.trust_file.exists() {
                    fs::remove_file(&self.paths.keys.trust_file)?;
                }
                Ok(())
            }
        }
    }

    fn restore_enable_local_state(
        &self,
        identity_bytes: Option<&[u8]>,
        trust_bytes: Option<&[u8]>,
    ) -> AppResult<()> {
        match identity_bytes {
            Some(bytes) => crypto::restore_identity_file(&self.paths.keys, bytes)?,
            None if self.paths.keys.identity_file.exists() => {
                fs::remove_file(&self.paths.keys.identity_file)?;
            }
            None => {}
        }
        self.restore_trust_file(trust_bytes)
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
            .any(|recipient| recipient.enc_key == request.recipient.enc_key)
        {
            crypto::remove_pending(&self.paths.keys, &request.id)?;
            return Ok(MigrationSummary { migrated_files: 0 });
        }
        let summary = migrate::atomic(self, || {
            crypto::add_recipient(&self.paths.keys, identity, &request.recipient)?;
            let summary = migrate::reencrypt_store(self, identity, &mut progress)?;
            crypto::remove_pending(&self.paths.keys, &request.id)?;
            Ok(summary)
        })?;
        crypto::advance_trust_pins(&self.paths.keys)?;
        Ok(summary)
    }

    /// Reject a pending join request without granting access.
    pub fn deny_pending(&self, request: &PendingRequest) -> AppResult<()> {
        Ok(crypto::remove_pending(&self.paths.keys, &request.id)?)
    }

    fn require_identity(&self, context: &'static str) -> AppResult<&crypto::UnlockedIdentity> {
        self.identity
            .as_ref()
            .ok_or_else(|| EncryptionError::Locked { context }.into())
    }

    /// The unlocked identity, but only if it's a current store recipient — the
    /// precondition for re-encrypting. A device awaiting approval can't grant
    /// history it can't itself read, so this fails with a clear message.
    fn require_reencrypt_identity(
        &self,
        context: &'static str,
    ) -> AppResult<&crypto::UnlockedIdentity> {
        let identity = self.require_identity(context)?;
        if !crypto::identity_is_recipient(&self.paths.keys, identity)? {
            bail!(
                "this device's key is not a current store recipient, so it cannot re-encrypt; approve it from a device that can already read this journal"
            );
        }
        Ok(identity)
    }

    /// Load the age identity into this store so encrypted entries can be read
    /// and written. Pass `Some(passphrase)` for a passphrase-protected identity
    /// and `None` for a plaintext one. After this succeeds, the store
    /// transparently handles both plaintext and encrypted entries.
    pub fn unlock(&mut self, passphrase: Option<&SecretString>) -> AppResult<()> {
        self.identity = Some(crypto::unlock_identity(&self.paths.keys, passphrase)?);
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

    /// Archive or unarchive a journal by renaming its directory. Returns the
    /// journal in its new state.
    pub fn set_journal_archived(&self, name: &str, archived: bool) -> AppResult<Journal> {
        storage::set_journal_archived(&self.paths.journal_root, name, archived)
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

    pub fn scan_import_sources(&self) -> AppResult<Vec<ImportSource>> {
        storage::scan_import_sources(&self.paths.journal_root, self.identity.as_ref())
    }

    pub fn read_entry(&self, journal: &str, path: &Path) -> AppResult<Entry> {
        storage::read_entry(journal, path, self.identity.as_ref())
    }

    pub fn read_entry_content(&self, path: &Path) -> AppResult<String> {
        storage::read_entry_content(path, self.identity.as_ref())
    }

    /// Read any store file as raw bytes, decrypting only when the caller says the
    /// backing file is encrypted.
    pub fn read_store_file(&self, path: &Path, encoding: StoreFileEncoding) -> AppResult<Vec<u8>> {
        match encoding {
            StoreFileEncoding::Plain => Ok(fs::read(path)?),
            StoreFileEncoding::Encrypted => {
                let identity = self
                    .identity
                    .as_ref()
                    .ok_or(crate::EncryptionError::Locked { context: "file" })?;
                Ok(crypto::decrypt_file_bytes(identity, path)?.copy_to_vec())
            }
        }
    }

    /// Write any store file atomically, encrypting only when the caller says the
    /// backing file is encrypted.
    pub fn write_store_file(
        &self,
        path: &Path,
        encoding: StoreFileEncoding,
        bytes: &[u8],
    ) -> AppResult<()> {
        match encoding {
            StoreFileEncoding::Plain => crypto::atomic_write(path, bytes)?,
            StoreFileEncoding::Encrypted => {
                let plaintext = crypto::PlaintextBytes::copy_from_slice(bytes);
                let ciphertext = crypto::encrypt_new_entry(
                    &self.paths.keys,
                    &plaintext,
                    self.identity.as_ref(),
                )?;
                crypto::atomic_write(path, ciphertext.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Whether files created in this store are encrypted on disk (i.e. a
    /// recipients roster exists). The FUSE mount uses this to decide whether a
    /// file created through the mount gets the `.age` suffix.
    pub fn encrypts_new_files(&self) -> bool {
        self.paths.keys.has_roster()
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
    /// creation/modification dates and recording an `[import]` provenance
    /// marker in the front matter. Encryption follows the store's setting, like
    /// [`create_entry_with_body`].
    pub fn create_imported_entry(&self, draft: ImportedEntryDraft<'_>) -> AppResult<PathBuf> {
        storage::create_imported_entry(&self.entry_codec(), &self.paths.journal_root, draft)
    }

    /// Add `secs` to an entry's accumulated `[datetime].writing_seconds` without
    /// touching `edited_at`. Used to record editor-open time after a real edit.
    pub fn add_writing_seconds(&self, path: &Path, secs: u64) -> AppResult<()> {
        if secs == 0 {
            return Ok(());
        }
        let codec = self.entry_codec();
        let content = codec.read(path)?;
        let Some(updated) = markdown::add_writing_seconds(&content, secs) else {
            return Ok(());
        };
        codec.write_existing(path, &updated)
    }

    /// Replace an existing entry's body, preserving its front matter. An empty
    /// body deletes the entry when `remove_if_empty` is set; an unchanged body is
    /// a no-op. The outcome lets callers record editing time only on a real edit.
    pub fn set_entry_body(
        &self,
        path: &Path,
        remove_if_empty: bool,
        new_body: &str,
    ) -> AppResult<EditOutcome> {
        storage::set_entry_body(&self.entry_codec(), path, remove_if_empty, new_body)
    }

    /// The codec for reading and writing this store's entry files, carrying the
    /// recipients/identity and whether new entries are encrypted.
    fn entry_codec(&self) -> storage::EntryCodec<'_> {
        storage::EntryCodec::new(self.paths.keys.clone(), self.identity.as_ref())
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

    /// Whether the entry at `path` already has a `[weather]` table. Lets the
    /// editor-save trigger fetch weather only when it's missing, so imported
    /// (e.g. Day One) weather is never clobbered. Reads and decrypts the file.
    pub fn entry_has_weather(&self, path: &Path) -> AppResult<bool> {
        let codec = self.entry_codec();
        let content = codec.read(path)?;
        let (front_matter, _) = markdown::split_front_matter(&content);
        Ok(front_matter
            .map(markdown::front_matter_fields)
            .and_then(|fm| fm.weather)
            .is_some())
    }

    /// Replace one metadata field of an entry's front matter (and refresh
    /// `edited_at`), leaving the body untouched. A no-op if the file has no
    /// front matter.
    pub fn set_entry_metadata_field(&self, path: &Path, field: MetadataField) -> AppResult<()> {
        self.set_entry_metadata_fields(path, &[field])
    }

    /// Replace several metadata fields in one file rewrite, applying them in
    /// order and refreshing `edited_at` once. Preferred when fields land together
    /// (e.g. weather + air quality) so the entry is read, re-rendered, and
    /// re-encrypted a single time. A no-op if `fields` is empty or the file has no
    /// front matter.
    pub fn set_entry_metadata_fields(
        &self,
        path: &Path,
        fields: &[MetadataField],
    ) -> AppResult<()> {
        if fields.is_empty() {
            return Ok(());
        }
        let codec = self.entry_codec();
        let content = codec.read(path)?;
        let Some(new_content) = markdown::with_metadata_fields(&content, fields) else {
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
                .ok_or(EncryptionError::Locked { context: "asset" })?;
            Ok(Some(
                crypto::decrypt_file_bytes(identity, &path)?.copy_to_vec(),
            ))
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
            .ok_or(EncryptionError::Locked { context: "store" })?;
        migrate::decrypt_store(self, identity, &mut progress)
    }

    pub fn encrypt_store(
        &self,
        mut progress: impl FnMut(usize, usize),
    ) -> AppResult<migrate::MigrationSummary> {
        if !self.encryption_enabled() && migrate::store_has_encrypted_entry_files(self)? {
            return Err(EncryptionError::RecipientsMissing {
                path: self.paths.keys.devices_file.clone(),
            }
            .into());
        }
        migrate::encrypt_store(self, &mut progress)
    }
}

fn read_optional_file(path: &Path) -> AppResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
