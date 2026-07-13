use crate::{
    AppResult, JournalStorePaths,
    library::{
        CachePolicy, CacheRead, CacheStatus, CachedLibrary, CachedRecord, FileStamp,
        LibraryDiscovery, LibraryLoadReport, LibrarySnapshot, path_for_record,
    },
    storage,
};
use notema_domain::Entry;
use notema_encryption as crypto;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

const CACHE_WIRE_VERSION: u32 = 1;
const PLAIN_CACHE_FILE: &str = "library-cache.msgpack";
const ENCRYPTED_CACHE_FILE: &str = "library-cache.msgpack.age";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum CacheSecurity {
    Plaintext,
    Encrypted { recipients_sha256: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheFile {
    wire_version: u32,
    app_version: String,
    store_id: crate::StoreId,
    journal_root: PathBuf,
    security: CacheSecurity,
    journals: Vec<crate::Journal>,
    records: Vec<CacheRecordFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheRecordFile {
    stamp: FileStamp,
    entry: Entry,
}

pub(super) fn read(
    paths: &JournalStorePaths,
    identity: Option<&crypto::UnlockedIdentity>,
    policy: CachePolicy,
) -> AppResult<CacheRead> {
    let started = Instant::now();
    let mut report = LibraryLoadReport::default();
    if policy == CachePolicy::Off {
        report.cache_status = CacheStatus::Disabled;
        return Ok(CacheRead {
            cached: None,
            report,
        });
    }

    let encrypted = paths.keys.has_roster();
    if encrypted && identity.is_none() {
        report.cache_status = CacheStatus::Locked;
        return Ok(CacheRead {
            cached: None,
            report,
        });
    }
    let expected_security = security(paths)?;
    let path = cache_path(paths, encrypted);
    let bytes = match read_bytes(&path, identity, encrypted) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            report.cache_status = CacheStatus::Missing;
            report.cache_read = started.elapsed();
            return Ok(CacheRead {
                cached: None,
                report,
            });
        }
        Err(error) => {
            report.cache_status = CacheStatus::Corrupt;
            report.cache_warning = Some(format!("cache read failed: {error:#}"));
            report.cache_read = started.elapsed();
            return Ok(CacheRead {
                cached: None,
                report,
            });
        }
    };
    let cache: CacheFile = match rmp_serde::from_slice(bytes.as_ref()) {
        Ok(cache) => cache,
        Err(error) => {
            report.cache_status = CacheStatus::Corrupt;
            report.cache_warning = Some(format!("cache decode failed: {error}"));
            report.cache_read = started.elapsed();
            return Ok(CacheRead {
                cached: None,
                report,
            });
        }
    };
    let canonical_root = fs::canonicalize(&paths.journal_root)?;
    let store_id = crate::store_id::read(&paths.journal_root)?;
    let compatible = cache.wire_version == CACHE_WIRE_VERSION
        && cache.app_version == env!("CARGO_PKG_VERSION")
        && Some(cache.store_id.clone()) == store_id
        && cache.journal_root == canonical_root
        && cache.security == expected_security;
    if !compatible {
        report.cache_status = CacheStatus::Incompatible;
        report.cache_read = started.elapsed();
        return Ok(CacheRead {
            cached: None,
            report,
        });
    }

    report.cache_read = started.elapsed();
    report.cache_status = CacheStatus::Hit;
    report.entries = cache.records.len();
    report.cache_hits = cache.records.len();
    Ok(CacheRead {
        cached: Some(CachedLibrary {
            journals: cache.journals,
            records: cache
                .records
                .into_iter()
                .map(|record| CachedRecord {
                    stamp: record.stamp,
                    entry: record.entry,
                })
                .collect(),
            warning: report.cache_warning.clone(),
        }),
        report,
    })
}

pub(super) fn validate(
    paths: &JournalStorePaths,
    identity: Option<&crypto::UnlockedIdentity>,
    cached: Option<CachedLibrary>,
    policy: CachePolicy,
    progress: Option<&(dyn Fn(crate::LibraryLoadProgress) + Sync)>,
) -> AppResult<LibrarySnapshot> {
    let discovery = discover(paths, progress)?;
    validate_discovery(paths, identity, cached, policy, discovery, progress)
}

pub(super) fn discover(
    paths: &JournalStorePaths,
    progress: Option<&(dyn Fn(crate::LibraryLoadProgress) + Sync)>,
) -> AppResult<LibraryDiscovery> {
    let discovery_started = Instant::now();
    if let Some(progress) = progress {
        progress(crate::LibraryLoadProgress::Discovering { entries_found: 0 });
    }
    let journals = storage::discover_journals(&paths.journal_root)?;
    let report_discovery = |entries_found| {
        if let Some(progress) = progress {
            progress(crate::LibraryLoadProgress::Discovering { entries_found });
        }
    };
    let entries = storage::collect_discovered_entries_with_progress(
        &journals,
        progress.map(|_| &report_discovery as &(dyn Fn(usize) + Sync)),
    )?;
    Ok(LibraryDiscovery {
        journals,
        entries,
        elapsed: discovery_started.elapsed(),
    })
}

pub(super) fn validate_discovery(
    paths: &JournalStorePaths,
    identity: Option<&crypto::UnlockedIdentity>,
    cached: Option<CachedLibrary>,
    policy: CachePolicy,
    discovery: LibraryDiscovery,
    progress: Option<&(dyn Fn(crate::LibraryLoadProgress) + Sync)>,
) -> AppResult<LibrarySnapshot> {
    let validation_started = Instant::now();
    let LibraryDiscovery {
        mut journals,
        entries: discovered,
        elapsed: discovery,
    } = discovery;
    storage::initialize_journals(&mut journals);

    let had_cache = cached.is_some();
    let journals_changed = cached
        .as_ref()
        .is_none_or(|cache| cache.journals != journals);
    let cache_warning = cached.as_ref().and_then(|cache| cache.warning.clone());
    let mut records: HashMap<PathBuf, CachedRecord> = cached
        .into_iter()
        .flat_map(|cache| cache.records)
        .map(|record| (path_for_record(&record), record))
        .collect();
    let mut stamps = HashMap::with_capacity(discovered.len());
    let mut entries = Vec::with_capacity(discovered.len());
    let mut misses = Vec::new();
    for discovered in discovered {
        stamps.insert(discovered.source.path.clone(), discovered.stamp);
        match records.remove(&discovered.source.path) {
            Some(record)
                if policy == CachePolicy::Normal
                    && record.stamp == discovered.stamp
                    && record.entry.journal == discovered.source.journal =>
            {
                entries.push(record.entry);
            }
            _ => misses.push(discovered.source),
        }
    }
    let cache_hits = entries.len();
    let cache_misses = misses.len();
    let removed_records = records.len();
    if let Some(progress) = progress {
        progress(crate::LibraryLoadProgress::Reading {
            current: cache_hits,
            total: cache_hits + cache_misses,
        });
    }

    let source_started = Instant::now();
    let report_miss = |current, total| {
        if let Some(progress) = progress {
            progress(crate::LibraryLoadProgress::Reading {
                current: cache_hits + current,
                total: cache_hits + total,
            });
        }
    };
    entries.extend(storage::read_entries_with_progress(
        misses,
        identity,
        progress.map(|_| &report_miss as &(dyn Fn(usize, usize) + Sync)),
    )?);
    entries.sort_by(|left, right| right.path.cmp(&left.path));
    let source_read = source_started.elapsed();

    let mut report = LibraryLoadReport {
        discovery,
        source_read,
        entries: entries.len(),
        cache_hits,
        cache_misses,
        removed_records,
        cache_status: if had_cache && !journals_changed && cache_misses == 0 && removed_records == 0
        {
            CacheStatus::Hit
        } else {
            CacheStatus::Rebuilt
        },
        cache_warning,
        ..LibraryLoadReport::default()
    };

    let encrypted = paths.keys.has_roster();
    let should_save = policy != CachePolicy::Off
        && !(encrypted && identity.is_none())
        && (policy == CachePolicy::Rebuild
            || !had_cache
            || journals_changed
            || cache_misses > 0
            || removed_records > 0);
    if should_save {
        let write_started = Instant::now();
        if let Err(error) = save(paths, encrypted, &journals, &entries, &stamps) {
            report.cache_warning = Some(format!("cache save failed: {error:#}"));
        }
        report.cache_write = write_started.elapsed();
    }
    report.total = discovery.saturating_add(validation_started.elapsed());
    Ok(LibrarySnapshot {
        journals,
        entries,
        report,
    })
}

fn read_bytes(
    path: &Path,
    identity: Option<&crypto::UnlockedIdentity>,
    encrypted: bool,
) -> AppResult<Option<CacheBytes>> {
    if encrypted {
        if !path.exists() {
            return Ok(None);
        }
        return Ok(Some(CacheBytes::Secret(crypto::decrypt_file_bytes(
            identity.context("encrypted cache requires an unlocked identity")?,
            path,
        )?)));
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(CacheBytes::Plain(bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

enum CacheBytes {
    Plain(Vec<u8>),
    Secret(crypto::PlaintextBytes),
}

impl AsRef<[u8]> for CacheBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Plain(bytes) => bytes,
            Self::Secret(bytes) => bytes.as_bytes(),
        }
    }
}

fn save(
    paths: &JournalStorePaths,
    encrypted: bool,
    journals: &[crate::Journal],
    entries: &[Entry],
    stamps: &HashMap<PathBuf, FileStamp>,
) -> AppResult<()> {
    let records = entries
        .iter()
        .filter_map(|entry| {
            Some(CacheRecordFile {
                stamp: *stamps.get(&entry.path)?,
                entry: entry.clone(),
            })
        })
        .collect();
    let store_id =
        crate::store_id::read(&paths.journal_root)?.context("journal store marker is missing")?;
    let cache = CacheFile {
        wire_version: CACHE_WIRE_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        store_id,
        journal_root: fs::canonicalize(&paths.journal_root)?,
        security: security(paths)?,
        journals: journals.to_vec(),
        records,
    };
    let serialized = rmp_serde::to_vec_named(&cache)?;
    if encrypted {
        let plaintext = crypto::PlaintextBytes::from_vec(serialized);
        let ciphertext = crypto::encrypt_bytes(&paths.keys, &plaintext)?;
        crypto::atomic_write_private(&encrypted_path(paths), ciphertext.as_bytes())?;
    } else {
        crypto::atomic_write_private(&plain_path(paths), &serialized)?;
    }
    Ok(())
}

fn security(paths: &JournalStorePaths) -> AppResult<CacheSecurity> {
    if !paths.keys.has_roster() {
        return Ok(CacheSecurity::Plaintext);
    }
    let mut keys: Vec<String> = crypto::read_recipients(&paths.keys)?
        .into_iter()
        .map(|recipient| recipient.encryption_key)
        .collect();
    keys.sort();
    let mut digest = Sha256::new();
    for key in keys {
        let length = u32::try_from(key.len())?;
        digest.update(length.to_le_bytes());
        digest.update(key.as_bytes());
    }
    Ok(CacheSecurity::Encrypted {
        recipients_sha256: hex::encode(digest.finalize()),
    })
}

pub(super) fn remove_incompatible(paths: &JournalStorePaths, encrypted: bool) -> AppResult<()> {
    if encrypted {
        remove_if_exists(&plain_path(paths))?;
    } else {
        remove_if_exists(&encrypted_path(paths))?;
    }
    Ok(())
}

pub(super) fn invalidate(paths: &JournalStorePaths) -> AppResult<()> {
    for path in [plain_path(paths), encrypted_path(paths)] {
        remove_if_exists(&path)?;
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> AppResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn cache_path(paths: &JournalStorePaths, encrypted: bool) -> PathBuf {
    if encrypted {
        encrypted_path(paths)
    } else {
        plain_path(paths)
    }
}

fn plain_path(paths: &JournalStorePaths) -> PathBuf {
    paths.config_dir.join(PLAIN_CACHE_FILE)
}

fn encrypted_path(paths: &JournalStorePaths) -> PathBuf {
    paths.config_dir.join(ENCRYPTED_CACHE_FILE)
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntryAssetOptions, EntryDraft, JournalStore};
    use notema_domain::Metadata;
    use tempfile::tempdir;

    fn store_with_entries(root: &Path, config: &Path, bodies: &[&str]) -> JournalStore {
        let store = JournalStore::new(root, config);
        store.ensure().unwrap();
        store.create_journal("daily").unwrap();
        for body in bodies {
            store
                .create_entry(
                    EntryDraft::new("daily", body, &Metadata::default()),
                    EntryAssetOptions::default(),
                )
                .unwrap();
        }
        store
    }

    #[test]
    fn cached_snapshot_is_available_before_source_validation() {
        let dir = tempdir().unwrap();
        let store = store_with_entries(
            &dir.path().join("journals"),
            &dir.path().join("config"),
            &["first", "second"],
        );
        let first = store.load_library(CachePolicy::Normal).unwrap();
        assert_eq!(first.report.cache_misses, 2);

        let cached = store.read_cached_library(CachePolicy::Normal).unwrap();
        let snapshot = cached.cached.as_ref().unwrap().snapshot();
        assert_eq!(snapshot.entries.len(), 2);

        let validated = store
            .validate_library(cached.cached, CachePolicy::Normal)
            .unwrap();
        assert_eq!(validated.report.cache_hits, 2);
        assert_eq!(validated.report.cache_misses, 0);
    }

    #[test]
    fn rebuild_reports_entry_progress() {
        let dir = tempdir().unwrap();
        let store = store_with_entries(
            &dir.path().join("journals"),
            &dir.path().join("config"),
            &["first", "second"],
        );
        let updates = std::sync::Mutex::new(Vec::new());

        store
            .load_library_with_progress(CachePolicy::Rebuild, &|update| {
                updates.lock().unwrap().push(update);
            })
            .unwrap();

        let updates = updates.into_inner().unwrap();
        assert_eq!(
            updates.first(),
            Some(&crate::LibraryLoadProgress::Discovering { entries_found: 0 })
        );
        assert!(updates.contains(&crate::LibraryLoadProgress::Discovering { entries_found: 2 }));
        let reading = updates.iter().filter_map(|update| match update {
            crate::LibraryLoadProgress::Reading { current, total } => Some((*current, *total)),
            crate::LibraryLoadProgress::Discovering { .. } => None,
        });
        let reading = reading.collect::<Vec<_>>();
        assert!(reading.iter().all(|(_, total)| *total == 2));
        assert_eq!(reading.iter().map(|(current, _)| *current).max(), Some(2));
    }

    #[test]
    fn setup_discovery_is_read_only_and_reused_for_rebuild() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("journals");
        let journal = root.join("daily");
        let store = JournalStore::new(&root, dir.path().join("config"));
        store.ensure().unwrap();
        fs::create_dir_all(&journal).unwrap();
        fs::write(journal.join("entry.md"), "# First\n").unwrap();
        let sidecar = journal.join(".journal.toml");

        let discovery = store.discover_library_with_progress(&|_| {}).unwrap();
        assert_eq!(discovery.entry_count(), 1);
        assert_eq!(discovery.journal_names().collect::<Vec<_>>(), ["daily"]);
        assert!(!sidecar.exists());

        let snapshot = store
            .load_discovered_library_with_progress(CachePolicy::Rebuild, discovery, &|_| {})
            .unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert!(sidecar.exists());
    }

    #[test]
    fn validation_persists_journal_only_changes() {
        let dir = tempdir().unwrap();
        let store = store_with_entries(
            &dir.path().join("journals"),
            &dir.path().join("config"),
            &[],
        );
        store.load_library(CachePolicy::Normal).unwrap();
        let cached = store
            .read_cached_library(CachePolicy::Normal)
            .unwrap()
            .cached;
        store.create_journal("second").unwrap();

        let validated = store.validate_library(cached, CachePolicy::Normal).unwrap();
        assert_eq!(validated.journals.len(), 2);

        let persisted = store.read_cached_library(CachePolicy::Normal).unwrap();
        assert_eq!(persisted.cached.unwrap().snapshot().journals.len(), 2);
    }

    #[test]
    fn validation_reloads_changed_entries_and_drops_deleted_entries() {
        let dir = tempdir().unwrap();
        let store = store_with_entries(
            &dir.path().join("journals"),
            &dir.path().join("config"),
            &["first", "second"],
        );
        let initial = store.load_library(CachePolicy::Normal).unwrap();
        let changed_path = initial.entries[0].path.clone();
        let deleted_path = initial.entries[1].path.clone();
        let cached = store
            .read_cached_library(CachePolicy::Normal)
            .unwrap()
            .cached;
        fs::write(&changed_path, "changed body with a different length\n").unwrap();
        fs::remove_file(&deleted_path).unwrap();

        let validated = store.validate_library(cached, CachePolicy::Normal).unwrap();

        assert_eq!(validated.entries.len(), 1);
        assert_eq!(
            validated.entries[0].body,
            "changed body with a different length\n"
        );
        assert_eq!(validated.report.cache_hits, 0);
        assert_eq!(validated.report.cache_misses, 1);
        assert_eq!(validated.report.removed_records, 1);
    }

    #[test]
    fn app_version_or_store_mismatch_is_not_trusted() {
        let dir = tempdir().unwrap();
        let store = store_with_entries(
            &dir.path().join("journals"),
            &dir.path().join("config"),
            &["body"],
        );
        store.load_library(CachePolicy::Normal).unwrap();
        let path = plain_path(store.paths());
        let mut cache: CacheFile = rmp_serde::from_slice(&fs::read(&path).unwrap()).unwrap();
        cache.app_version = "other".to_owned();
        fs::write(&path, rmp_serde::to_vec_named(&cache).unwrap()).unwrap();

        let read = store.read_cached_library(CachePolicy::Normal).unwrap();
        assert!(read.cached.is_none());
        assert_eq!(read.report.cache_status, CacheStatus::Incompatible);
    }

    #[test]
    fn binary_cache_round_trips_non_finite_source_values() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("journals");
        let store = store_with_entries(&root, &dir.path().join("config"), &[]);
        let path = root.join("daily/2026-07-13-nonfinite.md");
        fs::write(
            &path,
            "+++\nschema_version = 1\n[weather]\ntemperature_celsius = nan\n+++\n\nBody\n",
        )
        .unwrap();

        store.load_library(CachePolicy::Normal).unwrap();
        let cached = store.read_cached_library(CachePolicy::Normal).unwrap();
        let temperature = cached.cached.unwrap().snapshot().entries[0]
            .weather
            .as_ref()
            .unwrap()
            .temperature_celsius
            .unwrap();
        assert!(temperature.is_nan());
    }

    #[cfg(unix)]
    #[test]
    fn plaintext_cache_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let store = store_with_entries(
            &dir.path().join("journals"),
            &dir.path().join("config"),
            &["body"],
        );
        store.load_library(CachePolicy::Normal).unwrap();
        let mode = fs::metadata(plain_path(store.paths()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
