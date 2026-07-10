use super::*;

impl App {
    pub(crate) fn begin_new_journal_input(&mut self) {
        self.overlay = Overlay::NewJournal(TextInput::default());
    }

    pub(crate) fn new_journal_input(&self) -> Option<&TextInput> {
        match &self.overlay {
            Overlay::NewJournal(name) => Some(name),
            _ => None,
        }
    }

    pub(crate) fn new_journal_input_mut(&mut self) -> Option<&mut TextInput> {
        match &mut self.overlay {
            Overlay::NewJournal(name) => Some(name),
            _ => None,
        }
    }

    /// One editing path for every single-line field: forward the key to the
    /// field that owns the caret, then run its after-edit hook when the text
    /// actually changed. Lives next to [`Self::focused_text_input_mut`] so a
    /// new field is added to both in one place.
    pub(crate) fn handle_text_input_key(&mut self, key: crossterm::event::KeyEvent) {
        match &mut self.overlay {
            Overlay::NewJournal(input) => {
                input.input(key);
            }
            Overlay::EditMetadata(state) => {
                if state.input.input(key) {
                    state.rebuild_filter();
                }
            }
            Overlay::EditFeelings(state) => {
                if state.input.input(key) {
                    state.rebuild_filter();
                }
            }
            Overlay::EditLocation(state) => state.input_key(key),
            _ => self.search_input_key(key),
        }
    }

    /// The text field that currently owns the caret, if any: an overlay's
    /// focused input, or the search box while typing in it. Selection and
    /// caret commands route through here so every field shares one binding.
    pub(crate) fn focused_text_input_mut(&mut self) -> Option<&mut TextInput> {
        match &mut self.overlay {
            Overlay::NewJournal(name) => Some(name),
            Overlay::EditMetadata(state) if state.focus == EditMetadataFocus::Input => {
                Some(&mut state.input)
            }
            Overlay::EditFeelings(state) if state.focus == EditMetadataFocus::Input => {
                Some(&mut state.input)
            }
            Overlay::EditLocation(state) => match state.focus {
                EditLocationFocus::Query => Some(&mut state.query),
                EditLocationFocus::Name => Some(&mut state.name),
                EditLocationFocus::List => None,
            },
            Overlay::None if self.nav.mode == Mode::Search && self.nav.focus == Focus::Entries => {
                Some(&mut self.search.query)
            }
            _ => None,
        }
    }

    pub(crate) fn edit_metadata_state(&self) -> Option<&EditMetadataState> {
        match &self.overlay {
            Overlay::EditMetadata(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn edit_metadata_state_mut(&mut self) -> Option<&mut EditMetadataState> {
        match &mut self.overlay {
            Overlay::EditMetadata(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn edit_feeling_state(&self) -> Option<&EditFeelingState> {
        match &self.overlay {
            Overlay::EditFeelings(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn edit_feeling_state_mut(&mut self) -> Option<&mut EditFeelingState> {
        match &mut self.overlay {
            Overlay::EditFeelings(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn selected_entry_mood(&self) -> Option<i8> {
        self.resolved_selected_entry().and_then(|entry| entry.mood)
    }

    pub(crate) fn selected_entry_starred(&self) -> bool {
        self.resolved_selected_entry()
            .is_some_and(|entry| entry.starred)
    }

    /// The selected entry's location as a one-line label, if any.
    #[cfg(test)]
    pub(crate) fn selected_entry_location(&self) -> Option<String> {
        self.resolved_selected_entry()
            .and_then(|entry| entry.location.as_ref())
            .and_then(|location| location.display_label())
    }

    pub(crate) fn begin_edit_mood(&mut self) {
        let saved = self.editing_mood();
        let draft = saved.unwrap_or(0);
        self.overlay = Overlay::EditMood(EditMoodState { saved, draft });
    }

    pub(crate) fn edit_mood_state(&self) -> Option<&EditMoodState> {
        match &self.overlay {
            Overlay::EditMood(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn edit_mood_state_mut(&mut self) -> Option<&mut EditMoodState> {
        match &mut self.overlay {
            Overlay::EditMood(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn begin_confirm_delete(&mut self) {
        match self.nav.focus {
            Focus::Journals => self.begin_confirm_delete_journal(),
            // Insights never holds a delete target (its `d` stays unbound), but the
            // match must stay total.
            Focus::Entries | Focus::EntryView | Focus::Insights => {
                self.begin_confirm_delete_entry()
            }
        }
    }

    fn begin_confirm_delete_entry(&mut self) {
        let has_body = self
            .selected_entry()
            .map(|e| !e.body.trim().is_empty())
            .unwrap_or(false);
        self.overlay = Overlay::ConfirmDelete(DeleteContext::Entry { has_body });
    }

    fn begin_confirm_delete_journal(&mut self) {
        let Some(journal) = self.selected_journal() else {
            return;
        };
        let name = journal.name.clone();
        let trash_count = self
            .library
            .entries
            .iter()
            .filter(|e| e.journal == name && !e.body.trim().is_empty())
            .count();
        let delete_count = self
            .library
            .entries
            .iter()
            .filter(|e| e.journal == name && e.body.trim().is_empty())
            .count();
        self.overlay = Overlay::ConfirmDelete(DeleteContext::Journal {
            name,
            trash_count,
            delete_count,
        });
    }

    /// Open the metadata-shortcuts reference popup, if the selected entry can be
    /// acted on. The shortcuts themselves stay live whether or not it is open.
    pub(crate) fn open_metadata_menu(&mut self) {
        if self.can_act_on_selected_entry() {
            self.overlay = Overlay::MetadataMenu;
        }
    }

    pub(crate) fn has_overlay(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    pub(crate) fn close_overlay(&mut self) {
        // Cache is scoped to the entry-viewing session, not the viewer overlay
        // (see `sync_image_warm`), so reopening within the same entry stays warm.
        self.overlay = Overlay::None;
    }
}
