use std::{
    fs,
    path::{Path, PathBuf},
};

mod crypto;
pub(crate) mod markdown;
mod migrate;
mod storage;

pub use journal_core::{
    AppResult, Entry, EntryEncryptionState, EntryMetadata, EntryPath, JournalResult, SearchHit,
    SearchScopeFilter, search_loaded_entries,
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

#[derive(Clone)]
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
        self.identity = Some(crypto::unlock_identity(
            &self.encryption_paths(),
            passphrase,
        )?);
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

    /// Create an entry from an external import, preserving its original
    /// creation/modification dates and recording an `import_id` provenance
    /// marker in the front matter. Encryption follows the store's setting, like
    /// [`create_entry_with_body`].
    #[allow(clippy::too_many_arguments)]
    pub fn create_imported_entry(
        &self,
        journal: &str,
        body: &str,
        metadata: EntryMetadata<'_>,
        created_at: chrono::DateTime<chrono::Local>,
        updated_at: chrono::DateTime<chrono::Local>,
        import_id: &str,
    ) -> JournalResult<PathBuf> {
        if self.encryption_enabled() {
            storage::create_encrypted_imported_entry_with_body_and_metadata(
                &self.paths.journal_root,
                journal,
                body,
                metadata,
                created_at,
                updated_at,
                import_id,
                &self.encryption_paths(),
            )
        } else {
            storage::create_imported_entry_with_body_and_metadata(
                &self.paths.journal_root,
                journal,
                body,
                metadata,
                created_at,
                updated_at,
                import_id,
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
        self.update_entry_metadata(path, |content| {
            markdown::set_tags_in_front_matter(content, tags)
        })
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
    ) -> JournalResult<storage::AssetReport> {
        let encrypted = storage::is_encrypted_entry_file(path);
        let content = if encrypted {
            let Some(identity) = self.identity.as_ref() else {
                return Ok(storage::AssetReport::default());
            };
            storage::read_entry_content_with_identity(path, Some(identity))?
        } else {
            fs::read_to_string(path)?
        };

        let (front_matter, body) = markdown::split_front_matter(&content);
        let body = body.trim_start_matches('\n');

        let paths = self.encryption_paths();
        let encryption = encrypted.then_some(&paths);
        let (new_body, report) = storage::ingest_and_cleanup_opts(
            path,
            body,
            encryption,
            download_remote,
            replace_offline,
        )?;

        if let Some(new_body) = new_body {
            let new_content = if let Some(fm) = front_matter {
                let reassembled =
                    format!("+++\n{fm}\n+++\n\n{}", new_body.trim_start_matches('\n'));
                markdown::set_updated_at_now_in_content(&reassembled)
            } else {
                new_body
            };
            if encrypted {
                storage::write_encrypted_entry_content(&paths, path, &new_content)?;
            } else {
                fs::write(path, &new_content)?;
            }
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
    ) -> JournalResult<Option<Vec<u8>>> {
        let Some(path) = storage::resolve_entry_asset_path(entry_path, file_name)? else {
            return Ok(None);
        };
        if path.extension().is_some_and(|ext| ext == "age") {
            let identity = self
                .identity
                .as_ref()
                .ok_or("encrypted asset requires an unlocked journal encryption identity")?;
            Ok(Some(crypto::decrypt_to_bytes(identity, &path)?))
        } else {
            Ok(Some(fs::read(path)?))
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
