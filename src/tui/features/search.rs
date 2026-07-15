use std::time::Instant;

use notema_domain::{Entry, EntryEncryptionState, SearchHit, normalize_feeling};

use crate::tui::{
    app::{AppModel, Focus, Mode, SearchScope},
    features::metadata::metadata_values,
    search::search_loaded_entries,
    state::MetadataKind,
};

impl AppModel {
    pub(crate) fn begin_search(&mut self) {
        let scope = if self.nav.focus == Focus::Journals {
            SearchScope::AllJournals
        } else {
            self.current_journal_scope()
        };
        self.enter_search(scope, String::new(), Vec::new());
    }

    /// Enter search mode with a prepared `query`/`hits`, focusing the entry list
    /// and selecting the first hit.
    pub(crate) fn enter_search(&mut self, scope: SearchScope, query: String, hits: Vec<SearchHit>) {
        self.search.scope = scope;
        self.nav.mode = Mode::Search;
        self.nav.focus = Focus::Entries;
        self.search.query.set_text(&query);
        self.search.hits = hits;
        self.commit_search_selection();
        // An all-journals search follows the global theme (see context_journal).
        self.apply_effective_theme();
    }

    pub(crate) fn exit_search(&mut self) {
        self.nav.mode = Mode::Browse;
        self.search.scope = SearchScope::AllJournals;
        self.search.query.clear();
        self.search.hits.clear();
        self.commit_search_selection();
        self.apply_effective_theme();
    }

    pub(crate) fn update_search_results(&mut self) {
        self.search.hits = self.search_results();
        self.commit_search_selection();
    }

    /// The search scope for a metadata/feeling drill-down: the selected journal,
    /// or all journals when none is selected.
    pub(crate) fn current_journal_scope(&self) -> SearchScope {
        self.selected_journal()
            .map(|journal| SearchScope::Journal(journal.name.clone()))
            .unwrap_or(SearchScope::AllJournals)
    }

    /// Shared tail of every search entry/exit: clear the debounce state,
    /// invalidate the row cache, and reset the selection to the first hit.
    fn commit_search_selection(&mut self) {
        self.search.dirty = false;
        self.search.last_edit = None;
        self.caches.bump_rows();
        self.nav.selected_entry_index = (!self.search.hits.is_empty()).then_some(0);
        self.reset_entry_scroll();
    }

    /// Mark the search query as changed without running the (expensive) hit
    /// recompute yet. The event loop calls [`Self::update_search_results`] once
    /// typing pauses, so a fast typist doesn't re-scan the whole corpus per key.
    fn mark_search_dirty(&mut self) {
        self.search.dirty = true;
        self.search.last_edit = Some(Instant::now());
    }

    /// The search field owns the caret only while typing in it.
    pub(crate) fn is_search_input_active(&self) -> bool {
        self.nav.mode == Mode::Search && self.nav.focus == Focus::Entries
    }

    /// Feed a key press to the search field, deferring the hit recompute when
    /// it changed the query (debounce).
    pub(crate) fn search_input_key(&mut self, key: crossterm::event::KeyEvent) {
        if self.search.query.input(key) {
            self.mark_search_dirty();
        }
    }

    /// Insert a pasted block into the search field, deferring the hit recompute
    /// like [`Self::search_input_key`].
    pub(crate) fn search_input_paste(&mut self, text: &str) {
        if self.search.query.paste_str(text) {
            self.mark_search_dirty();
        }
    }

    pub(crate) fn search_results(&self) -> Vec<SearchHit> {
        let query = self.search.query.as_str();
        if let Some(tag) = query.strip_prefix("tags:") {
            self.search_results_by_metadata(MetadataKind::Tags, tag.trim())
        } else if let Some(person) = query.strip_prefix("people:") {
            self.search_results_by_metadata(MetadataKind::People, person.trim())
        } else if let Some(activity) = query.strip_prefix("activities:") {
            self.search_results_by_metadata(MetadataKind::Activities, activity.trim())
        } else if let Some(feeling) = query.strip_prefix("feelings:") {
            self.search_results_by_feeling(feeling.trim())
        } else if let Some(value) = query.strip_prefix("star:") {
            match parse_starred_value(value) {
                Some(want) => self.search_results_by_starred(want),
                // An unparseable flag (e.g. `star:maybe`) matches nothing,
                // mirroring how an unknown `feelings:` value yields no hits.
                None => Vec::new(),
            }
        } else {
            search_loaded_entries(&self.library.entries, query, &self.search.scope)
        }
    }

    /// Build hits from the in-scope, unlocked entries matching `predicate`.
    fn search_results_matching(&self, predicate: impl Fn(&Entry) -> bool) -> Vec<SearchHit> {
        self.library
            .entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.encryption_state,
                    EntryEncryptionState::EncryptedLocked
                        | EntryEncryptionState::EncryptedUnreadable
                ) && match self.search.scope {
                    SearchScope::AllJournals => true,
                    SearchScope::Journal(ref journal) => entry.journal == *journal,
                } && predicate(entry)
            })
            .map(SearchHit::from_entry)
            .collect()
    }

    pub(crate) fn search_results_by_metadata(
        &self,
        kind: MetadataKind,
        query: &str,
    ) -> Vec<SearchHit> {
        let query_lower = query.to_lowercase();
        self.search_results_matching(|entry| {
            metadata_values(entry, kind)
                .iter()
                .any(|value| value.to_lowercase().contains(&query_lower))
        })
    }

    pub(crate) fn search_results_by_feeling(&self, feeling: &str) -> Vec<SearchHit> {
        let Some(feeling) = normalize_feeling(feeling) else {
            return Vec::new();
        };
        self.search_results_matching(|entry| {
            entry
                .feelings
                .iter()
                .any(|entry_feeling| entry_feeling == &feeling)
        })
    }

    pub(crate) fn search_results_by_starred(&self, want: bool) -> Vec<SearchHit> {
        self.search_results_matching(|entry| entry.starred == want)
    }
}

/// Parse the value of a `star:` query. An empty value is the friendlier
/// `true` (the common intent when filtering for favorites); `true`/`1` and
/// `false`/`0` are accepted case-insensitively; anything else is `None` (no
/// match).
fn parse_starred_value(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "" | "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}
