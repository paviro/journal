use super::*;
use crate::tui::state::{ListNav, SelectableList};

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
            let target = if notema_storage::is_archived_name(&entry.journal) {
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
        if self.editor.is_none() && !self.allow_selected_entry_edit() {
            return;
        }
        let (active_values, archived_only) = self.metadata_partitioned(kind);
        let active_len = active_values.len();
        // Archived-only values live after the active ones; they stay hidden until
        // the user's filter matches them (see `EditMetadataState::rebuild_filter`).
        let all_values: Vec<(String, usize)> =
            active_values.into_iter().chain(archived_only).collect();
        let filtered: Vec<usize> = (0..active_len).collect();
        let entry_tags: Vec<String> = self.editing_metadata_values(kind);
        self.overlay = Overlay::EditMetadata(EditMetadataState::new(
            kind, all_values, filtered, entry_tags, active_len,
        ));
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
}

/// Which part of the metadata edit dialog has keyboard focus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditMetadataFocus {
    #[default]
    List,
    Input,
}

/// State for the free-form metadata overlay.
pub(crate) struct EditMetadataState {
    pub(crate) kind: MetadataKind,
    /// The offerable values: active-journal values first (indices `0..active_len`),
    /// then archived-only values, each sorted by usage count descending.
    pub(crate) all_values: Vec<(String, usize)>,
    /// How many leading `all_values` come from active journals. Archived-only
    /// values (the rest) are shown only when the filter query matches them.
    pub(crate) active_len: usize,
    /// Indices into `all_values` that match the current filter input.
    pub(crate) filtered: Vec<usize>,
    /// Values currently selected for the entry. Original casing is preserved;
    /// membership and dedup are compared case-insensitively.
    pub(crate) selected: Vec<String>,
    /// Stateful list selection and scroll offset.
    pub(crate) list: SelectableList,
    /// Text input for filtering values and adding new ones.
    pub(crate) input: TextInput,
    /// Whether keyboard events go to the list or to the input.
    pub(crate) focus: EditMetadataFocus,
}

impl EditMetadataState {
    pub(crate) fn new(
        kind: MetadataKind,
        all_values: Vec<(String, usize)>,
        filtered: Vec<usize>,
        selected: Vec<String>,
        active_len: usize,
    ) -> Self {
        let mut input = TextInput::default();
        input.set_placeholder_text("search or add");
        let mut state = Self {
            kind,
            all_values,
            active_len,
            filtered,
            selected,
            list: SelectableList::default(),
            input,
            focus: EditMetadataFocus::List,
        };
        state.normalize_list_state();
        state.focus_sole_selection();
        state
    }

    /// When exactly one value is already selected, open with the cursor on it so
    /// the dialog lands on the current choice (the event layer then scrolls it into
    /// view). With none or several selected there's no single value to focus, so the
    /// cursor stays at the top.
    fn focus_sole_selection(&mut self) {
        let [only] = self.selected.as_slice() else {
            return;
        };
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.all_values[i].0.eq_ignore_ascii_case(only))
        {
            self.select_index(pos);
        }
    }

    pub(crate) fn rebuild_filter(&mut self) {
        let query = self.input.as_str().to_lowercase();
        // With no query, offer only the active-journal values. Once the user types,
        // match across everything — including archived-only values — so they can
        // reuse an existing archived tag instead of creating a near-duplicate.
        let search_range = if query.is_empty() {
            0..self.active_len
        } else {
            0..self.all_values.len()
        };
        self.filtered = search_range
            .filter(|&i| self.all_values[i].0.to_lowercase().contains(&query))
            .collect();
        self.list.set_offset(0);
        self.normalize_list_state();
    }

    pub(crate) fn selected_value_index(&self) -> Option<usize> {
        self.selected_index()
            .and_then(|index| self.filtered.get(index).copied())
    }

    pub(crate) fn toggle_selected(&mut self) {
        if let Some(tag_idx) = self.selected_value_index() {
            let tag = self.all_values[tag_idx].0.clone();
            if let Some(pos) = self
                .selected
                .iter()
                .position(|t| t.eq_ignore_ascii_case(&tag))
            {
                self.selected.remove(pos);
            } else {
                self.selected.push(tag);
            }
        }
    }

    /// Add the trimmed input as a selected value, then clear the input and
    /// refilter. A brand-new value is inserted at the active/archived boundary
    /// (a fresh value has count 0, the tail of the active region) and grows it,
    /// so it stays in the range `rebuild_filter` searches with an empty query —
    /// otherwise the newly added value wouldn't show until save + reopen.
    pub(crate) fn add_from_input(&mut self) {
        let input = self.input.as_str().trim();
        if !input.is_empty() {
            // Reuse an existing value's casing on a case-insensitive match so typing
            // "iphone" doesn't fork an existing "iPhone"; otherwise keep it as typed.
            let tag = self
                .all_values
                .iter()
                .find(|(t, _)| t.eq_ignore_ascii_case(input))
                .map_or_else(|| input.to_string(), |(t, _)| t.clone());
            if !self.selected.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                self.selected.push(tag.clone());
                if !self
                    .all_values
                    .iter()
                    .any(|(t, _)| t.eq_ignore_ascii_case(&tag))
                {
                    self.all_values.insert(self.active_len, (tag, 0));
                    self.active_len += 1;
                }
            }
        }
        self.input.clear();
        self.rebuild_filter();
    }
}

impl ListNav for EditMetadataState {
    fn list(&self) -> &SelectableList {
        &self.list
    }

    fn list_mut(&mut self) -> &mut SelectableList {
        &mut self.list
    }

    fn item_count(&self) -> usize {
        self.filtered.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_state(count: usize) -> EditMetadataState {
        let all_values: Vec<(String, usize)> = (0..count)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered: Vec<usize> = (0..count).collect();
        EditMetadataState::new(MetadataKind::Tags, all_values, filtered, Vec::new(), count)
    }

    #[test]
    fn tag_keyboard_selection_scrolls_down_to_remain_visible() {
        let mut state = tag_state(10);

        for _ in 0..5 {
            state.move_down();
            state.ensure_selected_visible(4);
        }

        assert_eq!(state.selected_index(), Some(5));
        assert_eq!(state.offset(), 2);
    }

    #[test]
    fn tag_keyboard_selection_scrolls_up_to_remain_visible() {
        let mut state = tag_state(10);
        state.select_index(5);
        state.list.set_offset(5);

        state.move_up();
        state.ensure_selected_visible(4);

        assert_eq!(state.selected_index(), Some(4));
        assert_eq!(state.offset(), 4);
    }

    #[test]
    fn opening_focuses_the_sole_selected_value() {
        let all_values: Vec<(String, usize)> = (0..10)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered: Vec<usize> = (0..10).collect();
        let state = EditMetadataState::new(
            MetadataKind::Tags,
            all_values,
            filtered,
            vec!["tag-07".to_string()],
            10,
        );
        // A single existing value opens with the cursor on it, so the event layer
        // can scroll it into view instead of stranding it off-screen.
        assert_eq!(state.selected_index(), Some(7));
    }

    #[test]
    fn opening_stays_at_top_with_zero_or_many_selected() {
        let all_values: Vec<(String, usize)> = (0..10)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered: Vec<usize> = (0..10).collect();

        // Several selected: no single "current" value, so start at the top.
        let many = EditMetadataState::new(
            MetadataKind::Tags,
            all_values.clone(),
            filtered.clone(),
            vec!["tag-05".to_string(), "tag-08".to_string()],
            10,
        );
        assert_eq!(many.selected_index(), Some(0));

        // None selected: likewise start at the top.
        let none = EditMetadataState::new(MetadataKind::Tags, all_values, filtered, Vec::new(), 10);
        assert_eq!(none.selected_index(), Some(0));
    }

    #[test]
    fn filter_hides_archived_only_values_until_query_matches() {
        // One active value (index 0) and one archived-only value (index 1).
        let all_values = vec![("berlin".to_string(), 3), ("wanderlust".to_string(), 5)];
        let mut state =
            EditMetadataState::new(MetadataKind::Tags, all_values, vec![0], Vec::new(), 1);

        // With no query only the active value is offered.
        assert_eq!(state.filtered, vec![0]);

        // Typing part of the archived-only value surfaces it (so the user reuses
        // it instead of creating a near-duplicate).
        state.input = "wan".into();
        state.rebuild_filter();
        assert_eq!(state.filtered, vec![1]);

        // Clearing the query hides it again.
        state.input.clear();
        state.rebuild_filter();
        assert_eq!(state.filtered, vec![0]);
    }

    #[test]
    fn add_from_input_shows_the_new_value_immediately() {
        // One active value; an archived-only value sits after the active boundary.
        let all_values = vec![("berlin".to_string(), 3), ("wanderlust".to_string(), 5)];
        let mut state =
            EditMetadataState::new(MetadataKind::Tags, all_values, vec![0], Vec::new(), 1);

        state.input = "hiking".into();
        state.add_from_input();

        // Adding selects the value and clears the input.
        assert!(state.selected.contains(&"hiking".to_string()));
        assert!(state.input.is_empty());
        // The new value lands in the active region and is visible with the empty
        // filter — it must not vanish until save + reopen.
        assert_eq!(state.active_len, 2);
        let shown: Vec<&str> = state
            .filtered
            .iter()
            .map(|&i| state.all_values[i].0.as_str())
            .collect();
        assert!(shown.contains(&"hiking"));
        // The archived-only value stays hidden until its query matches.
        assert!(!shown.contains(&"wanderlust"));
    }

    #[test]
    fn add_from_input_reuses_existing_value_without_duplicating() {
        let mut state = tag_state(3);
        let before = state.all_values.len();

        state.input = "TAG-01".into();
        state.add_from_input();

        // Case-insensitive match to an existing value: selected using the existing
        // casing, and not duplicated.
        assert_eq!(state.all_values.len(), before);
        assert_eq!(state.active_len, 3);
        assert!(state.selected.contains(&"tag-01".to_string()));
    }

    #[test]
    fn add_from_input_keeps_casing_for_a_new_value() {
        let mut state = tag_state(3);
        let before = state.all_values.len();

        state.input = "iPhone".into();
        state.add_from_input();

        // No existing match: the typed casing is preserved and offered.
        assert_eq!(state.all_values.len(), before + 1);
        assert_eq!(state.active_len, 4);
        assert!(state.selected.contains(&"iPhone".to_string()));
        assert!(state.all_values.iter().any(|(t, _)| t == "iPhone"));
    }

    #[test]
    fn toggle_selected_preserves_display_casing() {
        let all_values = vec![("iPhone".to_string(), 5)];
        let mut state =
            EditMetadataState::new(MetadataKind::Tags, all_values, vec![0], Vec::new(), 1);

        state.toggle_selected();

        assert_eq!(state.selected, vec!["iPhone".to_string()]);
    }
}
