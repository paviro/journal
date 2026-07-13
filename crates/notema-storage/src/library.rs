use crate::{Entry, Journal};
use serde::{Deserialize, Serialize};
use std::{fs::Metadata, path::PathBuf, time::Duration};

/// How a library load may use or update the local derived cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CachePolicy {
    Off,
    #[default]
    Normal,
    Rebuild,
}

/// Progress while reconciling the on-disk journal tree with the local cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryLoadProgress {
    Discovering { entries_found: usize },
    Reading { current: usize, total: usize },
}

/// A complete, internally consistent journal-library view.
#[derive(Debug, Clone, PartialEq)]
pub struct LibrarySnapshot {
    pub journals: Vec<Journal>,
    pub entries: Vec<Entry>,
    pub report: LibraryLoadReport,
}

/// A read-only inventory of the source tree that can be validated after the
/// caller has accepted the selected folder.
pub struct LibraryDiscovery {
    pub(crate) journals: Vec<Journal>,
    pub(crate) entries: Vec<DiscoveredEntry>,
    pub(crate) elapsed: Duration,
}

impl LibraryDiscovery {
    pub fn journal_names(&self) -> impl Iterator<Item = &str> {
        self.journals.iter().map(|journal| journal.name.as_str())
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Privacy-safe diagnostics for cache and source loading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryLoadReport {
    pub total: Duration,
    pub discovery: Duration,
    pub cache_read: Duration,
    pub source_read: Duration,
    pub cache_write: Duration,
    pub entries: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub removed_records: usize,
    pub cache_status: CacheStatus,
    pub cache_warning: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CacheStatus {
    Hit,
    #[default]
    Missing,
    Disabled,
    Locked,
    Corrupt,
    Incompatible,
    Rebuilt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileStamp {
    len: u64,
    modified: Option<(u64, u32)>,
    #[cfg(unix)]
    changed: (i64, i64),
}

impl FileStamp {
    pub(crate) fn from_metadata(metadata: &Metadata) -> Self {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| (duration.as_secs(), duration.subsec_nanos()));
        #[cfg(unix)]
        let changed = {
            use std::os::unix::fs::MetadataExt;
            (metadata.ctime(), metadata.ctime_nsec())
        };
        Self {
            len: metadata.len(),
            modified,
            #[cfg(unix)]
            changed,
        }
    }
}

/// Opaque version of an entry file captured alongside an authoritative read.
/// Pass it back when saving to avoid overwriting a file changed by another
/// process while the editor was open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryRevision(FileStamp);

impl EntryRevision {
    pub(crate) fn read(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self(FileStamp::from_metadata(&std::fs::metadata(path)?)))
    }
}

pub(crate) struct DiscoveredEntry {
    pub source: notema_domain::EntryPath,
    pub stamp: FileStamp,
}

pub(crate) struct CachedRecord {
    pub stamp: FileStamp,
    pub entry: Entry,
}

/// A decoded cache kept opaque so callers cannot confuse it with validated data.
pub struct CachedLibrary {
    pub(crate) journals: Vec<Journal>,
    pub(crate) records: Vec<CachedRecord>,
    pub(crate) warning: Option<String>,
}

/// Result of probing the cache without touching the journal source tree.
pub struct CacheRead {
    pub cached: Option<CachedLibrary>,
    pub report: LibraryLoadReport,
}

impl CachedLibrary {
    /// Clone the cached view for immediate read-only display while this value is
    /// retained as the validation seed.
    pub fn snapshot(&self) -> LibrarySnapshot {
        LibrarySnapshot {
            journals: self.journals.clone(),
            entries: self
                .records
                .iter()
                .map(|record| record.entry.clone())
                .collect(),
            report: LibraryLoadReport {
                entries: self.records.len(),
                cache_hits: self.records.len(),
                cache_status: CacheStatus::Hit,
                cache_warning: self.warning.clone(),
                ..LibraryLoadReport::default()
            },
        }
    }
}

pub(crate) fn path_for_record(record: &CachedRecord) -> PathBuf {
    record.entry.path.clone()
}
