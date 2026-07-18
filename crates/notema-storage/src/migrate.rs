use crate::{AppResult, JournalStore, storage};
use anyhow::{Context, bail};
use chrono::Local;
use notema_encryption::{self as crypto, KeyPaths};
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationSummary {
    pub migrated_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptSummary {
    pub migrated_files: usize,
    pub backup_path: Option<PathBuf>,
    pub disabled_identity_file: PathBuf,
    pub disabled_trust_file: Option<PathBuf>,
}

/// The local files a device retires when it notices encryption was disabled on
/// another device — the private key and roster pins it held while encrypted,
/// renamed aside rather than deleted. Returned by [`reconcile_disabled_encryption`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisabledElsewhereCleanup {
    pub disabled_identity_file: Option<PathBuf>,
    pub disabled_trust_file: Option<PathBuf>,
}

enum MigrationMode<'a> {
    Encrypt {
        recipients: &'a crypto::EncryptionRecipients,
    },
    Decrypt {
        identity: &'a crypto::UnlockedIdentity,
    },
}

/// Progress sink for a whole-store migration: called with `(done, total)` once
/// at the start (`0, total`) and after each file is converted.
pub(crate) type ProgressFn<'a> = &'a mut dyn FnMut(usize, usize);

pub(crate) fn encrypt_store(
    store: &JournalStore,
    progress: ProgressFn<'_>,
) -> AppResult<MigrationSummary> {
    let backup = backup_store(&store.paths().journal_root)?;
    match encrypt_store_without_backup(store, progress) {
        Ok(summary) => {
            fs::remove_dir_all(&backup)?;
            Ok(summary)
        }
        Err(error) => {
            if let Err(restore_error) = restore_store(&store.paths().journal_root, &backup) {
                bail!(
                    "{error}; ALSO failed to roll back the store: {restore_error}. \
                     A backup of the pre-encryption store remains at {}",
                    backup.display()
                );
            }
            Err(anyhow::anyhow!(
                "{error}; encryption failed and the store was restored unchanged"
            ))
        }
    }
}

pub(crate) fn encrypt_store_without_backup(
    store: &JournalStore,
    progress: ProgressFn<'_>,
) -> AppResult<MigrationSummary> {
    let paths = store.paths();
    let recipients = crypto::EncryptionRecipients::for_store(&paths.keys)?;
    let migrated_files = migrate_store_files(
        paths.journal_root.as_path(),
        MigrationMode::Encrypt {
            recipients: &recipients,
        },
        progress,
    )?;
    Ok(MigrationSummary { migrated_files })
}

pub(crate) fn decrypt_store(
    store: &JournalStore,
    identity: &crypto::UnlockedIdentity,
    progress: ProgressFn<'_>,
) -> AppResult<DecryptSummary> {
    let paths = store.paths();
    let migration = migrate_store(
        paths.journal_root.as_path(),
        MigrationMode::Decrypt { identity },
        progress,
    )?;
    clear_age_dir(&paths.keys)?;
    let disabled_trust_file = disable_trust_file(&paths.keys)?;
    let disabled_identity_file = disable_identity_file(&paths.keys)?;
    Ok(DecryptSummary {
        migrated_files: migration.migrated_files,
        backup_path: migration.backup_path,
        disabled_identity_file,
        disabled_trust_file,
    })
}

/// Notice an encryption *disable* that happened on another device and mirror it
/// locally. When that device turned encryption off it deleted the synced roster
/// (`devices.toml`) and decrypted every entry, but this device still holds the
/// `identity.toml` and `devices-trust.toml` it used while encrypted. Detect that —
/// a roster this device had pinned that is now gone — and retire the key and pins
/// by renaming them aside, exactly as a local [`decrypt_store`] does, so the
/// device drops back to plaintext instead of trying to unlock a store that no
/// longer exists. Returns `None` (no change) when there is nothing to reconcile.
///
/// Gated to fail safe:
/// - Requires the local trust pins to exist, so a freshly-enrolled device whose
///   synced `.age/` folder simply hasn't downloaded yet is never mistaken for a
///   disable — it has an identity but has never pinned a roster.
/// - Requires no encrypted entries to remain, so a half-synced store (roster gone
///   but entries still `.age`) keeps the key that can still read them until the
///   plaintext conversions finish syncing.
pub(crate) fn reconcile_disabled_encryption(
    store: &JournalStore,
) -> AppResult<Option<DisabledElsewhereCleanup>> {
    let paths = &store.paths().keys;
    if paths.devices_file.exists() || !paths.trust_file.exists() {
        return Ok(None);
    }
    if store_has_encrypted_entry_files(store)? {
        return Ok(None);
    }
    let disabled_trust_file = disable_trust_file(paths)?;
    let disabled_identity_file = if paths.identity_file.exists() {
        Some(disable_identity_file(paths)?)
    } else {
        None
    };
    Ok(Some(DisabledElsewhereCleanup {
        disabled_identity_file,
        disabled_trust_file,
    }))
}

/// Retire this device's now-dead private key after its store access was revoked
/// (denied, removed, or a request that never synced): the store is still
/// encrypted for other devices, so only `identity.toml` is renamed aside
/// (recoverable), letting a fresh `enroll` request access without the user
/// deleting the file by hand. The roster trust pins are deliberately kept — the
/// genesis is unchanged, so they still guard a re-enroll against a swapped or
/// rolled-back roster. Returns the renamed path, or `None` when no identity
/// exists here.
pub(crate) fn retire_revoked_identity(store: &JournalStore) -> AppResult<Option<PathBuf>> {
    let paths = &store.paths().keys;
    if !paths.identity_file.exists() {
        return Ok(None);
    }
    Ok(Some(disable_identity_file(paths)?))
}

/// Tear down the synced key folder when encryption is disabled: drop the signed
/// `devices.toml` roster and any leftover `pending-*.toml` join requests (which
/// would otherwise keep syncing and resurface as phantom approval modals), then
/// remove the `.age` folder itself if nothing else is left in it. The local trust
/// pins are not deleted here — the caller renames `devices-trust.toml` aside (like
/// the identity), keeping a recoverable copy; they are meaningless once the roster
/// is gone and would otherwise reject a freshly re-enabled store as a "changed
/// genesis".
fn clear_age_dir(paths: &KeyPaths) -> AppResult<()> {
    if paths.devices_file.exists() {
        fs::remove_file(&paths.devices_file)?;
    }
    if !paths.age_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&paths.age_dir)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("pending-") && name.ends_with(".toml"))
        {
            fs::remove_file(path)?;
        }
    }
    // Leaves the folder in place if the user dropped unrelated files in it.
    let _ = fs::remove_dir(&paths.age_dir);
    Ok(())
}

/// Re-encrypt every encrypted file (entries and their assets) to the store's
/// *current* recipient set. Runs after a recipient is added or removed so the
/// change reaches existing history, not just new entries. Requires an unlocked
/// identity that can decrypt the store as it stands now.
///
/// Converts every file or returns `Err` on the first failure, leaving the store
/// partially converted. Callers must run this inside [`atomic`] so such a
/// failure rolls the whole store back rather than stranding it mid-conversion.
pub(crate) fn reencrypt_store(
    store: &JournalStore,
    identity: &crypto::UnlockedIdentity,
    progress: ProgressFn<'_>,
) -> AppResult<MigrationSummary> {
    let paths = store.paths();
    let mut files = Vec::new();
    collect_store_files_including_trash(paths.journal_root.as_path(), &mut |path| {
        if path.extension() == Some(OsStr::new("age")) {
            files.push(path.to_path_buf());
        }
        Ok(())
    })?;
    files.sort();
    let recipients = crypto::EncryptionRecipients::for_store(&paths.keys)?;

    progress(0, files.len());
    for (done, path) in files.iter().enumerate() {
        reencrypt_file(path, &recipients, identity)?;
        progress(done + 1, files.len());
    }
    Ok(MigrationSummary {
        migrated_files: files.len(),
    })
}

fn reencrypt_file(
    path: &Path,
    recipients: &crypto::EncryptionRecipients,
    identity: &crypto::UnlockedIdentity,
) -> AppResult<()> {
    // Stream old ciphertext -> plaintext -> new ciphertext without buffering the
    // whole file. Safe to write back to the same path: the source is fully read and
    // re-encrypted into a sibling temp, which is only then renamed over `path`.
    let reader = crypto::decrypt_file_reader(identity, path)?;
    recipients.encrypt_reader_to_file(reader, path)?;
    Ok(())
}

pub(crate) fn store_has_encrypted_entry_files(store: &JournalStore) -> AppResult<bool> {
    let mut has_match = false;
    collect_store_files_including_trash(store.paths().journal_root.as_path(), &mut |path| {
        if storage::is_encrypted_entry_file(path) {
            has_match = true;
        }
        Ok(())
    })?;
    Ok(has_match)
}

struct MigrationResult {
    migrated_files: usize,
    backup_path: Option<PathBuf>,
}

fn migrate_store(
    root: &Path,
    mode: MigrationMode<'_>,
    progress: ProgressFn<'_>,
) -> AppResult<MigrationResult> {
    let encrypting = matches!(mode, MigrationMode::Encrypt { .. });
    let backup = backup_store(root)?;
    let result = migrate_store_files(root, mode, progress);

    let migrated_files = match result {
        Ok(migrated_files) => migrated_files,
        Err(error) => {
            bail!(
                "migration failed; plaintext backup remains at {}: {error}",
                backup.display()
            );
        }
    };

    let backup_path = if encrypting {
        fs::remove_dir_all(&backup)?;
        None
    } else {
        Some(backup)
    };

    Ok(MigrationResult {
        migrated_files,
        backup_path,
    })
}

fn migrate_store_files(
    root: &Path,
    mode: MigrationMode<'_>,
    progress: ProgressFn<'_>,
) -> AppResult<usize> {
    let entry_files = migration_files(root, &mode)?;
    let asset_files = migration_asset_files(root, &mode)?;
    let total = entry_files.len() + asset_files.len();
    if total == 0 {
        return Ok(0);
    }
    ensure_no_migration_collisions(&entry_files, &mode)?;
    ensure_no_asset_collisions(&asset_files, &mode)?;

    progress(0, total);
    let mut done = 0usize;
    for source in &entry_files {
        match &mode {
            MigrationMode::Encrypt { recipients } => encrypt_plain_entry(source, recipients)?,
            MigrationMode::Decrypt { identity } => decrypt_encrypted_entry(source, identity)?,
        }
        done += 1;
        progress(done, total);
    }
    // Assets carry the same `.age` suffix as entries but keep clean body
    // links, so converting them only renames files — no entry is rewritten.
    for source in &asset_files {
        convert_asset_file(source, &mode)?;
        done += 1;
        progress(done, total);
    }
    Ok(total)
}

/// Collect asset files (inside any `*.assets/` folder) that need converting:
/// plaintext files when encrypting, `.age` files when decrypting.
fn migration_asset_files(root: &Path, mode: &MigrationMode<'_>) -> AppResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_store_files_including_trash(root, &mut |path| {
        if is_in_assets_dir(path) && asset_matches_mode(path, mode) {
            files.push(path.to_path_buf());
        }
        Ok(())
    })?;
    files.sort();
    Ok(files)
}

fn is_in_assets_dir(path: &Path) -> bool {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".assets"))
}

fn asset_matches_mode(path: &Path, mode: &MigrationMode<'_>) -> bool {
    let is_encrypted = path.extension() == Some(OsStr::new("age"));
    match mode {
        MigrationMode::Encrypt { .. } => !is_encrypted,
        MigrationMode::Decrypt { .. } => is_encrypted,
    }
}

/// Encrypt (`<name>` → `<name>.age`) or decrypt (`<name>.age` → `<name>`) one
/// asset file in place, atomically via temp + rename.
fn convert_asset_file(path: &Path, mode: &MigrationMode<'_>) -> AppResult<()> {
    match mode {
        MigrationMode::Encrypt { recipients } => {
            let target = append_age(path);
            recipients.encrypt_reader_to_file(fs::File::open(path)?, &target)?;
            fs::remove_file(path)?;
        }
        MigrationMode::Decrypt { identity } => {
            let target = strip_age(path)?;
            let reader = crypto::decrypt_file_reader(identity, path)?;
            // This path intentionally writes plaintext to disk; streaming keeps
            // memory constant but the output is the decrypted file itself.
            stream_to_atomic_file(reader, &target)?;
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn append_age(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".age");
    path.with_file_name(name)
}

fn strip_age(path: &Path) -> AppResult<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("asset path has no UTF-8 file name")?;
    let base = name
        .strip_suffix(".age")
        .context("encrypted asset does not end in .age")?;
    Ok(path.with_file_name(base))
}

fn migration_files(root: &Path, mode: &MigrationMode<'_>) -> AppResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_store_files_including_trash(root, &mut |path| {
        let matches = match mode {
            MigrationMode::Encrypt { .. } => storage::is_plain_entry_file(path),
            MigrationMode::Decrypt { .. } => storage::is_encrypted_entry_file(path),
        };
        if matches {
            files.push(path.to_path_buf());
        }
        Ok(())
    })?;
    files.sort();
    Ok(files)
}

fn collect_store_files_including_trash(
    dir: &Path,
    visit: &mut impl FnMut(&Path) -> AppResult<()>,
) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_store_files_including_trash(&path, visit)?;
            continue;
        }
        visit(&path)?;
    }

    Ok(())
}

fn ensure_no_migration_collisions(files: &[PathBuf], mode: &MigrationMode<'_>) -> AppResult<()> {
    for source in files {
        let target = migration_target(source, mode)?;
        if target.exists() {
            bail!(
                "cannot migrate {}; target already exists: {}",
                source.display(),
                target.display()
            );
        }
    }
    Ok(())
}

/// Guard the asset conversions the same way [`ensure_no_migration_collisions`]
/// guards entries: refuse to run if converting an asset would clobber a file
/// that already exists (an inconsistent store holding both `x.png` and
/// `x.png.age`), since the conversion renames onto the target in place.
fn ensure_no_asset_collisions(files: &[PathBuf], mode: &MigrationMode<'_>) -> AppResult<()> {
    for source in files {
        let target = match mode {
            MigrationMode::Encrypt { .. } => append_age(source),
            MigrationMode::Decrypt { .. } => strip_age(source)?,
        };
        if target.exists() {
            bail!(
                "cannot migrate asset {}; target already exists: {}",
                source.display(),
                target.display()
            );
        }
    }
    Ok(())
}

fn encrypt_plain_entry(path: &Path, recipients: &crypto::EncryptionRecipients) -> AppResult<()> {
    let target = path.with_extension("md.age");
    recipients.encrypt_reader_to_file(fs::File::open(path)?, &target)?;
    fs::remove_file(path)?;
    Ok(())
}

fn decrypt_encrypted_entry(path: &Path, identity: &crypto::UnlockedIdentity) -> AppResult<()> {
    let target = decrypted_entry_path(path)?;
    let reader = crypto::decrypt_file_reader(identity, path)?;
    // Stream the plaintext straight to disk (decrypting the store intentionally
    // produces plaintext files). We can't cheaply re-validate the whole payload
    // as UTF-8 while streaming, so we keep only the emptiness guard via the byte
    // count; entry text is UTF-8-validated on read.
    let written = stream_to_atomic_file(reader, &target)?;
    if written == 0 {
        fs::remove_file(&target)?;
        bail!("decrypted entry is empty: {}", path.display());
    }
    fs::remove_file(path)?;
    Ok(())
}

/// Copy `reader` into `path` via an atomic temp+rename, returning the number of
/// bytes written. Used for the decrypt-migration paths, which produce plaintext
/// files on disk by design.
fn stream_to_atomic_file<R: io::Read>(mut reader: R, path: &Path) -> AppResult<u64> {
    let mut written = 0u64;
    crypto::atomic_write_with(path, false, |file| {
        written = io::copy(&mut reader, file)?;
        Ok(())
    })?;
    Ok(written)
}

fn migration_target(path: &Path, mode: &MigrationMode<'_>) -> AppResult<PathBuf> {
    match mode {
        MigrationMode::Encrypt { .. } => Ok(path.with_extension("md.age")),
        MigrationMode::Decrypt { .. } => decrypted_entry_path(path),
    }
}

fn decrypted_entry_path(path: &Path) -> AppResult<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("encrypted entry path has no UTF-8 file name")?;
    let plain_name = name
        .strip_suffix(".md.age")
        .context("encrypted entry path does not end in .md.age")?;
    Ok(path.with_file_name(format!("{plain_name}.md")))
}

/// Run `op` as an all-or-nothing change to the store: snapshot the whole
/// journal root first, and on any error roll every file (entries, assets, and
/// the `devices.toml` roster) back to the snapshot so a failed key change leaves
/// no trace. The snapshot is deleted on success. Key-changing operations must run
/// their roster mutation *and* [`reencrypt_store`] inside this so the two can't
/// diverge. (The local trust pins live outside the root; callers advance them
/// only after this returns `Ok`.)
pub(crate) fn atomic<T>(store: &JournalStore, op: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    let root = store.paths().journal_root.clone();
    let backup = backup_store(&root)?;
    match op() {
        Ok(value) => {
            fs::remove_dir_all(&backup)?;
            Ok(value)
        }
        Err(error) => {
            if let Err(restore_error) = restore_store(&root, &backup) {
                bail!(
                    "{error}; ALSO failed to roll back the store: {restore_error}. \
                     A backup of the pre-change store remains at {}",
                    backup.display()
                );
            }
            Err(error)
        }
    }
}

/// Replace `root` with `backup` wholesale: drop the (partially changed) root and
/// move the snapshot into its place. A single rename, so no half-converted files
/// or leftover temps survive.
pub(crate) fn restore_store(root: &Path, backup: &Path) -> AppResult<()> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    fs::rename(backup, root)?;
    Ok(())
}

pub(crate) fn backup_store(root: &Path) -> AppResult<PathBuf> {
    let backup = backup_path(root);
    copy_dir_all(root, &backup)?;
    Ok(backup)
}

fn backup_path(root: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d%H%M%S%f");
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("notema");
    root.with_file_name(format!("{name}.backup-{timestamp}"))
}

fn copy_dir_all(source: &Path, target: &Path) -> AppResult<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

/// Retire this device's private key when encryption is turned off: rename
/// `identity.toml` aside as `identity.disabled-<timestamp>.toml` — a recoverable
/// copy, not a delete. Returns the new path.
fn disable_identity_file(paths: &KeyPaths) -> AppResult<PathBuf> {
    rename_aside(&paths.identity_file, "identity", "toml")
}

/// Retire this device's roster trust pins the same way as its key, renaming
/// `devices-trust.toml` aside rather than deleting it. Returns the new path, or
/// `None` when there were no pins on this device to retire.
fn disable_trust_file(paths: &KeyPaths) -> AppResult<Option<PathBuf>> {
    if !paths.trust_file.exists() {
        return Ok(None);
    }
    Ok(Some(rename_aside(
        &paths.trust_file,
        "devices-trust",
        "toml",
    )?))
}

/// Rename `path` aside as `<stem>.disabled-<timestamp>.<ext>` next to it,
/// returning the new path. Shared by the key and trust-pin retirement so both
/// leave a recoverable, uniformly-named copy when encryption is disabled.
fn rename_aside(path: &Path, stem: &str, ext: &str) -> AppResult<PathBuf> {
    let target = disabled_path(path, stem, ext);
    fs::rename(path, &target)?;
    Ok(target)
}

fn disabled_path(path: &Path, stem: &str, ext: &str) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d%H%M%S");
    disabled_path_for_timestamp(path, stem, ext, &timestamp.to_string())
}

fn disabled_path_for_timestamp(path: &Path, stem: &str, ext: &str, timestamp: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let base = parent.join(format!("{stem}.disabled-{timestamp}.{ext}"));
    if !base.exists() {
        return base;
    }

    for _ in 0..32 {
        let candidate = parent.join(format!(
            "{stem}.disabled-{timestamp}-{}.{ext}",
            storage::random_id(6)
        ));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(format!(
        "{stem}.disabled-{timestamp}-{}.{ext}",
        Local::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn disabled_path_uses_timestamped_filename() {
        let dir = tempdir().unwrap();
        let identity = dir.path().join("identity.toml");

        let disabled = disabled_path_for_timestamp(&identity, "identity", "toml", "20260702123456");

        assert_eq!(
            disabled,
            dir.path().join("identity.disabled-20260702123456.toml")
        );
    }

    #[test]
    fn disabled_path_reuses_stem_and_extension_for_trust_pins() {
        let dir = tempdir().unwrap();
        let trust = dir.path().join("devices-trust.toml");

        let disabled =
            disabled_path_for_timestamp(&trust, "devices-trust", "toml", "20260702123456");

        assert_eq!(
            disabled,
            dir.path()
                .join("devices-trust.disabled-20260702123456.toml")
        );
    }
}
