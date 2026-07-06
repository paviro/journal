use crate::entry::{Entry, EntryEncryptionState, SearchHit, SearchScopeFilter};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

/// Minimum fuzzy score per matched query char for a hit to show. nucleo awards
/// ~16 points per matched char, so this floor drops the most scattered
/// subsequence matches; raise for stricter matching, lower for looser.
const MIN_SCORE_PER_CHAR: u32 = 12;

/// Filter already-loaded entries in memory. No disk I/O or decryption — the
/// caller's `Entry` cache already holds decrypted `content` for every entry.
///
/// A prefix-less query is matched fuzzily against the entry body and every
/// metadata field (tags, people, activities, feelings) merged into one
/// haystack. Whitespace splits the query into atoms that must all match (AND),
/// and small typos are tolerated. Field-specific (`tags:` etc.) searches stay
/// exact and are handled by the caller before reaching here.
///
/// Results are ordered by fuzzy relevance; ties keep their incoming date order.
pub fn search_loaded_entries(
    entries: &[Entry],
    query: &str,
    scope: SearchScopeFilter<'_>,
) -> Vec<SearchHit> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let pattern = Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    // Reused scratch buffer for the UTF-32 transcode so scoring each entry
    // doesn't reallocate. The haystack string itself is precomputed per entry
    // (`Entry::search_haystack`), so this loop never rebuilds it.
    let mut char_buf = Vec::new();

    // Score floor scaled by the number of query characters we expect to match.
    let query_chars = trimmed.chars().filter(|c| !c.is_whitespace()).count() as u32;
    let min_score = query_chars * MIN_SCORE_PER_CHAR;

    let mut scored = Vec::new();
    for entry in entries {
        if matches!(scope, SearchScopeFilter::Journal(journal) if entry.journal != journal) {
            continue;
        }
        if entry.encryption_state == EntryEncryptionState::EncryptedLocked {
            continue;
        }

        let candidate = Utf32Str::new(&entry.search_haystack, &mut char_buf);
        if let Some(score) = pattern
            .score(candidate, &mut matcher)
            .filter(|&s| s >= min_score)
        {
            scored.push((
                score,
                SearchHit {
                    id: entry.id.clone(),
                    journal: entry.journal.clone(),
                    created_at: entry.created_at.clone(),
                    title: entry.display_label(),
                    preview: entry.preview.clone(),
                },
            ));
        }
    }

    // Stable sort by descending score keeps the date order among ties.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, hit)| hit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryEncryptionState;
    use std::path::PathBuf;

    fn plain_entry(id: &str, journal: &str, content: &str) -> Entry {
        let mut entry = Entry {
            id: id.to_string(),
            journal: journal.to_string(),
            path: PathBuf::from(format!("{journal}/{id}.md")),
            encryption_state: EntryEncryptionState::Plain,
            created_at: None,
            created: None,
            updated_at: None,
            preview: String::new(),
            tags: Vec::new(),
            people: Vec::new(),
            activities: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            import_id: None,
            content: content.to_string(),
            word_count: content.split_whitespace().count(),
            search_haystack: String::new(),
        };
        entry.rebuild_search_haystack();
        entry
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
    fn search_matches_metadata_without_prefix() {
        let mut tagged = plain_entry("a", "work", "nothing relevant");
        tagged.tags = vec!["project-x".to_string()];
        tagged.rebuild_search_haystack();
        let mut person = plain_entry("b", "work", "nothing relevant");
        person.people = vec!["Alice".to_string()];
        person.rebuild_search_haystack();
        let mut activity = plain_entry("c", "work", "nothing relevant");
        activity.activities = vec!["running".to_string()];
        activity.rebuild_search_haystack();
        let mut feeling = plain_entry("d", "work", "nothing relevant");
        feeling.feelings = vec!["happy".to_string()];
        feeling.rebuild_search_haystack();

        let entries = vec![tagged, person, activity, feeling];

        assert_eq!(
            search_loaded_entries(&entries, "project-x", SearchScopeFilter::AllJournals).len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&entries, "alice", SearchScopeFilter::AllJournals).len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&entries, "running", SearchScopeFilter::AllJournals).len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&entries, "happy", SearchScopeFilter::AllJournals).len(),
            1
        );
    }

    #[test]
    fn multi_word_query_matches_across_body_and_metadata() {
        let mut entry = plain_entry("a", "work", "hello this is a test");
        entry.tags = vec!["love".to_string()];
        entry.rebuild_search_haystack();

        // Every space-separated atom must match somewhere in the merged haystack.
        assert_eq!(
            search_loaded_entries(
                &[entry.clone()],
                "hello test love",
                SearchScopeFilter::AllJournals
            )
            .len(),
            1
        );
        // An atom that appears nowhere fails the whole query.
        assert!(
            search_loaded_entries(
                &[entry],
                "hello test missing",
                SearchScopeFilter::AllJournals
            )
            .is_empty()
        );
    }

    #[test]
    fn small_typos_still_match() {
        let entries = vec![plain_entry("a", "work", "the quick brown fox")];

        // Dropped letter: "quik" is a subsequence of "quick".
        assert_eq!(
            search_loaded_entries(&entries, "quik brown", SearchScopeFilter::AllJournals).len(),
            1
        );
    }

    #[test]
    fn results_are_ranked_by_relevance() {
        let entries = vec![
            // Looser match: the word is split by a gap.
            plain_entry("a", "work", "pro ject notes"),
            // Exact contiguous match should rank higher.
            plain_entry("b", "work", "project status update"),
        ];

        let hits = search_loaded_entries(&entries, "project", SearchScopeFilter::AllJournals);

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "b");
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let entries = vec![plain_entry("a", "work", "needle")];

        let hits = search_loaded_entries(&entries, "", SearchScopeFilter::AllJournals);

        assert!(hits.is_empty());
    }
}
