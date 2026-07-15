use notema_domain::{Entry, EntryEncryptionState, SearchHit, SearchScope, normalize_for_search};

/// Filter already-loaded entries in memory. No disk I/O or decryption — the
/// caller's `Entry` cache already holds decrypted `content` for every entry.
///
/// A prefix-less query matches against the entry body and every metadata field
/// (tags, people, activities, feelings) merged into one haystack. Whitespace
/// splits the query into terms that must all match (AND). Each term is matched
/// against whole *words* in the haystack — as an exact word, a prefix, a
/// substring, or within a small edit distance for typos — never as a scattered
/// subsequence, so a search only surfaces entries that actually contain the
/// word. Matching is case- and accent-insensitive. Field-specific (`tags:` etc.)
/// searches stay exact and are handled by the caller before reaching here.
///
/// Results are ordered by match quality; ties keep their incoming date order.
pub(crate) fn search_loaded_entries(
    entries: &[Entry],
    query: &str,
    scope: &SearchScope,
) -> Vec<SearchHit> {
    let terms: Vec<String> = query.split_whitespace().map(normalize_for_search).collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut scored = Vec::new();
    for entry in entries {
        if matches!(scope, SearchScope::Journal(journal) if &entry.journal != journal) {
            continue;
        }
        if matches!(
            entry.encryption_state,
            EntryEncryptionState::EncryptedLocked | EntryEncryptionState::EncryptedUnreadable
        ) {
            continue;
        }

        if let Some(score) = score_entry(&entry.search_haystack, &terms) {
            scored.push((score, SearchHit::from_entry(entry)));
        }
    }

    // Stable sort by descending score keeps the date order among ties.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, hit)| hit).collect()
}

/// A word boundary: anything that isn't alphanumeric. The haystack is
/// pre-normalized and largely ASCII, so the ASCII range check (no Unicode table
/// lookup) carries the hot path; non-ASCII chars fall back to `is_alphanumeric`.
#[inline]
fn is_separator(c: char) -> bool {
    if c.is_ascii() {
        !c.is_ascii_alphanumeric()
    } else {
        !c.is_alphanumeric()
    }
}

/// Score a pre-normalized `haystack` against every `term` (also normalized).
/// Every term must match (AND); the entry score is the sum of each term's best
/// match quality. Returns `None` if any term matches nothing.
fn score_entry(haystack: &str, terms: &[String]) -> Option<u32> {
    let mut total = 0;
    for term in terms {
        let quality = term_quality(haystack, term);
        if quality == 0 {
            return None;
        }
        total += quality;
    }
    Some(total)
}

/// How well `term` matches, higher is better; `0` means no match. Prefers a
/// whole-word hit, then a prefix, then a contiguous substring (which also
/// covers terms spanning a word boundary, e.g. a hyphenated `project-x`), then
/// a small typo. A substring is contiguous — never a scattered subsequence — so
/// this only matches entries that actually contain the term.
///
/// One lazy pass over the words, returning early the moment a whole word matches
/// exactly (the common case) so most of the haystack is never tokenized. The
/// costlier substring and edit-distance checks run only as fallbacks when no
/// word matched exactly or by prefix.
fn term_quality(haystack: &str, term: &str) -> u32 {
    let mut best = 0;
    for word in haystack.split(is_separator) {
        if word == term {
            return 4; // exact word — nothing ranks higher
        } else if !word.is_empty() && word.starts_with(term) {
            best = 3; // prefix — covers incremental typing
        }
    }
    if best >= 3 {
        return best;
    }
    // A substring (incl. one spanning a word boundary) beats a typo but not a
    // prefix; the SIMD-optimized scan runs only when no prefix hit was found.
    if haystack.contains(term) {
        return 2;
    }
    // Typo tolerance is the last resort — a second pass that only pays the
    // edit-distance cost when the term appears nowhere as a substring.
    let budget = typo_budget(term);
    if budget > 0
        && haystack
            .split(is_separator)
            .any(|word| within_edit_distance(word, term, budget))
    {
        return 1;
    }
    best
}

/// Allowed edit distance for a term, scaled by length so short terms stay
/// exact (avoiding `cat`↔`car` collisions) while long ones tolerate real typos.
fn typo_budget(term: &str) -> usize {
    match term.chars().count() {
        0..=3 => 0,
        4..=6 => 1,
        _ => 2,
    }
}

/// Whether `a` and `b` are within `k` single-char edits (Levenshtein). Skips
/// the full computation when the length gap alone already exceeds `k`, and
/// short-circuits once every cell in a row passes `k`.
fn within_edit_distance(a: &str, b: &str, k: usize) -> bool {
    if k == 0 {
        return a == b;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > k {
        return false;
    }

    // Classic two-row Levenshtein over the shorter axis.
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        let mut row_min = curr[0];
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
            row_min = row_min.min(curr[j + 1]);
        }
        if row_min > k {
            return false;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()] <= k
}

#[cfg(test)]
mod tests {
    use super::*;
    use notema_domain::build_search_haystack;
    use notema_domain::{EntryEncryptionState, Metadata};
    use std::path::PathBuf;

    fn entry_with(id: &str, journal: &str, body: &str, metadata: Metadata) -> Entry {
        let search_haystack = build_search_haystack(body, &metadata);
        let Metadata {
            activities,
            feelings,
            people,
            tags,
            mood,
            starred,
            location,
        } = metadata;
        Entry {
            id: id.to_string(),
            journal: journal.to_string(),
            path: PathBuf::from(format!("{journal}/{id}.md")),
            encryption_state: EntryEncryptionState::Plain,
            created_at: None,
            edited_at: None,
            preview: String::new(),
            activities,
            feelings,
            people,
            tags,
            mood,
            starred,
            location,
            weather: None,
            celestial: None,
            air_quality: None,
            import: None,
            body: body.to_string(),
            word_count: body.split_whitespace().count(),
            search_haystack,
            warning: None,
        }
    }

    fn plain_entry(id: &str, journal: &str, content: &str) -> Entry {
        entry_with(id, journal, content, Metadata::default())
    }

    #[test]
    fn search_matches_content() {
        let entries = vec![
            plain_entry("a", "work", "needle here"),
            plain_entry("b", "work", "nothing"),
        ];

        let hits = search_loaded_entries(&entries, "needle", &SearchScope::AllJournals);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
    }

    #[test]
    fn search_is_case_insensitive() {
        let entries = vec![plain_entry("a", "work", "NEEDLE here")];

        let hits = search_loaded_entries(&entries, "needle", &SearchScope::AllJournals);

        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_can_be_scoped_to_journal() {
        let entries = vec![
            plain_entry("a", "work", "needle"),
            plain_entry("b", "home", "needle"),
        ];

        let hits = search_loaded_entries(
            &entries,
            "needle",
            &SearchScope::Journal("work".to_string()),
        );

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
    }

    #[test]
    fn search_skips_locked_encrypted_entries() {
        let mut entry = plain_entry("a", "work", "needle");
        entry.encryption_state = EntryEncryptionState::EncryptedLocked;

        let hits = search_loaded_entries(&[entry], "needle", &SearchScope::AllJournals);

        assert!(hits.is_empty());
    }

    #[test]
    fn search_skips_unreadable_encrypted_entries() {
        let mut entry = plain_entry("a", "work", "needle");
        entry.encryption_state = EntryEncryptionState::EncryptedUnreadable;

        let hits = search_loaded_entries(&[entry], "needle", &SearchScope::AllJournals);

        assert!(hits.is_empty());
    }

    #[test]
    fn search_matches_metadata_without_prefix() {
        let tagged = entry_with(
            "a",
            "work",
            "nothing relevant",
            Metadata {
                tags: vec!["project-x".to_string()],
                ..Default::default()
            },
        );
        let person = entry_with(
            "b",
            "work",
            "nothing relevant",
            Metadata {
                people: vec!["Alice".to_string()],
                ..Default::default()
            },
        );
        let activity = entry_with(
            "c",
            "work",
            "nothing relevant",
            Metadata {
                activities: vec!["running".to_string()],
                ..Default::default()
            },
        );
        let feeling = entry_with(
            "d",
            "work",
            "nothing relevant",
            Metadata {
                feelings: vec!["happy".to_string()],
                ..Default::default()
            },
        );

        let entries = vec![tagged, person, activity, feeling];

        assert_eq!(
            search_loaded_entries(&entries, "project-x", &SearchScope::AllJournals).len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&entries, "alice", &SearchScope::AllJournals).len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&entries, "running", &SearchScope::AllJournals).len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&entries, "happy", &SearchScope::AllJournals).len(),
            1
        );
    }

    #[test]
    fn multi_word_query_matches_across_body_and_metadata() {
        let entry = entry_with(
            "a",
            "work",
            "hello this is a test",
            Metadata {
                tags: vec!["love".to_string()],
                ..Default::default()
            },
        );

        // Every space-separated atom must match somewhere in the merged haystack.
        assert_eq!(
            search_loaded_entries(
                std::slice::from_ref(&entry),
                "hello test love",
                &SearchScope::AllJournals
            )
            .len(),
            1
        );
        // An atom that appears nowhere fails the whole query.
        assert!(
            search_loaded_entries(&[entry], "hello test missing", &SearchScope::AllJournals)
                .is_empty()
        );
    }

    #[test]
    fn small_typos_still_match() {
        let entries = vec![plain_entry("a", "work", "the quick brown fox")];

        // Dropped letter: "quik" is a subsequence of "quick".
        assert_eq!(
            search_loaded_entries(&entries, "quik brown", &SearchScope::AllJournals).len(),
            1
        );
    }

    #[test]
    fn results_are_ranked_by_relevance() {
        let entries = vec![
            // Looser match: the term is only a prefix of a longer word.
            plain_entry("a", "work", "projections for the quarter"),
            // Exact whole-word match should rank higher.
            plain_entry("b", "work", "project status update"),
        ];

        let hits = search_loaded_entries(&entries, "project", &SearchScope::AllJournals);

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "b");
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let entries = vec![plain_entry("a", "work", "needle")];

        let hits = search_loaded_entries(&entries, "", &SearchScope::AllJournals);

        assert!(hits.is_empty());
    }

    // --- Accent folding ----------------------------------------------------

    #[test]
    fn accent_insensitive_both_directions() {
        let accented = plain_entry("a", "work", "un café à la fenêtre");
        let plain = plain_entry("b", "work", "just cafe here");

        // Unaccented query finds the accented entry, and vice versa.
        assert_eq!(
            search_loaded_entries(
                std::slice::from_ref(&accented),
                "cafe",
                &SearchScope::AllJournals
            )
            .len(),
            1
        );
        assert_eq!(
            search_loaded_entries(&[plain], "café", &SearchScope::AllJournals).len(),
            1
        );
    }

    #[test]
    fn german_umlaut_and_eszett_fold() {
        let entry = plain_entry("a", "work", "über die Straße gegangen");

        assert_eq!(
            search_loaded_entries(
                std::slice::from_ref(&entry),
                "uber",
                &SearchScope::AllJournals
            )
            .len(),
            1
        );
        // ß ↔ ss, either direction.
        assert_eq!(
            search_loaded_entries(
                std::slice::from_ref(&entry),
                "strasse",
                &SearchScope::AllJournals
            )
            .len(),
            1
        );
    }

    // --- Seeded stress corpus ---------------------------------------------
    //
    // A large deterministic corpus of common-word bodies. Because those words
    // share letters with any query, the old subsequence matcher lit up a huge
    // fraction of the corpus for a single unique word; these guard that a query
    // now only surfaces entries that actually contain it.

    use rand::{RngExt, SeedableRng, rngs::StdRng};

    /// Plain, mutually non-overlapping words (none a substring, prefix, or small
    /// typo of another) so a query for one is an unambiguous ground truth.
    const LEXICON: &[&str] = &[
        "river", "garden", "planet", "window", "coffee", "silver", "market", "pencil", "rocket",
        "jungle", "castle", "yellow", "bridge", "summer", "winter", "orange", "basket", "candle",
        "lantern", "harbor",
    ];

    /// A word that appears in no body except the one we plant it in.
    const SENTINEL: &str = "kaleidoscope";

    /// Build `count` entries of random lexicon-word bodies under `seed`, then
    /// plant [`SENTINEL`] in exactly one entry. That entry's id is returned.
    fn seeded_corpus(count: usize, seed: u64) -> (Vec<Entry>, String) {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut entries: Vec<Entry> = (0..count)
            .map(|i| {
                let len = rng.random_range(8..=20);
                let body: String = (0..len)
                    .map(|_| LEXICON[rng.random_range(0..LEXICON.len())])
                    .collect::<Vec<_>>()
                    .join(" ");
                plain_entry(&format!("e{i}"), "work", &body)
            })
            .collect();

        let target = count / 2;
        let id = format!("e{target}");
        entries[target] = plain_entry(&id, "work", &format!("today held a {SENTINEL} of colour"));
        (entries, id)
    }

    #[test]
    fn unique_word_surfaces_exactly_its_entry() {
        let (entries, sentinel_id) = seeded_corpus(5_000, 42);

        let hits = search_loaded_entries(&entries, SENTINEL, &SearchScope::AllJournals);

        assert_eq!(hits.len(), 1, "a unique word must surface only its entry");
        assert_eq!(hits[0].id, sentinel_id);
    }

    #[test]
    fn scattered_subsequence_matches_nothing() {
        let (entries, _) = seeded_corpus(5_000, 42);

        // "kldsc" is a subsequence of "kaleidoscope" but appears as a
        // contiguous substring nowhere — the old matcher would have matched the
        // sentinel entry (and others); the word matcher matches none.
        let hits = search_loaded_entries(&entries, "kldsc", &SearchScope::AllJournals);

        assert!(hits.is_empty());
    }

    #[test]
    fn typo_still_finds_the_unique_entry() {
        let (entries, sentinel_id) = seeded_corpus(5_000, 42);

        // One dropped letter from "kaleidoscope".
        let hits = search_loaded_entries(&entries, "kaleidoscpe", &SearchScope::AllJournals);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, sentinel_id);
    }

    #[test]
    fn common_word_matches_exactly_its_ground_truth() {
        let (entries, _) = seeded_corpus(5_000, 42);
        let query = "planet";

        // Ground truth: entries whose body contains the whole word. The lexicon
        // is built so no other word is a prefix, substring, or small typo of it,
        // so the matcher's decision reduces to exactly this.
        let mut expected: Vec<String> = entries
            .iter()
            .filter(|entry| entry.body.split_whitespace().any(|word| word == query))
            .map(|entry| entry.id.clone())
            .collect();
        expected.sort();

        assert!(
            !expected.is_empty() && expected.len() < entries.len(),
            "the query should hit some but not all entries"
        );

        let mut got: Vec<String> =
            search_loaded_entries(&entries, query, &SearchScope::AllJournals)
                .into_iter()
                .map(|hit| hit.id)
                .collect();
        got.sort();

        assert_eq!(got, expected);
    }

    #[test]
    fn same_seed_is_reproducible() {
        let ids = |seed| {
            let (entries, _) = seeded_corpus(500, seed);
            let mut ids: Vec<String> =
                search_loaded_entries(&entries, "garden", &SearchScope::AllJournals)
                    .into_iter()
                    .map(|hit| hit.id)
                    .collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(7), ids(7));
    }

    // --- Edit distance -----------------------------------------------------

    #[test]
    fn edit_distance_respects_the_budget() {
        assert!(within_edit_distance("quick", "quik", 1)); // one deletion
        assert!(within_edit_distance("color", "colour", 1)); // one insertion
        assert!(!within_edit_distance("cat", "car", 0)); // no budget, differ
        assert!(!within_edit_distance("kitten", "sitting", 2)); // distance 3
        assert!(!within_edit_distance("a", "abcdef", 2)); // length gap alone exceeds k
    }
}
