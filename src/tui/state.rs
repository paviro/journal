//! Focused state containers held by [`App`](super::app::App), split out so the
//! reset/lifecycle logic for each concern lives in one place.

use std::time::{Duration, Instant};

use journal_core::SearchHit;
use ratatui::widgets::ListState;

use super::app::{EditFeelingState, EditLocationState, EditMetadataState, SearchScope};
use super::image::ImageAsset;
use super::text_input::TextInput;

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
    pub(crate) query: TextInput,
    pub(crate) scope: SearchScope,
    pub(crate) hits: Vec<SearchHit>,
    /// Set when the query changed but the (expensive) hit recompute has been
    /// deferred; the event loop runs it once typing pauses (debounce).
    pub(crate) dirty: bool,
    /// Timestamp of the last search keystroke, for the debounce window.
    pub(crate) last_edit: Option<Instant>,
}

impl Default for SearchState {
    fn default() -> Self {
        let mut query = TextInput::default();
        query.set_placeholder_text("type to search");
        Self {
            query,
            scope: SearchScope::AllJournals,
            hits: Vec::new(),
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

    pub(crate) fn search_prefix(self) -> &'static str {
        match self {
            MetadataKind::Tags => "tags",
            MetadataKind::People => "people",
            MetadataKind::Activities => "activities",
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
    /// Reference popup listing the metadata shortcut keys. The keys work whether or
    /// not it is shown, so this only aids discovery.
    MetadataMenu,
    ConfirmDelete(DeleteContext),
    NewJournal(TextInput),
    EditMetadata(EditMetadataState),
    EditFeelings(EditFeelingState),
    EditMood(EditMoodState),
    // Boxed: this state is much larger than the other variants (candidate/preset
    // lists), so keeping it behind a pointer keeps `Overlay` small.
    EditLocation(Box<EditLocationState>),
    ImageViewer(ImageViewerState),
}
