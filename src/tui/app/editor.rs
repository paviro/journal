use super::{App, Focus};
use crate::tui::editor_state::{EditorTarget, EntryEditor};
use crate::tui::state::{MetadataKind, ToastVariant};
use notema_domain::Metadata;

impl App {
    pub(crate) fn selected_entry_edit_warning(&self) -> Option<String> {
        self.resolved_selected_entry()?.warning.clone()
    }

    pub(crate) fn allow_selected_entry_edit(&mut self) -> bool {
        let Some(warning) = self.selected_entry_edit_warning() else {
            return true;
        };
        self.toast(
            ToastVariant::Error,
            format!("Can't edit this entry. {warning}. Repair its +++ metadata block first."),
        );
        false
    }

    /// Open the internal editor on the selected entry, replacing the entry-view
    /// content in place. Refuses locked encrypted entries. Stays in the current
    /// column layout (fullscreen is a separate toggle).
    pub(crate) fn open_editor_for_selected(&mut self) -> crate::AppResult<()> {
        let Some(target) = self.selected_entry_target() else {
            return Ok(());
        };
        if target.locked {
            self.toast(ToastVariant::Error, "Encryption identity not available");
            return Ok(());
        }
        let Some(journal) = self
            .resolved_selected_entry()
            .map(|entry| entry.journal.clone())
        else {
            return Ok(());
        };
        let (entry, revision) = self
            .store
            .read_entry_with_revision(&journal, &target.path)?;
        self.replace_entry_from_disk(entry.clone());
        if !self.allow_selected_entry_edit() {
            return Ok(());
        }
        self.editor = Some(EntryEditor::for_existing(
            journal,
            target.path,
            target.title,
            revision,
            &entry.body,
            entry.metadata_bundle(),
        ));
        self.nav.focus = Focus::Reader;
        if self.config.editor.start_fullscreen {
            self.nav.reader_fullscreen = true;
        }
        Ok(())
    }

    /// Open the internal editor on a blank buffer for a new entry in the selected
    /// journal. Opens in-pane in the entry-view column (fullscreen stays a toggle);
    /// [`show_journal_insights`](Self::show_journal_insights) yields
    /// that column to the editor even with no entry selected.
    pub(crate) fn open_editor_for_new(&mut self) {
        let Some(journal) = self.selected_journal().map(|journal| journal.name.clone()) else {
            self.toast(ToastVariant::Info, "Create a journal first with n");
            return;
        };
        self.editor = Some(EntryEditor::for_new(journal));
        self.nav.focus = Focus::Reader;
        if self.config.editor.start_fullscreen {
            self.nav.reader_fullscreen = true;
        }
    }

    /// Enter one-shot compose mode: open a fullscreen new-entry editor for
    /// `journal`, seeded with any metadata from the `notema log` flags, and mark
    /// the app to quit once that entry is saved or discarded. No journal need be
    /// selected.
    pub(crate) fn begin_compose(&mut self, journal: String, metadata: Metadata) {
        let mut editor = EntryEditor::for_new(journal);
        editor.metadata = metadata.clone();
        editor.original_metadata = metadata;
        self.editor = Some(editor);
        self.nav.reader_fullscreen = true;
        self.nav.focus = Focus::Reader;
        self.compose = true;
        // Theme by the compose target, not the journal restored from state.
        self.apply_effective_theme();
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
            self.nav.reader_fullscreen = false;
            self.nav.focus = if self.has_selected_entry_target() {
                Focus::Reader
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
    pub(crate) fn editing_location(&self) -> Option<notema_domain::Location> {
        match &self.editor {
            Some(editor) => editor.metadata.location.clone(),
            None => self
                .resolved_selected_entry()
                .and_then(|entry| entry.location.clone()),
        }
    }

    /// Write a location edit into the open editor's buffer (applied to the
    /// entry only on save). No-op when the editor is closed.
    pub(crate) fn set_editor_location(&mut self, location: Option<notema_domain::Location>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let cleared = location.is_none();
        editor.metadata.location = location;
        self.toast(
            ToastVariant::Success,
            if cleared {
                "Location cleared"
            } else {
                "Location set"
            },
        );
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
        self.toast(ToastVariant::Success, format!("{} set", kind.title()));
    }

    pub(crate) fn set_editor_feelings(&mut self, feelings: &[String]) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.metadata.feelings = feelings.to_vec();
        self.toast(ToastVariant::Success, "Feelings set");
    }

    pub(crate) fn set_editor_mood(&mut self, mood: Option<i8>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.metadata.mood = mood;
        self.toast(
            ToastVariant::Success,
            if mood.is_some() {
                "Mood set"
            } else {
                "Mood cleared"
            },
        );
    }
}
