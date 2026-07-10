use super::{App, Focus};
use crate::tui::editor_state::{EditorTarget, EntryEditor};
use crate::tui::state::MetadataKind;
use journal_core::Metadata;

impl App {
    /// Open the internal editor on the selected entry, replacing the entry-view
    /// content in place. Refuses locked encrypted entries. Stays in the current
    /// column layout (fullscreen is a separate toggle).
    pub(crate) fn open_editor_for_selected(&mut self) {
        let Some(target) = self.selected_entry_target() else {
            return;
        };
        if target.locked {
            self.set_status("Encryption identity not available");
            return;
        }
        let Some((body, metadata)) = self
            .resolved_selected_entry()
            .map(|entry| (entry.body.clone(), entry.metadata_bundle()))
        else {
            return;
        };
        self.editor = Some(EntryEditor::for_existing(
            target.path,
            target.title,
            &body,
            metadata,
        ));
        self.nav.focus = Focus::EntryView;
        if self.config.editor.start_fullscreen {
            self.nav.entry_view_fullscreen = true;
        }
    }

    /// Open the internal editor on a blank buffer for a new entry in the selected
    /// journal. Opens in-pane in the entry-view column (fullscreen stays a toggle);
    /// [`show_journal_insights_preview`](Self::show_journal_insights_preview) yields
    /// that column to the editor even with no entry selected.
    pub(crate) fn open_editor_for_new(&mut self) {
        let Some(journal) = self.selected_journal().map(|journal| journal.name.clone()) else {
            self.set_status("Create a journal first with n");
            return;
        };
        self.editor = Some(EntryEditor::for_new(journal));
        self.nav.focus = Focus::EntryView;
        if self.config.editor.start_fullscreen {
            self.nav.entry_view_fullscreen = true;
        }
    }

    /// Enter one-shot compose mode: open a fullscreen new-entry editor for
    /// `journal`, seeded with any metadata from the `journal log` flags, and mark
    /// the app to quit once that entry is saved or discarded. No journal need be
    /// selected.
    pub(crate) fn begin_compose(&mut self, journal: String, metadata: Metadata) {
        let mut editor = EntryEditor::for_new(journal);
        editor.metadata = metadata.clone();
        editor.original_metadata = metadata;
        self.editor = Some(editor);
        self.nav.entry_view_fullscreen = true;
        self.nav.focus = Focus::EntryView;
        self.compose = true;
    }

    /// Discard the open editor without saving. A cancelled new-entry compose has
    /// nothing to show, so it undoes the forced fullscreen and drops back to the
    /// entry list; an existing entry stays in the viewer.
    pub(crate) fn cancel_editor(&mut self) {
        let was_new = matches!(
            self.editor.as_ref().map(|editor| &editor.target),
            Some(EditorTarget::New { .. })
        );
        self.editor = None;
        if was_new {
            self.nav.entry_view_fullscreen = false;
            self.nav.focus = if self.has_selected_entry_target() {
                Focus::EntryView
            } else {
                Focus::Entries
            };
        }
    }

    /// Metadata values to seed an edit dialog with: the editor's buffer when it is
    /// open, otherwise the selected entry's.
    pub(crate) fn editing_metadata_values(&self, kind: MetadataKind) -> Vec<String> {
        match &self.editor {
            Some(editor) => match kind {
                MetadataKind::Tags => editor.metadata.tags.clone(),
                MetadataKind::People => editor.metadata.people.clone(),
                MetadataKind::Activities => editor.metadata.activities.clone(),
            },
            None => self.selected_entry_metadata(kind),
        }
    }

    pub(crate) fn editing_feelings(&self) -> Vec<String> {
        match &self.editor {
            Some(editor) => editor.metadata.feelings.clone(),
            None => self.selected_entry_feelings(),
        }
    }

    pub(crate) fn editing_mood(&self) -> Option<i8> {
        match &self.editor {
            Some(editor) => editor.metadata.mood,
            None => self.selected_entry_mood(),
        }
    }

    /// The location the location dialog should open with: the open editor's
    /// draft (empty for a new entry), else the selected entry's.
    pub(crate) fn editing_location(&self) -> Option<journal_core::Location> {
        match &self.editor {
            Some(editor) => editor.metadata.location.clone(),
            None => self
                .resolved_selected_entry()
                .and_then(|entry| entry.location.clone()),
        }
    }

    /// Write a location edit into the open editor's buffer (applied to the
    /// entry only on save). No-op when the editor is closed.
    pub(crate) fn set_editor_location(&mut self, location: Option<journal_core::Location>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let cleared = location.is_none();
        editor.metadata.location = location;
        self.set_status(if cleared {
            "Location cleared"
        } else {
            "Location set"
        });
        // Fetch weather/air/celestial in the background now, so it's ready to
        // attach on save (or the save waits briefly on it). A cleared/coordless
        // location abandons any in-flight fetch.
        self.spawn_editor_environment();
    }

    /// Write a metadata edit into the open editor's buffer (applied to the entry
    /// only on save). No-op when the editor is closed.
    pub(crate) fn set_editor_metadata(&mut self, kind: MetadataKind, values: &[String]) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match kind {
            MetadataKind::Tags => editor.metadata.tags = values.to_vec(),
            MetadataKind::People => editor.metadata.people = values.to_vec(),
            MetadataKind::Activities => editor.metadata.activities = values.to_vec(),
        }
        self.set_status(format!("{} set", kind.title()));
    }

    pub(crate) fn set_editor_feelings(&mut self, feelings: &[String]) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.metadata.feelings = feelings.to_vec();
        self.set_status("Feelings set");
    }

    pub(crate) fn set_editor_mood(&mut self, mood: Option<i8>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.metadata.mood = mood;
        self.set_status(if mood.is_some() {
            "Mood set"
        } else {
            "Mood cleared"
        });
    }
}
