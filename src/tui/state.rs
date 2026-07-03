//! Focused state containers held by [`App`](super::app::App), split out so the
//! reset/lifecycle logic for each concern lives in one place.

use std::time::{Duration, Instant};

use crate::storage::SearchHit;
use ratatui::widgets::ListState;

use super::app::SearchScope;

const STATUS_DURATION: Duration = Duration::from_secs(3);

/// Vertical scroll offset for the entry preview panel.
#[derive(Default)]
pub(crate) struct ScrollState {
    pub(crate) entry_view: u16,
}

impl ScrollState {
    /// Reset the entry preview scroll.
    pub(crate) fn reset_entry_view(&mut self) {
        self.entry_view = 0;
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
    pub(crate) scope: SearchScope,
    pub(crate) hits: Vec<SearchHit>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            scope: SearchScope::AllJournals,
            hits: Vec::new(),
        }
    }
}

/// Which part of the edit-tags dialog has keyboard focus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditTagFocus {
    #[default]
    List,
    Input,
}

/// State for the edit-tags overlay.
pub(crate) struct EditTagState {
    /// All tags across every entry, sorted by usage count descending.
    pub(crate) all_tags: Vec<(String, usize)>,
    /// Indices into `all_tags` that match the current filter input.
    pub(crate) filtered: Vec<usize>,
    /// Tags currently selected for the entry (lowercased for look-up).
    pub(crate) selected: Vec<String>,
    /// Stateful list selection and scroll offset.
    pub(crate) list_state: ListState,
    /// Text input for filtering tags and adding new ones.
    pub(crate) input: String,
    /// Whether keyboard events go to the list or to the input.
    pub(crate) focus: EditTagFocus,
}

impl EditTagState {
    pub(crate) fn new(
        all_tags: Vec<(String, usize)>,
        filtered: Vec<usize>,
        selected: Vec<String>,
    ) -> Self {
        let mut state = Self {
            all_tags,
            filtered,
            selected,
            list_state: ListState::default(),
            input: String::new(),
            focus: EditTagFocus::List,
        };
        state.normalize_list_state();
        state
    }

    pub(crate) fn rebuild_filter(&mut self) {
        let query = self.input.to_lowercase();
        self.filtered = self
            .all_tags
            .iter()
            .enumerate()
            .filter(|(_, (tag, _))| tag.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        *self.list_state.offset_mut() = 0;
        self.normalize_list_state();
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    pub(crate) fn selected_tag_index(&self) -> Option<usize> {
        self.selected_index()
            .and_then(|index| self.filtered.get(index).copied())
    }

    pub(crate) fn offset(&self) -> usize {
        self.list_state.offset()
    }

    pub(crate) fn normalize_list_state(&mut self) {
        normalize_list_state(&mut self.list_state, self.filtered.len());
    }

    pub(crate) fn select_index(&mut self, index: usize) {
        if index < self.filtered.len() {
            self.list_state.select(Some(index));
        }
    }

    pub(crate) fn move_up(&mut self) {
        move_list_selection(&mut self.list_state, self.filtered.len(), -1);
    }

    pub(crate) fn move_down(&mut self) {
        move_list_selection(&mut self.list_state, self.filtered.len(), 1);
    }

    pub(crate) fn scroll_by(&mut self, delta: i16, viewport_height: u16) {
        scroll_list_offset(
            &mut self.list_state,
            delta,
            self.filtered.len(),
            viewport_height,
        );
    }

    pub(crate) fn ensure_selected_visible(&mut self, viewport_height: u16) {
        ensure_selected_visible(&mut self.list_state, self.filtered.len(), viewport_height);
    }

    pub(crate) fn toggle_selected(&mut self) {
        if let Some(tag_idx) = self.selected_tag_index() {
            let tag = self.all_tags[tag_idx].0.to_lowercase();
            if let Some(pos) = self.selected.iter().position(|t| t == &tag) {
                self.selected.remove(pos);
            } else {
                self.selected.push(tag);
            }
        }
    }
}

/// State for the edit-feelings overlay.
pub(crate) struct EditFeelingState {
    /// Fixed feelings vocabulary in display order.
    pub(crate) all_feelings: Vec<String>,
    /// Feelings currently selected for the entry.
    pub(crate) selected: Vec<String>,
    /// Stateful list selection and scroll offset.
    pub(crate) list_state: ListState,
}

impl EditFeelingState {
    pub(crate) fn new(all_feelings: Vec<String>, selected: Vec<String>) -> Self {
        let mut state = Self {
            all_feelings,
            selected,
            list_state: ListState::default(),
        };
        state.normalize_list_state();
        state
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    pub(crate) fn offset(&self) -> usize {
        self.list_state.offset()
    }

    pub(crate) fn normalize_list_state(&mut self) {
        normalize_list_state(&mut self.list_state, self.all_feelings.len());
    }

    pub(crate) fn select_index(&mut self, index: usize) {
        if index < self.all_feelings.len() {
            self.list_state.select(Some(index));
        }
    }

    pub(crate) fn move_up(&mut self) {
        move_list_selection(&mut self.list_state, self.all_feelings.len(), -1);
    }

    pub(crate) fn move_down(&mut self) {
        move_list_selection(&mut self.list_state, self.all_feelings.len(), 1);
    }

    pub(crate) fn scroll_by(&mut self, delta: i16, viewport_height: u16) {
        scroll_list_offset(
            &mut self.list_state,
            delta,
            self.all_feelings.len(),
            viewport_height,
        );
    }

    pub(crate) fn ensure_selected_visible(&mut self, viewport_height: u16) {
        ensure_selected_visible(
            &mut self.list_state,
            self.all_feelings.len(),
            viewport_height,
        );
    }

    pub(crate) fn toggle_selected(&mut self) {
        if let Some(index) = self.selected_index() {
            let feeling = self.all_feelings[index].clone();
            if let Some(pos) = self.selected.iter().position(|v| v == &feeling) {
                self.selected.remove(pos);
            } else {
                self.selected.push(feeling);
            }
        }
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

    let max_offset = len.saturating_sub(viewport_height as usize);
    let offset = if delta < 0 {
        state.offset().saturating_sub(delta.unsigned_abs() as usize)
    } else {
        state.offset().saturating_add(delta as usize)
    };
    *state.offset_mut() = offset.min(max_offset);
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

/// The single modal overlay that can be active over the browse view. Making
/// this an enum keeps the modals mutually exclusive by construction.
#[derive(Default)]
pub(crate) enum Overlay {
    #[default]
    None,
    ConfirmDelete,
    NewJournal(String),
    EditTags(EditTagState),
    EditFeelings(EditFeelingState),
    EditMood(EditMoodState),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_state(count: usize) -> EditTagState {
        let all_tags: Vec<(String, usize)> = (0..count)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered: Vec<usize> = (0..count).collect();
        EditTagState::new(all_tags, filtered, Vec::new())
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
        *state.list_state.offset_mut() = 5;

        state.move_up();
        state.ensure_selected_visible(4);

        assert_eq!(state.selected_index(), Some(4));
        assert_eq!(state.offset(), 4);
    }
}
