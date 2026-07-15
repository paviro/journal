use crate::tui::{
    app::{AppModel, Focus, Mode},
    features::{location::EditLocationFocus, metadata::EditMetadataFocus},
    state::{DeleteContext, Overlay},
    text_input::TextInput,
};

impl AppModel {
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

    /// Insert a pasted block into whichever single-line field owns the caret,
    /// running the same after-edit hook as typing. Mirrors
    /// [`Self::handle_text_input_key`] so both stay in sync.
    pub(crate) fn handle_text_input_paste(&mut self, text: &str) {
        match &mut self.overlay {
            Overlay::NewJournal(input) => {
                input.paste_str(text);
            }
            Overlay::EditMetadata(state) => {
                if state.input.paste_str(text) {
                    state.rebuild_filter();
                }
            }
            Overlay::EditFeelings(state) => {
                if state.input.paste_str(text) {
                    state.rebuild_filter();
                }
            }
            Overlay::EditLocation(state) => state.input_paste(text),
            _ => self.search_input_paste(text),
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

    pub(crate) fn begin_confirm_delete(&mut self) {
        match self.nav.focus {
            Focus::Journals => self.begin_confirm_delete_journal(),
            // Insights never holds a delete target (its `d` stays unbound), but the
            // match must stay total.
            Focus::Entries | Focus::Reader | Focus::Insights => self.begin_confirm_delete_entry(),
        }
    }

    fn begin_confirm_delete_entry(&mut self) {
        let has_body = self
            .selected_entry()
            .map(|e| !e.body.trim().is_empty())
            .unwrap_or(false);
        // Default the selection to Cancel so a stray Enter never deletes.
        self.overlay = Overlay::ConfirmDelete(DeleteContext::Entry { has_body }, false);
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
        self.overlay = Overlay::ConfirmDelete(
            DeleteContext::Journal {
                name,
                trash_count,
                delete_count,
            },
            false,
        );
    }

    /// Open the metadata-shortcuts reference popup, if the selected entry can be
    /// acted on. The shortcuts themselves stay live whether or not it is open.
    pub(crate) fn open_metadata_menu(&mut self) {
        if self.can_act_on_selected_entry() {
            self.overlay = Overlay::MetadataMenu;
        }
    }

    /// Open the global keyboard-shortcut cheatsheet.
    pub(crate) fn open_help(&mut self) {
        self.overlay = Overlay::Help { scroll: 0 };
    }

    pub(crate) fn has_overlay(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    pub(crate) fn close_overlay(&mut self) {
        // Cache is scoped to the entry-viewing session, not the viewer overlay
        // (see `prepare_image_warm`), so reopening within the same entry stays warm.
        self.overlay = Overlay::None;
    }
}
