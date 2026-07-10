//! Focused state containers held by [`App`](super::app::App), split out so the
//! reset/lifecycle logic for each concern lives in one place.

use std::time::{Duration, Instant};

use journal_context_provider::{DeviceFix, GeocodeHit};
use journal_core::feelings::FeelingGroup;
use journal_core::{Location, SearchHit};
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
    /// Values currently selected for the entry. Original casing is preserved;
    /// membership and dedup are compared case-insensitively.
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
        let input = self.input.trim();
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

/// Which field of the location dialog has keyboard focus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditLocationFocus {
    #[default]
    Query,
    Name,
    List,
}

/// Progress of an on-demand geocode lookup, surfaced as the dialog's status line.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) enum LocationResolveStatus {
    #[default]
    Idle,
    Resolving,
    Resolved,
    NoMatch,
    Error(String),
}

/// A recent/most-common existing location offered as a preset. `label` is its
/// display line; `location` is copied wholesale when the preset is chosen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocationPreset {
    pub(crate) label: String,
    pub(crate) location: Location,
}

/// State for the location overlay: two text fields (a free-form address or
/// coordinate query, and a place label) plus a list that shows geocode candidate
/// matches once a lookup returns, or recent/common presets otherwise. Geocoding
/// is dispatched to a background worker; `pending_request_id` guards against a
/// stale reply landing after a newer request.
pub(crate) struct EditLocationState {
    /// Free-form address, or `"lat, lon"` coordinates.
    pub(crate) query: String,
    /// The place's human label — maps to [`Location::name`].
    pub(crate) name: String,
    /// Coordinates + names resolved from the query, a candidate, or a preset.
    pub(crate) resolved: Option<Location>,
    /// Recent-then-common existing locations, shown when no lookup is active.
    pub(crate) presets: Vec<LocationPreset>,
    /// Candidate matches from the last forward geocode; while non-empty they
    /// replace the presets in the list.
    pub(crate) candidates: Vec<GeocodeHit>,
    pub(crate) list: SelectableList,
    pub(crate) focus: EditLocationFocus,
    pub(crate) status: LocationResolveStatus,
    /// Whether the current query text has already been looked up. When set, Enter
    /// in the address field commits instead of re-querying; editing the query
    /// clears it (and the shown result), flipping Enter back to "look up".
    pub(crate) query_looked_up: bool,
    /// Id assigned to the next dispatched request; the in-flight one is kept in
    /// `pending_request_id` so a late reply for an older query can be dropped.
    pub(crate) next_request_id: u64,
    pub(crate) pending_request_id: Option<u64>,
}

impl EditLocationState {
    pub(crate) fn new(current: Option<Location>, presets: Vec<LocationPreset>) -> Self {
        let name = current
            .as_ref()
            .and_then(|loc| loc.name.clone())
            .unwrap_or_default();
        let query = current.as_ref().map(query_seed).unwrap_or_default();
        // Treat the seeded query as already looked up only when the stored location
        // carries address detail — a bare coordinate pair still needs a lookup, so
        // Enter should resolve it rather than save.
        let query_looked_up = !query.is_empty()
            && current
                .as_ref()
                .is_some_and(|location| location.has_named_parts());
        let mut state = Self {
            query_looked_up,
            query,
            name,
            resolved: current,
            presets,
            candidates: Vec::new(),
            list: SelectableList::default(),
            focus: EditLocationFocus::Query,
            status: LocationResolveStatus::Idle,
            next_request_id: 0,
            pending_request_id: None,
        };
        state.normalize_list_state();
        state
    }

    /// The list shows geocode candidates once a lookup returns them, else presets.
    pub(crate) fn showing_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    /// The labels currently shown in the list — candidate matches (our parsed
    /// label, falling back to the raw display name) or preset labels.
    pub(crate) fn list_labels(&self) -> Vec<String> {
        if self.showing_candidates() {
            self.candidates
                .iter()
                .map(|hit| {
                    hit.location
                        .display_label()
                        .unwrap_or_else(|| hit.display_name.clone())
                })
                .collect()
        } else {
            self.presets
                .iter()
                .map(|preset| preset.label.clone())
                .collect()
        }
    }

    fn row_count(&self) -> usize {
        if self.showing_candidates() {
            self.candidates.len()
        } else {
            self.presets.len()
        }
    }

    /// Cycle focus Query → Name → List → Query. The list is skipped when it's
    /// empty, so Tab just toggles between the two input fields.
    pub(crate) fn switch_focus(&mut self) {
        let has_list = self.row_count() > 0;
        self.focus = match self.focus {
            EditLocationFocus::Query => EditLocationFocus::Name,
            EditLocationFocus::Name if has_list => EditLocationFocus::List,
            EditLocationFocus::Name => EditLocationFocus::Query,
            EditLocationFocus::List => EditLocationFocus::Query,
        };
    }

    /// Type a char into whichever text field has focus (inert on the list).
    pub(crate) fn input_char(&mut self, ch: char) {
        match self.focus {
            EditLocationFocus::Query => {
                self.query.push(ch);
                self.invalidate_lookup();
            }
            EditLocationFocus::Name => self.name.push(ch),
            EditLocationFocus::List => {}
        }
    }

    /// Backspace the focused text field (inert on the list).
    pub(crate) fn backspace(&mut self) {
        match self.focus {
            EditLocationFocus::Query => {
                self.query.pop();
                self.invalidate_lookup();
            }
            EditLocationFocus::Name => {
                self.name.pop();
            }
            EditLocationFocus::List => {}
        }
    }

    /// Editing the query invalidates the last lookup: drop the resolved result and
    /// candidate matches, clear the status preview, and flip Enter back to "look
    /// up". The typed name is untouched.
    fn invalidate_lookup(&mut self) {
        self.query_looked_up = false;
        self.resolved = None;
        self.candidates.clear();
        self.status = LocationResolveStatus::Idle;
        self.normalize_list_state();
    }

    /// Fold a finished forward-geocode reply into the dialog: replace the
    /// candidate list, move focus onto it when there are matches, and update the
    /// status line.
    pub(crate) fn apply_candidates(&mut self, hits: Vec<GeocodeHit>) {
        self.candidates = hits;
        self.list.set_offset(0);
        if self.candidates.is_empty() {
            self.status = LocationResolveStatus::NoMatch;
            // Don't leave focus stranded on a list that just emptied.
            if self.focus == EditLocationFocus::List && self.presets.is_empty() {
                self.focus = EditLocationFocus::Query;
            }
        } else {
            self.status = LocationResolveStatus::Resolved;
            self.focus = EditLocationFocus::List;
            self.select_index(0);
        }
        self.normalize_list_state();
    }

    /// Adopt a freshly grabbed device fix: mirror the coordinates into the query
    /// field and make them — with their accuracy and provider — the resolved,
    /// saveable value. Any stale address fields are dropped; the reverse-geocoded
    /// names for this new spot arrive next, via
    /// [`apply_reverse`](Self::apply_reverse).
    pub(crate) fn seed_device_fix(&mut self, fix: &DeviceFix) {
        self.query = format!("{}, {}", fix.latitude, fix.longitude);
        self.resolved = Some(Location {
            latitude: Some(fix.latitude),
            longitude: Some(fix.longitude),
            accuracy_m: fix.accuracy_m,
            source: Some(fix.source.to_string()),
            ..Location::default()
        });
    }

    /// Fold a finished reverse-geocode reply into the dialog: enrich the resolved
    /// coordinates with the returned names (keeping the user's coordinates). The
    /// coordinates are now looked up, so Enter in the address field will save.
    pub(crate) fn apply_reverse(&mut self, hit: Option<GeocodeHit>) {
        match hit {
            Some(hit) => {
                let mut location = hit.location;
                // Keep the coordinates the user entered or the device grabbed,
                // along with that grab's accuracy and provider.
                if let Some(resolved) = &self.resolved {
                    location.latitude = resolved.latitude.or(location.latitude);
                    location.longitude = resolved.longitude.or(location.longitude);
                    location.accuracy_m = resolved.accuracy_m.or(location.accuracy_m);
                    location.source = resolved.source.clone().or(location.source);
                }
                // A POI/venue name fills the name field unless the user typed one,
                // so composed() (which takes the name from that field) keeps it.
                if self.name.trim().is_empty()
                    && let Some(name) = &location.name
                {
                    self.name = name.clone();
                }
                self.resolved = Some(location);
                self.status = LocationResolveStatus::Resolved;
            }
            // The coordinates the user entered are still resolved and saveable;
            // only the name lookup came back empty.
            None => self.status = LocationResolveStatus::NoMatch,
        }
        self.query_looked_up = true;
    }

    /// Adopt the highlighted preset/candidate as the resolved location, seeding
    /// the query field from it. Its name (a preset's label, or a POI/venue name
    /// from geocoding) fills the name field only when the user hasn't typed one,
    /// so a deliberate custom name is never clobbered.
    pub(crate) fn select_row(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let location = if self.showing_candidates() {
            self.candidates.get(index).map(|hit| hit.location.clone())
        } else {
            self.presets
                .get(index)
                .map(|preset| preset.location.clone())
        };
        if let Some(location) = location {
            if self.name.trim().is_empty()
                && let Some(name) = &location.name
            {
                self.name = name.clone();
            }
            self.query = query_seed(&location);
            self.resolved = Some(location);
            self.status = LocationResolveStatus::Resolved;
        }
    }

    /// The location to persist: the resolved coordinates/address with the typed
    /// name applied. `None` when nothing is set (clears the entry's location).
    pub(crate) fn composed(&self) -> Option<Location> {
        let mut location = self.resolved.clone().unwrap_or_default();
        let name = self.name.trim();
        location.name = (!name.is_empty()).then(|| name.to_string());
        (!location.is_empty()).then_some(location)
    }
}

impl ListNav for EditLocationState {
    fn list(&self) -> &SelectableList {
        &self.list
    }

    fn list_mut(&mut self) -> &mut SelectableList {
        &mut self.list
    }

    fn item_count(&self) -> usize {
        self.row_count()
    }
}

/// Seed the address/coords field from a location: its coordinates when known (so
/// it stays re-resolvable). Empty otherwise — the place name lives in its own
/// field and must not be echoed here.
fn query_seed(location: &Location) -> String {
    match (location.latitude, location.longitude) {
        (Some(lat), Some(lon)) => format!("{lat}, {lon}"),
        _ => String::new(),
    }
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
    /// Reference popup listing the metadata shortcut keys. The keys work whether or
    /// not it is shown, so this only aids discovery.
    MetadataMenu,
    ConfirmDelete(DeleteContext),
    NewJournal(String),
    EditMetadata(EditMetadataState),
    EditFeelings(EditFeelingState),
    EditMood(EditMoodState),
    // Boxed: this state is much larger than the other variants (candidate/preset
    // lists), so keeping it behind a pointer keeps `Overlay` small.
    EditLocation(Box<EditLocationState>),
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

    #[test]
    fn add_from_input_shows_the_new_value_immediately() {
        // One active value; an archived-only value sits after the active boundary.
        let all_values = vec![("berlin".to_string(), 3), ("wanderlust".to_string(), 5)];
        let mut state =
            EditMetadataState::new(MetadataKind::Tags, all_values, vec![0], Vec::new(), 1);

        state.input = "hiking".to_string();
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

    fn hit(name: &str, lat: f64, lon: f64) -> GeocodeHit {
        GeocodeHit {
            display_name: name.to_string(),
            location: Location {
                city: Some(name.to_string()),
                latitude: Some(lat),
                longitude: Some(lon),
                ..Location::default()
            },
        }
    }

    fn device_fix(lat: f64, lon: f64) -> DeviceFix {
        DeviceFix {
            latitude: lat,
            longitude: lon,
            accuracy_m: Some(12.0),
            source: journal_context_provider::DeviceLocationSource::CoreLocation,
        }
    }

    #[test]
    fn location_composes_name_only_and_clears_when_empty() {
        let mut state = EditLocationState::new(None, Vec::new());
        assert_eq!(state.composed(), None);

        state.name = "Home".to_string();
        let composed = state.composed().unwrap();
        assert_eq!(composed.name.as_deref(), Some("Home"));
        assert!(composed.latitude.is_none());
    }

    #[test]
    fn location_selecting_a_candidate_fills_resolved_and_saves_coordinates() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.apply_candidates(vec![hit("Paris", 48.85, 2.35)]);

        // A match list takes focus and reports resolved.
        assert!(state.showing_candidates());
        assert_eq!(state.focus, EditLocationFocus::List);
        assert_eq!(state.status, LocationResolveStatus::Resolved);

        state.select_row();
        let composed = state.composed().unwrap();
        assert_eq!(composed.latitude, Some(48.85));
        assert_eq!(composed.city.as_deref(), Some("Paris"));
    }

    #[test]
    fn location_picking_a_geocoded_address_keeps_the_typed_name() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.name = "Home".to_string();
        // A plain address candidate carries a road/city but no POI name.
        let candidate = GeocodeHit {
            display_name: "Bahnhofstraße 1, Berlin".to_string(),
            location: Location {
                road: Some("Bahnhofstraße".to_string()),
                house_number: Some("1".to_string()),
                city: Some("Berlin".to_string()),
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Location::default()
            },
        };
        state.apply_candidates(vec![candidate]);
        state.select_row();

        let composed = state.composed().unwrap();
        assert_eq!(
            composed.name.as_deref(),
            Some("Home"),
            "typed name survives"
        );
        assert_eq!(composed.road.as_deref(), Some("Bahnhofstraße"));
        assert_eq!(composed.city.as_deref(), Some("Berlin"));
        assert_eq!(composed.latitude, Some(52.52));
    }

    #[test]
    fn location_picking_a_poi_fills_an_empty_name() {
        // No name typed: a candidate's POI name (a shop/venue) fills it.
        let mut state = EditLocationState::new(None, Vec::new());
        let candidate = GeocodeHit {
            display_name: "Corner Cafe, Bahnhofstraße 1, Berlin".to_string(),
            location: Location {
                name: Some("Corner Cafe".to_string()),
                road: Some("Bahnhofstraße".to_string()),
                house_number: Some("1".to_string()),
                city: Some("Berlin".to_string()),
                ..Location::default()
            },
        };
        state.apply_candidates(vec![candidate]);
        state.select_row();

        assert_eq!(
            state.composed().unwrap().name.as_deref(),
            Some("Corner Cafe")
        );
    }

    #[test]
    fn location_no_candidates_reports_no_match_and_keeps_presets() {
        let preset = LocationPreset {
            label: "Berlin".to_string(),
            location: Location {
                city: Some("Berlin".to_string()),
                ..Location::default()
            },
        };
        let mut state = EditLocationState::new(None, vec![preset]);
        state.apply_candidates(Vec::new());

        assert!(!state.showing_candidates());
        assert_eq!(state.status, LocationResolveStatus::NoMatch);
        assert_eq!(state.item_count(), 1, "presets stay listed");
    }

    #[test]
    fn location_tab_skips_the_list_when_empty() {
        // No presets and no candidates: Tab only toggles the two input fields.
        let mut state = EditLocationState::new(None, Vec::new());
        assert_eq!(state.focus, EditLocationFocus::Query);
        state.switch_focus();
        assert_eq!(state.focus, EditLocationFocus::Name);
        state.switch_focus();
        assert_eq!(state.focus, EditLocationFocus::Query, "the list is skipped");

        // With a preset present, the list joins the cycle.
        let preset = LocationPreset {
            label: "Berlin".to_string(),
            location: Location {
                city: Some("Berlin".to_string()),
                ..Location::default()
            },
        };
        let mut state = EditLocationState::new(None, vec![preset]);
        state.switch_focus(); // Query -> Name
        state.switch_focus(); // Name -> List
        assert_eq!(state.focus, EditLocationFocus::List);
    }

    #[test]
    fn location_reverse_keeps_user_coordinates_and_adds_names() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.resolved = Some(Location {
            latitude: Some(1.0),
            longitude: Some(2.0),
            ..Location::default()
        });
        state.apply_reverse(Some(hit("Town", 9.9, 9.9)));

        let resolved = state.resolved.unwrap();
        assert_eq!(resolved.latitude, Some(1.0));
        assert_eq!(resolved.longitude, Some(2.0));
        assert_eq!(resolved.city.as_deref(), Some("Town"));
    }

    #[test]
    fn location_reverse_poi_name_survives_into_composed() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.seed_device_fix(&device_fix(52.52, 13.405));
        // A reverse hit that carries a POI/venue name, user hasn't typed one.
        let poi = GeocodeHit {
            display_name: "Corner Cafe".to_string(),
            location: Location {
                name: Some("Corner Cafe".to_string()),
                city: Some("Berlin".to_string()),
                latitude: Some(9.9),
                longitude: Some(9.9),
                ..Location::default()
            },
        };
        state.apply_reverse(Some(poi));

        let composed = state.composed().unwrap();
        assert_eq!(
            composed.name.as_deref(),
            Some("Corner Cafe"),
            "POI name saved"
        );
        assert_eq!(composed.latitude, Some(52.52), "grabbed coordinates kept");
    }

    #[test]
    fn location_device_grab_seeds_coords_then_reverse_names_them() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.name = "Desk".to_string();
        // Simulate a stale prior address to prove the grab starts clean.
        state.resolved = Some(Location {
            city: Some("Elsewhere".to_string()),
            ..Location::default()
        });

        // A grabbed fix mirrors into the query field and becomes the resolved,
        // saveable coordinates (with accuracy + provider) — stale address dropped.
        state.seed_device_fix(&device_fix(52.52, 13.405));
        assert_eq!(state.query, "52.52, 13.405");
        let resolved = state.resolved.clone().unwrap();
        assert_eq!(resolved.latitude, Some(52.52));
        assert_eq!(resolved.longitude, Some(13.405));
        assert_eq!(resolved.accuracy_m, Some(12.0));
        assert_eq!(resolved.source.as_deref(), Some("corelocation"));
        assert!(resolved.city.is_none(), "stale address is cleared");

        // The reverse lookup then names the spot, keeping the grabbed coordinates
        // (with accuracy + provider) and the name the user had typed.
        state.apply_reverse(Some(hit("Berlin", 9.9, 9.9)));
        let composed = state.composed().unwrap();
        assert_eq!(composed.latitude, Some(52.52));
        assert_eq!(composed.longitude, Some(13.405));
        assert_eq!(composed.accuracy_m, Some(12.0), "device accuracy kept");
        assert_eq!(
            composed.source.as_deref(),
            Some("corelocation"),
            "provider kept"
        );
        assert_eq!(composed.city.as_deref(), Some("Berlin"));
        assert_eq!(composed.name.as_deref(), Some("Desk"), "typed name kept");
    }

    #[test]
    fn location_opened_with_coords_only_still_needs_a_lookup() {
        // Only coordinates stored: Enter should look up, not save.
        let coords_only = Location {
            latitude: Some(52.5),
            longitude: Some(13.4),
            ..Location::default()
        };
        let state = EditLocationState::new(Some(coords_only), Vec::new());
        assert!(!state.query.is_empty());
        assert!(!state.query_looked_up);

        // Coordinates plus address detail count as already resolved.
        let resolved = Location {
            city: Some("Berlin".to_string()),
            latitude: Some(52.5),
            longitude: Some(13.4),
            ..Location::default()
        };
        let state = EditLocationState::new(Some(resolved), Vec::new());
        assert!(state.query_looked_up);
    }

    #[test]
    fn location_query_flips_to_save_after_lookup_then_back_on_edit() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.focus = EditLocationFocus::Query;
        state.query = "52.5, 13.4".to_string();
        state.resolved = Some(Location {
            latitude: Some(52.5),
            longitude: Some(13.4),
            ..Location::default()
        });

        // A finished reverse lookup marks the query resolved (Enter would save).
        state.apply_reverse(Some(hit("Berlin", 52.5, 13.4)));
        assert!(state.query_looked_up);
        assert!(state.resolved.is_some());

        // Editing the query reverts to look-up mode and clears the shown result.
        state.input_char('5');
        assert!(!state.query_looked_up);
        assert!(state.resolved.is_none());
        assert_eq!(state.status, LocationResolveStatus::Idle);
    }

    #[test]
    fn add_from_input_reuses_existing_value_without_duplicating() {
        let mut state = tag_state(3);
        let before = state.all_values.len();

        state.input = "TAG-01".to_string();
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

        state.input = "iPhone".to_string();
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
