use super::*;

/// A list of `(display value, usage count)` pairs, sorted by count descending.
pub(crate) type MetadataCounts = Vec<(String, usize)>;

impl App {
    /// Split metadata values into `(active, archived_only)`. Archived journals
    /// don't contribute to the offered list or usage counts, so `active` counts
    /// only non-archived entries. `archived_only` holds values that appear *solely*
    /// in archived journals (with their archived usage counts) — surfaced in the
    /// picker when the user's filter matches, so they don't recreate a variant of
    /// an existing tag.
    pub(crate) fn metadata_partitioned(
        &self,
        kind: MetadataKind,
    ) -> (MetadataCounts, MetadataCounts) {
        use std::collections::BTreeMap;

        let mut active: BTreeMap<String, CasingCount> = BTreeMap::new();
        let mut archived: BTreeMap<String, CasingCount> = BTreeMap::new();
        for entry in &self.library.entries {
            let target = if journal_storage::is_archived_name(&entry.journal) {
                &mut archived
            } else {
                &mut active
            };
            for value in metadata_values(entry, kind) {
                let lower = value.to_lowercase();
                let cc = target.entry(lower).or_default();
                cc.total += 1;
                *cc.forms.entry(value.clone()).or_default() += 1;
            }
        }

        // Keep only archived values whose key never appears in an active journal.
        archived.retain(|key, _| !active.contains_key(key));
        (sort_casing(active), sort_casing(archived))
    }

    pub(crate) fn begin_edit_tags(&mut self) {
        self.begin_edit_metadata(MetadataKind::Tags);
    }

    pub(crate) fn begin_edit_people(&mut self) {
        self.begin_edit_metadata(MetadataKind::People);
    }

    pub(crate) fn begin_edit_activities(&mut self) {
        self.begin_edit_metadata(MetadataKind::Activities);
    }

    fn begin_edit_metadata(&mut self, kind: MetadataKind) {
        let (active_values, archived_only) = self.metadata_partitioned(kind);
        let active_len = active_values.len();
        // Archived-only values live after the active ones; they stay hidden until
        // the user's filter matches them (see `EditMetadataState::rebuild_filter`).
        let all_values: Vec<(String, usize)> =
            active_values.into_iter().chain(archived_only).collect();
        let filtered: Vec<usize> = (0..active_len).collect();
        let entry_tags: Vec<String> = self.selected_entry_metadata(kind).into_iter().collect();
        self.overlay = Overlay::EditMetadata(EditMetadataState::new(
            kind, all_values, filtered, entry_tags, active_len,
        ));
    }

    pub(crate) fn begin_edit_feelings(&mut self) {
        let selected = self.selected_entry_feelings();
        self.overlay = Overlay::EditFeelings(EditFeelingState::new(FEELING_GROUPS, selected));
    }

    pub(crate) fn begin_tag_search(&mut self, tag: &str) {
        self.begin_metadata_search(MetadataKind::Tags, tag);
    }

    pub(crate) fn begin_people_search(&mut self, person: &str) {
        self.begin_metadata_search(MetadataKind::People, person);
    }

    pub(crate) fn begin_activity_search(&mut self, activity: &str) {
        self.begin_metadata_search(MetadataKind::Activities, activity);
    }

    fn begin_metadata_search(&mut self, kind: MetadataKind, value: &str) {
        let scope = self.current_journal_scope();
        let hits = self.search_results_by_metadata(kind, value);
        self.enter_search(scope, format!("{}:{value}", kind.search_prefix()), hits);
    }

    pub(crate) fn begin_feeling_search(&mut self, feeling: &str) {
        let scope = self.current_journal_scope();
        let hits = self.search_results_by_feeling(feeling);
        self.enter_search(scope, format!("feelings:{feeling}"), hits);
    }
}
