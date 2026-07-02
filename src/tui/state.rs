//! Focused state containers held by [`App`](super::app::App), split out so the
//! reset/lifecycle logic for each concern lives in one place.

use std::time::{Duration, Instant};

use crate::storage::SearchHit;

use super::app::SearchScope;

const STATUS_DURATION: Duration = Duration::from_secs(3);

/// Vertical scroll offsets for the three panels.
#[derive(Default)]
pub(crate) struct ScrollState {
    pub(crate) journal: u16,
    pub(crate) entry: u16,
    pub(crate) entry_view: u16,
}

impl ScrollState {
    /// Reset every panel's scroll to the top.
    pub(crate) fn reset(&mut self) {
        self.journal = 0;
        self.reset_entry();
    }

    /// Reset the entry list and entry preview scroll, leaving the journal list.
    pub(crate) fn reset_entry(&mut self) {
        self.entry = 0;
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
#[derive(Default, Clone, Copy, PartialEq, Eq)]
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
    /// Index into `filtered` for the list cursor.
    pub(crate) cursor: usize,
    /// Scroll offset for the tag list inside the dialog.
    pub(crate) scroll: u16,
    /// Text input for filtering tags and adding new ones.
    pub(crate) input: String,
    /// Whether keyboard events go to the list or to the input.
    pub(crate) focus: EditTagFocus,
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
}
