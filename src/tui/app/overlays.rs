use super::*;

use crate::tui::state::{ListNav, ThemePickerState};

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
        if self.editor.is_none() && !self.allow_selected_entry_edit() {
            return;
        }
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
            Focus::Entries | Focus::Reader | Focus::Insights => self.begin_confirm_delete_entry(),
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

    pub(crate) fn open_settings_menu(&mut self) {
        self.overlay = Overlay::SettingsMenu;
    }

    /// Open the theme picker: list the theme files on disk (parse results
    /// cached per row), seed the selection on the configured theme, and
    /// remember the installed theme so Esc can restore it.
    pub(crate) fn open_theme_picker(&mut self) {
        use crate::tui::state::{SelectableList, ThemePickerEntry, ThemePickerState};

        let dir = crate::tui::theme::themes_dir(&self.config_path);
        if let Err(err) = crate::tui::theme::ensure_bundled(&dir) {
            self.toast(
                ToastVariant::Error,
                format!(
                    "Couldn't prepare themes: {}",
                    crate::tui::concise_error(&err)
                ),
            );
        }
        let mode = crate::tui::theme::mode();
        let mut entries: Vec<ThemePickerEntry> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|dirent| {
                let path = dirent.path();
                if path.extension().is_none_or(|ext| ext != "toml") {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                let dark = crate::tui::theme::load_file(&path, crate::tui::theme::Mode::Dark).ok();
                let light =
                    crate::tui::theme::load_file(&path, crate::tui::theme::Mode::Light).ok();
                let mode_agnostic = dark == light;
                Some(ThemePickerEntry {
                    theme: match mode {
                        crate::tui::theme::Mode::Dark => dark,
                        crate::tui::theme::Mode::Light => light,
                    },
                    name,
                    mode_agnostic,
                })
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        let mut state = ThemePickerState {
            entries,
            list: SelectableList::default(),
            previous: crate::tui::theme::theme(),
            previous_name: self.config.ui.theme.clone(),
            previous_chrome: crate::tui::theme::chrome_override(),
            previous_color_mode: crate::tui::theme::color_mode(),
        };
        let active = state
            .entries
            .iter()
            .position(|entry| entry.name == state.previous_name)
            .unwrap_or(0);
        state.select_index(active);
        self.overlay = Overlay::ThemePicker(state);
    }

    pub(crate) fn theme_picker_state(&self) -> Option<&ThemePickerState> {
        match &self.overlay {
            Overlay::ThemePicker(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn theme_picker_state_mut(&mut self) -> Option<&mut ThemePickerState> {
        match &mut self.overlay {
            Overlay::ThemePicker(state) => Some(state),
            _ => None,
        }
    }

    /// Live preview: install the highlighted theme if it parsed. Broken rows
    /// leave whatever is installed untouched.
    pub(crate) fn theme_picker_preview(&mut self) {
        if let Some(theme) = self
            .theme_picker_state()
            .and_then(|state| state.selected_entry())
            .and_then(|entry| entry.theme)
        {
            crate::tui::theme::install(theme);
        }
    }

    /// Move the picker selection to `index` and reader that row.
    pub(crate) fn theme_picker_select(&mut self, index: usize) {
        if let Some(state) = self.theme_picker_state_mut() {
            state.select_index(index);
        }
        self.theme_picker_preview();
    }

    /// Cycle the chrome override (default → flat → bordered → default),
    /// previewing live — `theme()` applies it on read, so the next frame
    /// re-chromes.
    /// Persisted on confirm; cancel restores the value from open time.
    pub(crate) fn theme_picker_cycle_chrome(&mut self) {
        use crate::tui::theme::{ChromeStyle, chrome_override, set_chrome_override};
        set_chrome_override(match chrome_override() {
            None => Some(ChromeStyle::Flat),
            Some(ChromeStyle::Flat) => Some(ChromeStyle::Bordered),
            Some(ChromeStyle::Bordered) => None,
        });
    }

    /// Cycle the color mode (auto → dark → light → auto), previewing live.
    /// Unlike the chrome override, a mode change invalidates every resolved
    /// theme (variants are flattened at load), so the picker's rows re-resolve
    /// and the highlighted one re-installs.
    pub(crate) fn theme_picker_cycle_mode(&mut self) {
        use crate::config::ColorMode;
        // No-op on rows where the switch is hidden (its hint is gone too).
        if !self
            .theme_picker_state()
            .is_some_and(|state| state.mode_switchable())
        {
            return;
        }
        crate::tui::theme::set_color_mode(match crate::tui::theme::color_mode() {
            ColorMode::Auto => ColorMode::Dark,
            ColorMode::Dark => ColorMode::Light,
            ColorMode::Light => ColorMode::Auto,
        });
        let dir = crate::tui::theme::themes_dir(&self.config_path);
        let mode = crate::tui::theme::mode();
        if let Some(state) = self.theme_picker_state_mut() {
            for entry in &mut state.entries {
                let path = dir.join(format!("{}.toml", entry.name));
                entry.theme = crate::tui::theme::load_file(&path, mode).ok();
            }
        }
        self.theme_picker_preview();
    }

    /// Confirm the highlighted theme: persist it to the config and close. A
    /// broken row or a failed save toasts and keeps the picker open.
    pub(crate) fn theme_picker_confirm(&mut self) {
        let Some(entry) = self
            .theme_picker_state()
            .and_then(|state| state.selected_entry())
        else {
            return;
        };
        let name = entry.name.clone();
        let Some(theme) = entry.theme else {
            self.toast(
                ToastVariant::Error,
                format!("Theme '{name}' is broken; fix its file or pick another"),
            );
            return;
        };
        crate::tui::theme::install(theme);
        self.config.ui.theme = name.clone();
        self.config.ui.color_mode = crate::tui::theme::color_mode();
        self.config.ui.chrome = match crate::tui::theme::chrome_override() {
            None => crate::config::ChromeMode::Default,
            Some(crate::tui::theme::ChromeStyle::Flat) => crate::config::ChromeMode::Flat,
            Some(crate::tui::theme::ChromeStyle::Bordered) => crate::config::ChromeMode::Bordered,
        };
        if let Err(err) = crate::config::save_config(&self.config_path, &self.config) {
            self.toast(
                ToastVariant::Error,
                format!("Couldn't save config: {}", crate::tui::concise_error(&err)),
            );
            return;
        }
        self.toast(ToastVariant::Success, format!("Theme set to {name}"));
        self.close_overlay();
    }

    /// Cancel the picker: restore the theme, chrome override, and color mode
    /// from open time; the config was never touched.
    pub(crate) fn theme_picker_cancel(&mut self) {
        if let Some(state) = self.theme_picker_state() {
            crate::tui::theme::set_color_mode(state.previous_color_mode);
            crate::tui::theme::set_chrome_override(state.previous_chrome);
            crate::tui::theme::install(state.previous);
        }
        self.close_overlay();
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
