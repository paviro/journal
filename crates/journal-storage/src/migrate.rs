use crate::{AppResult, JournalStore, crypto, storage};
use chrono::Local;
use nanoid::nanoid;
use std::{
    fs,
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
}

enum MigrationMode<'a> {
    Encrypt {
        paths: &'a crypto::EncryptionPaths,
    },
    Decrypt {
        identity: &'a crypto::UnlockedIdentity,
    },
}

pub fn encrypt_store(store: &JournalStore) -> AppResult<MigrationSummary> {
    let paths = store.encryption_paths();
    let migrated_files = migrate_store(
        store.paths().journal_root.as_path(),
        MigrationMode::Encrypt { paths: &paths },
    )?
    .migrated_files;
    Ok(MigrationSummary { migrated_files })
}

pub fn decrypt_store(
    store: &JournalStore,
    identity: &crypto::UnlockedIdentity,
) -> AppResult<DecryptSummary> {
    let paths = store.encryption_paths();
    let migration = migrate_store(
        store.paths().journal_root.as_path(),
        MigrationMode::Decrypt { identity },
    )?;
    if paths.recipients_file.exists() {
        fs::remove_file(&paths.recipients_file)?;
    }
    let disabled_identity_file = disable_identity_file(&paths)?;
    Ok(DecryptSummary {
        migrated_files: migration.migrated_files,
        backup_path: migration.backup_path,
        disabled_identity_file,
    })
}

pub fn store_has_encrypted_entry_files(store: &JournalStore) -> AppResult<bool> {
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

fn migrate_store(root: &Path, mode: MigrationMode<'_>) -> AppResult<MigrationResult> {
    let files = migration_files(root, &mode)?;
    if files.is_empty() {
        return Ok(MigrationResult {
            migrated_files: 0,
            backup_path: None,
        });
    }
    ensure_no_migration_collisions(&files, &mode)?;
    let backup = backup_store(root)?;

    let result = (|| -> AppResult<()> {
        for source in &files {
            match mode {
                MigrationMode::Encrypt { paths } => encrypt_plain_entry(source, paths)?,
                MigrationMode::Decrypt { identity } => decrypt_encrypted_entry(source, identity)?,
            }
        }
        Ok(())
    })();

    if let Err(error) = result {
        return Err(format!(
            "migration failed; plaintext backup remains at {}: {error}",
            backup.display()
        )
        .into());
    }

    let backup_path = if matches!(mode, MigrationMode::Encrypt { .. }) {
        fs::remove_dir_all(&backup)?;
        None
    } else {
        Some(backup)
    };

    Ok(MigrationResult {
        migrated_files: files.len(),
        backup_path,
    })
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
            return Err(format!(
                "cannot migrate {}; target already exists: {}",
                source.display(),
                target.display()
            )
            .into());
        }
    }
    Ok(())
}

fn encrypt_plain_entry(path: &Path, paths: &crypto::EncryptionPaths) -> AppResult<()> {
    let target = path.with_extension("md.age");
    let temp = crate::sibling_temp_path(&target, "tmp.age");
    crypto::encrypt_file(paths, path, &temp)?;
    fs::rename(&temp, &target)?;
    fs::remove_file(path)?;
    Ok(())
}

fn decrypt_encrypted_entry(path: &Path, identity: &crypto::UnlockedIdentity) -> AppResult<()> {
    let target = decrypted_entry_path(path)?;
    let temp = crate::sibling_temp_path(&target, "tmp.md");
    crypto::decrypt_file(identity, path, &temp)?;
    let decrypted = fs::read_to_string(&temp)?;
    if decrypted.is_empty() {
        let _ = fs::remove_file(&temp);
        return Err(format!("decrypted entry is empty: {}", path.display()).into());
    }
    fs::rename(&temp, &target)?;
    fs::remove_file(path)?;
    Ok(())
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
        .ok_or("encrypted entry path has no UTF-8 file name")?;
    let plain_name = name
        .strip_suffix(".md.age")
        .ok_or("encrypted entry path does not end in .md.age")?;
    Ok(path.with_file_name(format!("{plain_name}.md")))
}

fn backup_store(root: &Path) -> AppResult<PathBuf> {
    let backup = backup_path(root);
    copy_dir_all(root, &backup)?;
    Ok(backup)
}

fn backup_path(root: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d%H%M%S%f");
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("journal");
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

fn disable_identity_file(paths: &crypto::EncryptionPaths) -> AppResult<PathBuf> {
    let target = disabled_identity_path(&paths.identity_file);
    fs::rename(&paths.identity_file, &target)?;
    Ok(target)
}

fn disabled_identity_path(identity_file: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d%H%M%S");
    disabled_identity_path_for_timestamp(identity_file, &timestamp.to_string())
}

fn disabled_identity_path_for_timestamp(identity_file: &Path, timestamp: &str) -> PathBuf {
    let parent = identity_file.parent().unwrap_or_else(|| Path::new(""));
    let base = parent.join(format!("identity.disabled-{timestamp}.age"));
    if !base.exists() {
        return base;
    }

    for _ in 0..32 {
        let candidate = parent.join(format!("identity.disabled-{timestamp}-{}.age", nanoid!(6)));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(format!(
        "identity.disabled-{timestamp}-{}.age",
        Local::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn disabled_identity_path_uses_timestamped_age_filename() {
        let dir = tempdir().unwrap();
        let identity = dir.path().join("identity.age");

        let disabled = disabled_identity_path_for_timestamp(&identity, "20260702123456");

        assert_eq!(
            disabled,
            dir.path().join("identity.disabled-20260702123456.age")
        );
    }
}
