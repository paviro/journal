//! Focused state containers held by [`App`](super::app::App), split out so the
//! reset/lifecycle logic for each concern lives in one place.

use std::time::{Duration, Instant};

use journal_core::feelings::FeelingGroup;
use journal_storage::SearchHit;
use ratatui::widgets::ListState;

use super::app::SearchScope;
use super::image::ImageAsset;

const STATUS_DURATION: Duration = Duration::from_secs(3);

/// Vertical scroll offsets for the panels that scroll their own body: the entry
/// preview, and the insights panel's ranked-list tabs (People / Activities / Tags).
#[derive(Default)]
pub(crate) struct ScrollState {
    pub(crate) entry_view: u16,
    /// First visible row of the insights list tabs, in row units (not pixels).
    pub(crate) insights: u16,
}

impl ScrollState {
    /// Reset the entry preview scroll.
    pub(crate) fn reset_entry_view(&mut self) {
        self.entry_view = 0;
    }

    /// Reset the insights list scroll — called when the tab, scope, or journal
    /// changes so a new list starts at the top.
    pub(crate) fn reset_insights(&mut self) {
        self.insights = 0;
    }
}

/// Transient status-bar message with an auto-expiry deadline.
#[derive(Default)]
pub(crate) struct StatusBar {
    text: String,
    until: Option<Instant>,
}

impl StatusBar {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn set(&mut self, message: impl Into<String>) {
        self.text = message.into();
        self.until = Some(Instant::now() + STATUS_DURATION);
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.until = None;
    }

    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.until
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    /// Clear the status if its deadline has passed, reporting whether it did.
    pub(crate) fn expire(&mut self) -> bool {
        if self
            .until
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.clear();
            return true;
        }

        false
    }

    /// Set a message whose deadline is already in the past (test helper).
    #[cfg(test)]
    pub(crate) fn set_expired(&mut self, message: impl Into<String>) {
        self.text = message.into();
        self.until = Some(Instant::now() - Duration::from_secs(1));
    }
}

/// Search query, scope and the hits it currently matches.
pub(crate) struct SearchState {
    pub(crate) query: String,
    /// Caret position as a char index into `query`, in `0..=query.chars().count()`.
    pub(crate) cursor: usize,
    pub(crate) scope: SearchScope,
    pub(crate) hits: Vec<SearchHit>,
    /// Blink phase of the search caret; toggled on a timer by the event loop and
    /// read when rendering the search field. `true` = caret block shown.
    pub(crate) cursor_visible: bool,
    /// Set when the query changed but the (expensive) hit recompute has been
    /// deferred; the event loop runs it once typing pauses (debounce).
    pub(crate) dirty: bool,
    /// Timestamp of the last search keystroke, for the debounce window.
    pub(crate) last_edit: Option<Instant>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            scope: SearchScope::AllJournals,
            hits: Vec::new(),
            cursor_visible: true,
            dirty: false,
            last_edit: None,
        }
    }
}

/// A `ListState` with the app's shared keyboard/scroll navigation, so overlay
/// list states don't each re-wire selection and offset handling. The item count
/// (`len`) is supplied per call because it lives on the owning state (a filtered
/// view for tags, the full vocabulary for feelings).
#[derive(Default)]
pub(crate) struct SelectableList {
    state: ListState,
}

impl SelectableList {
    pub(crate) fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub(crate) fn offset(&self) -> usize {
        self.state.offset()
    }

    pub(crate) fn set_offset(&mut self, offset: usize) {
        *self.state.offset_mut() = offset;
    }

    pub(crate) fn normalize(&mut self, len: usize) {
        normalize_list_state(&mut self.state, len);
    }

    pub(crate) fn select(&mut self, index: usize, len: usize) {
        if index < len {
            self.state.select(Some(index));
        }
    }

    pub(crate) fn move_by(&mut self, len: usize, delta: isize) {
        move_list_selection(&mut self.state, len, delta);
    }

    pub(crate) fn scroll_by(&mut self, delta: i16, len: usize, viewport_height: u16) {
        scroll_list_offset(&mut self.state, delta, len, viewport_height);
    }

    pub(crate) fn ensure_visible(&mut self, len: usize, viewport_height: u16) {
        ensure_selected_visible(&mut self.state, len, viewport_height);
    }
}

/// Keyboard/scroll navigation shared by the overlay list states. An implementor
/// exposes its [`SelectableList`] and current item count; the navigation methods
/// come for free, so `EditMetadataState` and `EditFeelingState` don't each re-forward
/// them with their own length source.
pub(crate) trait ListNav {
    fn list(&self) -> &SelectableList;
    fn list_mut(&mut self) -> &mut SelectableList;
    fn item_count(&self) -> usize;

    fn selected_index(&self) -> Option<usize> {
        self.list().selected()
    }

    fn offset(&self) -> usize {
        self.list().offset()
    }

    fn normalize_list_state(&mut self) {
        let len = self.item_count();
        self.list_mut().normalize(len);
    }

    fn select_index(&mut self, index: usize) {
        let len = self.item_count();
        self.list_mut().select(index, len);
    }

    fn move_up(&mut self) {
        let len = self.item_count();
        self.list_mut().move_by(len, -1);
    }

    fn move_down(&mut self) {
        let len = self.item_count();
        self.list_mut().move_by(len, 1);
    }

    fn scroll_by(&mut self, delta: i16, viewport_height: u16) {
        let len = self.item_count();
        self.list_mut().scroll_by(delta, len, viewport_height);
    }

    fn ensure_selected_visible(&mut self, viewport_height: u16) {
        let len = self.item_count();
        self.list_mut().ensure_visible(len, viewport_height);
    }
}

/// Which part of the metadata edit dialog has keyboard focus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditMetadataFocus {
    #[default]
    List,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataKind {
    Tags,
    People,
    Activities,
}

impl MetadataKind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            MetadataKind::Tags => "Tags",
            MetadataKind::People => "People",
            MetadataKind::Activities => "Activities",
        }
    }

    pub(crate) fn value_name(self) -> &'static str {
        match self {
            MetadataKind::Tags => "tag",
            MetadataKind::People => "person",
            MetadataKind::Activities => "activity",
        }
    }

    pub(crate) fn search_prefix(self) -> &'static str {
        match self {
            MetadataKind::Tags => "tags",
            MetadataKind::People => "people",
            MetadataKind::Activities => "activities",
        }
    }
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
    /// Values currently selected for the entry (lowercased for look-up).
    pub(crate) selected: Vec<String>,
    /// Stateful list selection and scroll offset.
    pub(crate) list: SelectableList,
    /// Text input for filtering values and adding new ones.
    pub(crate) input: String,
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
        let mut state = Self {
            kind,
            all_values,
            active_len,
            filtered,
            selected,
            list: SelectableList::default(),
            input: String::new(),
            focus: EditMetadataFocus::List,
        };
        state.normalize_list_state();
        state
    }

    pub(crate) fn rebuild_filter(&mut self) {
        let query = self.input.to_lowercase();
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
            let tag = self.all_values[tag_idx].0.to_lowercase();
            if let Some(pos) = self.selected.iter().position(|t| t == &tag) {
                self.selected.remove(pos);
            } else {
                self.selected.push(tag);
            }
        }
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
    /// The canonical vocabulary, borrowed from `journal_core::feelings::FEELING_GROUPS`.
    pub(crate) groups: &'static [FeelingGroup],
    /// Whether each group is expanded, parallel to `groups`. Groups start collapsed.
    pub(crate) expanded: Vec<bool>,
    /// Feelings currently selected for the entry (lowercased).
    pub(crate) selected: Vec<String>,
    /// Stateful selection and scroll offset over the *visible* rows.
    pub(crate) list: SelectableList,
    /// Text filtering the vocabulary; empty shows the grouped view.
    pub(crate) input: String,
    /// Whether keyboard events go to the list or the search input.
    pub(crate) focus: EditMetadataFocus,
}

impl EditFeelingState {
    pub(crate) fn new(groups: &'static [FeelingGroup], selected: Vec<String>) -> Self {
        let mut state = Self {
            expanded: vec![false; groups.len()],
            groups,
            selected,
            list: SelectableList::default(),
            input: String::new(),
            focus: EditMetadataFocus::List,
        };
        state.normalize_list_state();
        state.select_index(0);
        state
    }

    /// Whether a search query is narrowing the list.
    pub(crate) fn is_filtering(&self) -> bool {
        !self.input.trim().is_empty()
    }

    /// The rows currently shown. With no query: every header, plus the feelings of
    /// expanded groups. While filtering: a flat list of matching feelings (no
    /// headers), so a match is reachable without opening its group first.
    pub(crate) fn visible_rows(&self) -> Vec<FeelingRow> {
        let mut rows = Vec::new();
        if self.is_filtering() {
            let query = self.input.trim().to_lowercase();
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
            let query = self.input.trim().to_lowercase();
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

    /// Whether any feeling in `group` is currently selected.
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

pub(crate) fn normalize_list_state(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }

    let selected = state.selected().unwrap_or(0).min(len - 1);
    state.select(Some(selected));
    if state.offset() >= len {
        *state.offset_mut() = len - 1;
    }
}

pub(crate) fn move_list_selection(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        state.select(None);
        return;
    }

    let selected = state.selected().unwrap_or(0);
    let next = (selected as isize + delta).clamp(0, len as isize - 1) as usize;
    state.select(Some(next));
}

pub(crate) fn scroll_list_offset(
    state: &mut ListState,
    delta: i16,
    len: usize,
    viewport_height: u16,
) {
    if len == 0 || viewport_height == 0 {
        *state.offset_mut() = 0;
        return;
    }
    // Item-index space here (`len` items, one row each), but the clamp is the same
    // shape as the pixel lists', so share it.
    *state.offset_mut() =
        crate::tui::scroll::scroll_pixels(state.offset(), delta, len, viewport_height);
}

pub(crate) fn ensure_selected_visible(state: &mut ListState, len: usize, viewport_height: u16) {
    if len == 0 || viewport_height == 0 {
        *state.offset_mut() = 0;
        return;
    }

    let Some(selected) = state.selected().map(|index| index.min(len - 1)) else {
        return;
    };
    let viewport_height = viewport_height as usize;
    let offset = state.offset();
    let max_offset = len.saturating_sub(viewport_height);
    let next_offset = if selected < offset {
        selected
    } else if selected >= offset.saturating_add(viewport_height) {
        selected.saturating_add(1).saturating_sub(viewport_height)
    } else {
        offset
    };

    *state.offset_mut() = next_offset.min(max_offset);
}

/// State for the edit-mood overlay.
pub(crate) struct EditMoodState {
    /// The mood score currently saved on the entry (None = not set).
    pub(crate) saved: Option<i8>,
    /// The score being edited (-5..=5).
    pub(crate) draft: i8,
}

/// Fullscreen image viewer overlay: the entry's images in body order and the
/// one currently shown.
pub(crate) struct ImageViewerState {
    pub(crate) assets: Vec<ImageAsset>,
    pub(crate) index: usize,
}

pub(crate) enum DeleteContext {
    Entry {
        has_body: bool,
    },
    Journal {
        name: String,
        trash_count: usize,
        delete_count: usize,
    },
}

/// The single modal overlay that can be active over the browse view. Making
/// this an enum keeps the modals mutually exclusive by construction.
#[derive(Default)]
pub(crate) enum Overlay {
    #[default]
    None,
    ConfirmDelete(DeleteContext),
    NewJournal(String),
    EditMetadata(EditMetadataState),
    EditFeelings(EditFeelingState),
    EditMood(EditMoodState),
    ImageViewer(ImageViewerState),
}

#[cfg(test)]
mod tests {
    use super::*;
    use journal_core::feelings::Feeling;

    fn tag_state(count: usize) -> EditMetadataState {
        let all_values: Vec<(String, usize)> = (0..count)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered: Vec<usize> = (0..count).collect();
        EditMetadataState::new(MetadataKind::Tags, all_values, filtered, Vec::new(), count)
    }

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
        state.input = "co".to_string();
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
        state.input = "cont".to_string(); // matches "content" only
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
        state.input = "composed".to_string(); // alias of "calm"
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
        state.input = "happy-nope".to_string();
        state.rebuild_filter();
        assert!(state.visible_rows().is_empty());

        state.input.clear();
        state.rebuild_filter();
        // Back to one row per (collapsed) group header.
        assert_eq!(state.item_count(), state.groups.len());
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
    fn filter_hides_archived_only_values_until_query_matches() {
        // One active value (index 0) and one archived-only value (index 1).
        let all_values = vec![("berlin".to_string(), 3), ("wanderlust".to_string(), 5)];
        let mut state =
            EditMetadataState::new(MetadataKind::Tags, all_values, vec![0], Vec::new(), 1);

        // With no query only the active value is offered.
        assert_eq!(state.filtered, vec![0]);

        // Typing part of the archived-only value surfaces it (so the user reuses
        // it instead of creating a near-duplicate).
        state.input = "wan".to_string();
        state.rebuild_filter();
        assert_eq!(state.filtered, vec![1]);

        // Clearing the query hides it again.
        state.input.clear();
        state.rebuild_filter();
        assert_eq!(state.filtered, vec![0]);
    }
}
