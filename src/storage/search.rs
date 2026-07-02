use std::path::PathBuf;

use super::{Entry, EntryEncryptionState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub journal: String,
    pub title: String,
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScopeFilter<'a> {
    AllJournals,
    Journal(&'a str),
}

/// Filter already-loaded entries in memory. No disk I/O or decryption — the
/// caller's `Entry` cache already holds decrypted `content` for every entry.
pub fn search_loaded_entries(
    entries: &[Entry],
    query: &str,
    scope: SearchScopeFilter<'_>,
) -> Vec<SearchHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut hits = Vec::new();
    for entry in entries {
        if matches!(scope, SearchScopeFilter::Journal(journal) if entry.journal != journal) {
            continue;
        }
        if entry.encryption_state == EntryEncryptionState::EncryptedLocked {
            continue;
        }

        if entry.content.to_lowercase().contains(&needle) {
            hits.push(SearchHit {
                path: entry.path.clone(),
                journal: entry.journal.clone(),
                title: entry.title.clone(),
                preview: entry.preview.clone(),
            });
        }
    }

    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::scan_entries;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn search_matches_entries() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work").join("2026-07-01")).unwrap();
        fs::write(
            dir.path().join("work").join("2026-07-01").join("entry.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Alpha\nneedle\n",
        )
        .unwrap();

        let entries = scan_entries(dir.path()).unwrap();
        let hits = search_loaded_entries(&entries, "needle", SearchScopeFilter::AllJournals);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
        assert_eq!(hits[0].title, "Alpha");
    }

    #[test]
    fn search_can_be_scoped_to_a_journal() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work").join("2026-07-01")).unwrap();
        fs::create_dir_all(dir.path().join("home").join("2026-07-01")).unwrap();
        fs::write(
            dir.path().join("work").join("2026-07-01").join("entry.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Work\nneedle\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("home").join("2026-07-01").join("entry.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Home\nneedle\n",
        )
        .unwrap();

        let entries = scan_entries(dir.path()).unwrap();
        let hits = search_loaded_entries(&entries, "needle", SearchScopeFilter::Journal("work"));

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
        assert_eq!(hits[0].title, "Work");
    }
}
