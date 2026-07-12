use super::*;
use crate::tui::state::{ListNav, SelectableList};
use notema_domain::FeelingGroup;

impl App {
    pub(crate) fn begin_edit_feelings(&mut self) {
        if self.editor.is_none() && !self.allow_selected_entry_edit() {
            return;
        }
        let selected = self.editing_feelings();
        self.overlay = Overlay::EditFeelings(EditFeelingState::new(FEELING_GROUPS, selected));
    }

    pub(crate) fn begin_feeling_search(&mut self, feeling: &str) {
        let scope = self.current_journal_scope();
        let hits = self.search_results_by_feeling(feeling);
        self.enter_search(scope, format!("feelings:{feeling}"), hits);
    }
}

/// One visible row in the feelings picker: a group heading or a feeling under an
/// expanded group. Both carry indices back into [`EditFeelingState::groups`].
pub(crate) enum FeelingRow {
    Header { group: usize },
    Feeling { group: usize, feeling: usize },
}

/// State for the edit-feelings overlay. The vocabulary is the borrowed `'static`
/// [`FeelingGroup`] table; only expansion, selection, and the search box are
/// per-session. Groups collapse/expand, so the set of visible rows (and thus the
/// navigable list) changes as the user opens groups. A search box filters across
/// every group into a flat list of matches.
pub(crate) struct EditFeelingState {
    /// The canonical vocabulary, borrowed from `notema_domain::FEELING_GROUPS`.
    pub(crate) groups: &'static [FeelingGroup],
    /// Whether each group is expanded, parallel to `groups`. Groups start collapsed.
    pub(crate) expanded: Vec<bool>,
    /// Feelings currently selected for the entry (lowercased).
    pub(crate) selected: Vec<String>,
    /// Stateful selection and scroll offset over the *visible* rows.
    pub(crate) list: SelectableList,
    /// Text filtering the vocabulary; empty shows the grouped view.
    pub(crate) input: TextInput,
    /// Whether keyboard events go to the list or the search input.
    pub(crate) focus: EditMetadataFocus,
}

impl EditFeelingState {
    pub(crate) fn new(groups: &'static [FeelingGroup], selected: Vec<String>) -> Self {
        let mut input = TextInput::default();
        input.set_placeholder_text("type to search");
        let mut state = Self {
            expanded: vec![false; groups.len()],
            groups,
            selected,
            list: SelectableList::default(),
            input,
            focus: EditMetadataFocus::List,
        };
        state.normalize_list_state();
        state.select_index(0);
        state
    }

    /// Whether a search query is narrowing the list.
    pub(crate) fn is_filtering(&self) -> bool {
        !self.input.as_str().trim().is_empty()
    }

    /// The rows currently shown. With no query: every header, plus the feelings of
    /// expanded groups. While filtering: a flat list of matching feelings (no
    /// headers), so a match is reachable without opening its group first.
    pub(crate) fn visible_rows(&self) -> Vec<FeelingRow> {
        let mut rows = Vec::new();
        if self.is_filtering() {
            let query = self.input.as_str().trim().to_lowercase();
            for (group, g) in self.groups.iter().enumerate() {
                for (feeling, item) in g.feelings.iter().enumerate() {
                    let alias_match = item
                        .search_aliases
                        .iter()
                        .any(|alias| alias.contains(&query));
                    if item.name.contains(&query) || alias_match {
                        rows.push(FeelingRow::Feeling { group, feeling });
                    }
                }
            }
            return rows;
        }
        for (group, g) in self.groups.iter().enumerate() {
            rows.push(FeelingRow::Header { group });
            if self.expanded[group] {
                for feeling in 0..g.feelings.len() {
                    rows.push(FeelingRow::Feeling { group, feeling });
                }
            }
        }
        rows
    }

    /// The number of [`visible_rows`](Self::visible_rows), computed without
    /// allocating — this is the `ListNav` item count, queried on every navigation,
    /// layout and render pass.
    pub(crate) fn visible_row_count(&self) -> usize {
        if self.is_filtering() {
            let query = self.input.as_str().trim().to_lowercase();
            return self
                .groups
                .iter()
                .flat_map(|g| g.feelings.iter())
                .filter(|item| {
                    item.name.contains(&query)
                        || item
                            .search_aliases
                            .iter()
                            .any(|alias| alias.contains(&query))
                })
                .count();
        }
        self.groups
            .iter()
            .enumerate()
            .map(|(group, g)| {
                1 + if self.expanded[group] {
                    g.feelings.len()
                } else {
                    0
                }
            })
            .sum()
    }

    /// Toggle keyboard focus between the list and the search input.
    pub(crate) fn switch_focus(&mut self) {
        self.focus = match self.focus {
            EditMetadataFocus::List => EditMetadataFocus::Input,
            EditMetadataFocus::Input => EditMetadataFocus::List,
        };
    }

    /// Re-run the filter after the query changed: reset the scroll and land the
    /// cursor on the first match.
    pub(crate) fn rebuild_filter(&mut self) {
        self.list.set_offset(0);
        self.normalize_list_state();
        self.select_index(0);
    }

    /// How many of `group`'s feelings are currently selected.
    pub(crate) fn group_selected_count(&self, group: usize) -> usize {
        self.groups[group]
            .feelings
            .iter()
            .filter(|item| {
                self.selected
                    .iter()
                    .any(|value| value.as_str() == item.name)
            })
            .count()
    }

    /// Visible-row index of `group`'s header.
    fn header_index(&self, group: usize) -> usize {
        (0..group)
            .map(|g| {
                1 + if self.expanded[g] {
                    self.groups[g].feelings.len()
                } else {
                    0
                }
            })
            .sum()
    }

    /// Space/click on the current row: toggle a feeling's selection, or fold the
    /// group open/closed when the row is a header.
    pub(crate) fn toggle_selected(&mut self) {
        let rows = self.visible_rows();
        let Some(row) = self.selected_index().and_then(|index| rows.get(index)) else {
            return;
        };
        match *row {
            FeelingRow::Header { group } => self.expanded[group] = !self.expanded[group],
            FeelingRow::Feeling { group, feeling } => {
                let name = self.groups[group].feelings[feeling].name;
                if let Some(pos) = self.selected.iter().position(|v| v.as_str() == name) {
                    self.selected.remove(pos);
                } else {
                    self.selected.push(name.to_string());
                }
            }
        }
    }

    /// Right arrow: open the group under the cursor (a header, or the group a
    /// feeling belongs to — already open in that case). No-op while filtering,
    /// where the flat match list has no groups to fold.
    pub(crate) fn expand_selected(&mut self) {
        if self.is_filtering() {
            return;
        }
        let rows = self.visible_rows();
        if let Some(FeelingRow::Header { group }) =
            self.selected_index().and_then(|index| rows.get(index))
        {
            self.expanded[*group] = true;
        }
    }

    /// Left arrow: close the group under the cursor. When a feeling is focused,
    /// collapse its parent group and move the cursor up to that header. No-op
    /// while filtering.
    pub(crate) fn collapse_selected(&mut self) {
        if self.is_filtering() {
            return;
        }
        let rows = self.visible_rows();
        match self.selected_index().and_then(|index| rows.get(index)) {
            Some(FeelingRow::Header { group }) => self.expanded[*group] = false,
            Some(FeelingRow::Feeling { group, .. }) => {
                let group = *group;
                self.expanded[group] = false;
                let header = self.header_index(group);
                self.select_index(header);
            }
            None => {}
        }
    }
}

impl ListNav for EditFeelingState {
    fn list(&self) -> &SelectableList {
        &self.list
    }

    fn list_mut(&mut self) -> &mut SelectableList {
        &mut self.list
    }

    fn item_count(&self) -> usize {
        self.visible_row_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notema_domain::Feeling;

    static FEELING_FIXTURE: &[FeelingGroup] = &[
        FeelingGroup {
            name: "Peace",
            feelings: &[
                Feeling {
                    name: "calm",
                    search_aliases: &["composed"],
                },
                Feeling {
                    name: "content",
                    search_aliases: &[],
                },
            ],
        },
        FeelingGroup {
            name: "Joy",
            feelings: &[Feeling {
                name: "joyful",
                search_aliases: &[],
            }],
        },
    ];

    fn feeling_state() -> EditFeelingState {
        EditFeelingState::new(FEELING_FIXTURE, Vec::new())
    }

    #[test]
    fn feelings_start_collapsed_showing_only_headers() {
        let state = feeling_state();
        assert_eq!(state.item_count(), 2);
        assert_eq!(state.selected_index(), Some(0));
        assert!(matches!(
            state.visible_rows()[0],
            FeelingRow::Header { group: 0 }
        ));
    }

    #[test]
    fn feelings_expand_and_collapse_change_visible_rows() {
        let mut state = feeling_state();
        state.expand_selected(); // open "Peace" (2 feelings)
        assert_eq!(state.item_count(), 4); // header + 2 feelings + header
        state.move_down();
        state.move_down(); // now on "content"
        assert!(matches!(
            state.visible_rows()[state.selected_index().unwrap()],
            FeelingRow::Feeling {
                group: 0,
                feeling: 1
            }
        ));
        // Collapsing from inside the group returns the cursor to its header.
        state.collapse_selected();
        assert_eq!(state.item_count(), 2);
        assert_eq!(state.selected_index(), Some(0));
    }

    #[test]
    fn visible_row_count_matches_visible_rows_len() {
        let mut state = feeling_state();
        let check = |state: &EditFeelingState| {
            assert_eq!(state.visible_row_count(), state.visible_rows().len());
        };
        check(&state); // all collapsed
        state.expand_selected();
        check(&state); // one group open
        state.input = "co".into();
        state.rebuild_filter();
        check(&state); // filtering
    }

    #[test]
    fn feelings_toggle_selects_feelings_and_folds_headers() {
        let mut state = feeling_state();
        // Space on a header expands it rather than selecting anything.
        state.toggle_selected();
        assert!(state.selected.is_empty());
        assert!(state.expanded[0]);
        // Space on a feeling toggles selection.
        state.move_down();
        state.toggle_selected();
        assert_eq!(state.selected, vec!["calm".to_string()]);
        assert_eq!(state.group_selected_count(0), 1);
    }

    #[test]
    fn feelings_search_flattens_matches_across_groups() {
        let mut state = feeling_state();
        state.input = "cont".into(); // matches "content" only
        state.rebuild_filter();

        assert!(state.is_filtering());
        let rows = state.visible_rows();
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            rows[0],
            FeelingRow::Feeling {
                group: 0,
                feeling: 1
            } // content
        ));
        // Toggling the sole match selects it.
        state.toggle_selected();
        assert_eq!(state.selected, vec!["content".to_string()]);

        // Expand/collapse are inert while filtering.
        state.expand_selected();
        state.collapse_selected();
        assert!(!state.expanded[0]);
    }

    #[test]
    fn feelings_search_matches_aliases_and_selects_canonical() {
        let mut state = feeling_state();
        state.input = "composed".into(); // alias of "calm"
        state.rebuild_filter();

        let rows = state.visible_rows();
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            rows[0],
            FeelingRow::Feeling {
                group: 0,
                feeling: 0
            } // calm
        ));
        // Toggling the alias match selects the canonical feeling, not the alias.
        state.toggle_selected();
        assert_eq!(state.selected, vec!["calm".to_string()]);
    }

    #[test]
    fn feelings_clearing_search_restores_grouped_view() {
        let mut state = feeling_state();
        state.input = "happy-nope".into();
        state.rebuild_filter();
        assert!(state.visible_rows().is_empty());

        state.input.clear();
        state.rebuild_filter();
        // Back to one row per (collapsed) group header.
        assert_eq!(state.item_count(), state.groups.len());
    }
}
