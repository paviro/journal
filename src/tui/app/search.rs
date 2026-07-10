use super::*;

impl App {
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
    pub(super) fn enter_search(&mut self, scope: SearchScope, query: String, hits: Vec<SearchHit>) {
        self.search.scope = scope;
        self.nav.mode = Mode::Search;
        self.nav.focus = Focus::Entries;
        self.search.cursor = query.chars().count();
        self.search.query = query;
        self.search.hits = hits;
        self.commit_search_selection();
    }

    pub(crate) fn exit_search(&mut self) {
        self.nav.mode = Mode::Browse;
        self.search.scope = SearchScope::AllJournals;
        self.search.query.clear();
        self.search.cursor = 0;
        self.search.hits.clear();
        self.commit_search_selection();
    }

    pub(crate) fn update_search_results(&mut self) {
        self.search.hits = self.search_results();
        self.commit_search_selection();
    }

    /// The search scope for a metadata/feeling drill-down: the selected journal,
    /// or all journals when none is selected.
    pub(super) fn current_journal_scope(&self) -> SearchScope {
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

    /// The search caret is active (blinking) only while typing in the field.
    pub(crate) fn is_search_input_active(&self) -> bool {
        self.nav.mode == Mode::Search && self.nav.focus == Focus::Entries
    }

    /// Byte offset in `query` for the current caret char index, clamped to the end.
    fn search_cursor_byte(&self) -> usize {
        self.search
            .query
            .char_indices()
            .nth(self.search.cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(self.search.query.len())
    }

    /// Insert a typed char at the caret and advance it.
    pub(crate) fn search_insert(&mut self, ch: char) {
        let byte = self.search_cursor_byte();
        self.search.query.insert(byte, ch);
        self.search.cursor += 1;
        self.mark_search_dirty();
    }

    /// Delete the char before the caret (Backspace).
    pub(crate) fn search_backspace(&mut self) {
        if self.search.cursor == 0 {
            return;
        }
        self.search.cursor -= 1;
        let byte = self.search_cursor_byte();
        self.search.query.remove(byte);
        self.mark_search_dirty();
    }

    pub(crate) fn search_cursor_left(&mut self) {
        self.search.cursor = self.search.cursor.saturating_sub(1);
    }

    pub(crate) fn search_cursor_right(&mut self) {
        let max = self.search.query.chars().count();
        self.search.cursor = (self.search.cursor + 1).min(max);
    }

    pub(super) fn search_results(&self) -> Vec<SearchHit> {
        if let Some(tag) = self.search.query.strip_prefix("tags:") {
            self.search_results_by_metadata(MetadataKind::Tags, tag.trim())
        } else if let Some(person) = self.search.query.strip_prefix("people:") {
            self.search_results_by_metadata(MetadataKind::People, person.trim())
        } else if let Some(activity) = self.search.query.strip_prefix("activities:") {
            self.search_results_by_metadata(MetadataKind::Activities, activity.trim())
        } else if let Some(feeling) = self.search.query.strip_prefix("feelings:") {
            self.search_results_by_feeling(feeling.trim())
        } else if let Some(value) = self.search.query.strip_prefix("star:") {
            match parse_starred_value(value) {
                Some(want) => self.search_results_by_starred(want),
                // An unparseable flag (e.g. `star:maybe`) matches nothing,
                // mirroring how an unknown `feelings:` value yields no hits.
                None => Vec::new(),
            }
        } else {
            search_loaded_entries(
                &self.library.entries,
                &self.search.query,
                &self.search.scope,
            )
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

    pub(super) fn search_results_by_metadata(
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

    pub(super) fn search_results_by_feeling(&self, feeling: &str) -> Vec<SearchHit> {
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

    pub(super) fn search_results_by_starred(&self, want: bool) -> Vec<SearchHit> {
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
