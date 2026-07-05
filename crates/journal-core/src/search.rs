use crate::entry::{Entry, EntryEncryptionState, SearchHit, SearchScopeFilter};

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
                id: entry.id.clone(),
                journal: entry.journal.clone(),
                created_at: entry.created_at.clone(),
                title: entry.display_label(),
                preview: entry.preview.clone(),
            });
        }
    }

    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryEncryptionState;
    use std::path::PathBuf;

    fn plain_entry(id: &str, journal: &str, content: &str) -> Entry {
        Entry {
            id: id.to_string(),
            journal: journal.to_string(),
            path: PathBuf::from(format!("{journal}/{id}.md")),
            encryption_state: EntryEncryptionState::Plain,
            created_at: None,
            updated_at: None,
            preview: String::new(),
            tags: Vec::new(),
            people: Vec::new(),
            activities: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            content: content.to_string(),
        }
    }

    #[test]
    fn search_matches_content() {
        let entries = vec![
            plain_entry("a", "work", "needle here"),
            plain_entry("b", "work", "nothing"),
        ];

        let hits = search_loaded_entries(&entries, "needle", SearchScopeFilter::AllJournals);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
    }

    #[test]
    fn search_is_case_insensitive() {
        let entries = vec![plain_entry("a", "work", "NEEDLE here")];

        let hits = search_loaded_entries(&entries, "needle", SearchScopeFilter::AllJournals);

        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_can_be_scoped_to_journal() {
        let entries = vec![
            plain_entry("a", "work", "needle"),
            plain_entry("b", "home", "needle"),
        ];

        let hits = search_loaded_entries(&entries, "needle", SearchScopeFilter::Journal("work"));

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
    }

    #[test]
    fn search_skips_locked_encrypted_entries() {
        let mut entry = plain_entry("a", "work", "needle");
        entry.encryption_state = EntryEncryptionState::EncryptedLocked;

        let hits = search_loaded_entries(&[entry], "needle", SearchScopeFilter::AllJournals);

        assert!(hits.is_empty());
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let entries = vec![plain_entry("a", "work", "needle")];

        let hits = search_loaded_entries(&entries, "", SearchScopeFilter::AllJournals);

        assert!(hits.is_empty());
    }
}
