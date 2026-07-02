use crate::{
    AppResult,
    config::Config,
    crypto,
    markdown::split_front_matter,
    storage::{
        self, Entry, EntryEncryptionState, Journal, SearchHit, SearchScopeFilter,
        entry_timestamp_label, search_loaded_entries,
    },
};
use std::{path::PathBuf, time::Duration};

use super::state::{EditTagFocus, EditTagState, Overlay, ScrollState, SearchState, StatusBar};

pub(crate) const JOURNAL_LIST_WIDTH: u16 = 18;
pub(crate) const ENTRY_LIST_INLINE_WIDTH: u16 = 42;
pub(crate) const ENTRY_LIST_MIN_WIDTH: u16 = 40;
pub(crate) const TWO_PANEL_MIN_WIDTH: u16 = JOURNAL_LIST_WIDTH + ENTRY_LIST_MIN_WIDTH;
pub(crate) const INLINE_ENTRY_VIEW_MIN_WIDTH: u16 =
    JOURNAL_LIST_WIDTH + ENTRY_LIST_INLINE_WIDTH + ENTRY_LIST_MIN_WIDTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Journals,
    Entries,
    EntryView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Browse,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchScope {
    AllJournals,
    CurrentJournal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryTarget {
    pub(crate) path: PathBuf,
    pub(crate) title: String,
}

pub(crate) struct App {
    pub(crate) config: Config,
    pub(crate) encryption_paths: crypto::EncryptionPaths,
    pub(crate) unlocked_identity: Option<crypto::UnlockedIdentity>,
    pub(crate) journals: Vec<Journal>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) selected_journal: usize,
    pub(crate) selected_entry_index: usize,
    pub(crate) scroll: ScrollState,
    pub(crate) focus: Focus,
    pub(crate) mode: Mode,
    pub(crate) search: SearchState,
    pub(crate) overlay: Overlay,
    pub(crate) status_bar: StatusBar,
    pub(crate) entry_view_expanded: bool,
}

impl App {
    pub(crate) fn new(
        config: Config,
        encryption_paths: crypto::EncryptionPaths,
    ) -> AppResult<Self> {
        storage::ensure_workspace(&config.journal_root)?;
        let entry_paths = storage::collect_entry_paths(&config.journal_root)?;
        let unlocked_identity = if crypto::can_decrypt(&encryption_paths)
            && entry_paths
                .iter()
                .any(|entry| storage::is_encrypted_entry_file(&entry.path))
        {
            Some(crypto::prompt_unlock_identity(&encryption_paths)?)
        } else {
            None
        };
        let mut app = Self {
            config,
            encryption_paths,
            unlocked_identity,
            journals: Vec::new(),
            entries: Vec::new(),
            selected_journal: 0,
            selected_entry_index: 0,
            scroll: ScrollState::default(),
            focus: Focus::Journals,
            mode: Mode::Browse,
            search: SearchState::default(),
            overlay: Overlay::None,
            status_bar: StatusBar::default(),
            entry_view_expanded: false,
        };
        app.load_entries(entry_paths)?;
        Ok(app)
    }

    pub(crate) fn refresh(&mut self) -> AppResult<()> {
        storage::ensure_workspace(&self.config.journal_root)?;
        let entry_paths = storage::collect_entry_paths(&self.config.journal_root)?;
        self.load_entries(entry_paths)
    }

    fn load_entries(&mut self, entry_paths: Vec<storage::EntryPath>) -> AppResult<()> {
        self.journals = storage::list_journals(&self.config.journal_root)?;
        self.entries = storage::read_entries(entry_paths, self.unlocked_identity.as_ref())?;
        if self.selected_journal >= self.journals.len() {
            self.selected_journal = self.journals.len().saturating_sub(1);
            self.scroll.reset();
        }
        if !self.search.query.is_empty() {
            self.search.hits = self.search_results();
        }
        let previous_entry_index = self.selected_entry_index;
        self.selected_entry_index = self
            .selected_entry_index
            .min(self.current_entry_list_len().saturating_sub(1));
        if self.selected_entry_index != previous_entry_index {
            self.scroll.reset_entry();
        }
        Ok(())
    }

    pub(crate) fn selected_journal(&self) -> Option<&Journal> {
        self.journals.get(self.selected_journal)
    }

    pub(crate) fn selected_entries(&self) -> Vec<&Entry> {
        let Some(journal) = self.selected_journal() else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|entry| entry.journal == journal.name)
            .collect()
    }

    pub(crate) fn current_entry_list_len(&self) -> usize {
        match self.mode {
            Mode::Search => self.search.hits.len(),
            Mode::Browse => self.selected_entries().len(),
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let len = match self.focus {
            Focus::Journals if self.mode == Mode::Browse => self.journals.len(),
            Focus::Entries | Focus::EntryView | Focus::Journals => self.current_entry_list_len(),
        };
        if len == 0 {
            return;
        }

        let previous_entry_index = self.selected_entry_index;
        let index = match self.focus {
            Focus::Journals if self.mode == Mode::Browse => &mut self.selected_journal,
            _ => &mut self.selected_entry_index,
        };
        let next = (*index as isize + delta).clamp(0, len as isize - 1);
        *index = next as usize;
        if self.focus == Focus::Journals {
            self.selected_entry_index = 0;
            self.scroll.entry = 0;
        }
        if self.selected_entry_index != previous_entry_index {
            self.scroll.entry_view = 0;
        }
    }

    pub(crate) fn select_journal(&mut self, index: usize) {
        if index >= self.journals.len() {
            return;
        }

        if self.selected_journal != index {
            self.selected_journal = index;
            self.selected_entry_index = 0;
            self.scroll.reset_entry();
        }
    }

    pub(crate) fn select_entry_index(&mut self, index: usize) {
        if index >= self.current_entry_list_len() {
            return;
        }

        if self.selected_entry_index != index {
            self.selected_entry_index = index;
            self.scroll.entry_view = 0;
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let entries = self.selected_entries();
        entries.get(self.selected_entry_index).copied()
    }

    pub(crate) fn selected_search_hit(&self) -> Option<&SearchHit> {
        self.search.hits.get(self.selected_entry_index)
    }

    pub(crate) fn selected_entry_target(&self) -> Option<EntryTarget> {
        match self.mode {
            Mode::Search => {
                let hit = self.selected_search_hit()?;
                Some(EntryTarget {
                    path: hit.path.clone(),
                    title: self.search_hit_label(hit),
                })
            }
            Mode::Browse => {
                let entry = self.selected_entry()?;
                Some(EntryTarget {
                    path: entry.path.clone(),
                    title: entry.title.clone(),
                })
            }
        }
    }

    pub(crate) fn selected_entry_tags(&self) -> Vec<String> {
        match self.mode {
            Mode::Search => self
                .selected_search_hit()
                .and_then(|hit| {
                    self.entries
                        .iter()
                        .find(|entry| entry.path == hit.path)
                        .map(|entry| entry.tags.clone())
                })
                .unwrap_or_default(),
            Mode::Browse => self
                .selected_entry()
                .map(|entry| entry.tags.clone())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn has_selected_entry_target(&self) -> bool {
        self.selected_entry_target().is_some()
    }

    pub(crate) fn can_act_on_selected_entry(&self) -> bool {
        matches!(self.focus, Focus::Entries | Focus::EntryView) && self.has_selected_entry_target()
    }

    pub(crate) fn normalize_focus(&mut self, entry_view_available: bool) {
        if self.focus == Focus::EntryView && !entry_view_available {
            self.focus = Focus::Entries;
        }
    }

    pub(crate) fn selected_entry_view(&self) -> Option<(String, String)> {
        match self.mode {
            Mode::Search => {
                let hit = self.selected_search_hit()?;
                let entry = storage::read_entry_with_identity(
                    &hit.journal,
                    &hit.path,
                    self.unlocked_identity.as_ref(),
                )
                .ok()?;
                Some((entry_timestamp_label(&entry), markdown_body(&entry.content)))
            }
            Mode::Browse => {
                let entry = self.selected_entry()?;
                if entry.encryption_state == EntryEncryptionState::EncryptedLocked {
                    return Some((
                        entry_timestamp_label(entry),
                        "Encryption identity not available".to_string(),
                    ));
                }
                Some((entry_timestamp_label(entry), markdown_body(&entry.content)))
            }
        }
    }

    pub(crate) fn begin_new_journal_input(&mut self) {
        self.overlay = Overlay::NewJournal(String::new());
        self.clear_status();
    }

    pub(crate) fn new_journal_input(&self) -> Option<&str> {
        match &self.overlay {
            Overlay::NewJournal(name) => Some(name),
            _ => None,
        }
    }

    pub(crate) fn new_journal_input_mut(&mut self) -> Option<&mut String> {
        match &mut self.overlay {
            Overlay::NewJournal(name) => Some(name),
            _ => None,
        }
    }

    pub(crate) fn edit_tag_state(&self) -> Option<&EditTagState> {
        match &self.overlay {
            Overlay::EditTags(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn edit_tag_state_mut(&mut self) -> Option<&mut EditTagState> {
        match &mut self.overlay {
            Overlay::EditTags(state) => Some(state),
            _ => None,
        }
    }

    pub(crate) fn is_confirming_delete(&self) -> bool {
        matches!(self.overlay, Overlay::ConfirmDelete)
    }

    pub(crate) fn begin_confirm_delete(&mut self) {
        self.overlay = Overlay::ConfirmDelete;
    }

    pub(crate) fn close_overlay(&mut self) {
        self.overlay = Overlay::None;
    }

    pub(crate) fn select_journal_by_name(&mut self, name: &str) {
        if let Some(index) = self
            .journals
            .iter()
            .position(|journal| journal.name == name)
        {
            self.selected_journal = index;
            self.selected_entry_index = 0;
            self.scroll.journal = index.min(u16::MAX as usize) as u16;
            self.scroll.reset_entry();
            self.focus = Focus::Entries;
        }
    }

    /// Collect all tags across every loaded entry, sorted by usage count
    /// (most frequent first) and then alphabetically. Tags differing only in
    /// case are consolidated: the most common casing wins (ties go to the
    /// first alphabetically).
    pub(crate) fn all_tags_sorted(&self) -> Vec<(String, usize)> {
        // First pass — count per lowercased key, track casing frequency.
        let mut lower_to_casing: std::collections::BTreeMap<String, CasingCount> =
            std::collections::BTreeMap::new();
        for entry in &self.entries {
            for tag in &entry.tags {
                let lower = tag.to_lowercase();
                let entry = lower_to_casing.entry(lower).or_default();
                entry.total += 1;
                *entry.forms.entry(tag.clone()).or_default() += 1;
            }
        }
        let mut pairs: Vec<_> = lower_to_casing
            .into_values()
            .map(|cc| {
                // Pick the casing form with the highest frequency; ties → first alphabetically.
                let display = cc
                    .forms
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map(|(form, _)| form)
                    .unwrap_or_default();
                (display, cc.total)
            })
            .collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        pairs
    }

    pub(crate) fn begin_edit_tags(&mut self) {
        let all_tags = self.all_tags_sorted();
        let filtered: Vec<usize> = (0..all_tags.len()).collect();
        let entry_tags: Vec<String> = self
            .selected_entry_tags()
            .into_iter()
            .map(|t| t.to_lowercase())
            .collect();
        self.overlay = Overlay::EditTags(EditTagState {
            all_tags,
            filtered,
            selected: entry_tags,
            cursor: 0,
            scroll: 0,
            input: String::new(),
            focus: EditTagFocus::List,
        });
    }

    pub(crate) fn begin_tag_search(&mut self, tag: &str) {
        self.search.scope = self
            .selected_journal()
            .map(|journal| SearchScope::CurrentJournal(journal.name.clone()))
            .unwrap_or(SearchScope::AllJournals);
        self.mode = Mode::Search;
        self.focus = Focus::Entries;
        self.search.query = format!("tags:{tag}");
        self.search.hits = self.search_results_by_tag(tag);
        self.selected_entry_index = 0;
        self.scroll.reset_entry();
    }

    pub(crate) fn begin_search(&mut self) {
        self.search.scope = if self.focus == Focus::Journals {
            SearchScope::AllJournals
        } else {
            self.selected_journal()
                .map(|journal| SearchScope::CurrentJournal(journal.name.clone()))
                .unwrap_or(SearchScope::AllJournals)
        };
        self.mode = Mode::Search;
        self.focus = Focus::Entries;
        self.search.query.clear();
        self.search.hits.clear();
        self.selected_entry_index = 0;
        self.scroll.reset_entry();
    }

    pub(crate) fn exit_search(&mut self) {
        self.mode = Mode::Browse;
        self.search.scope = SearchScope::AllJournals;
        self.search.query.clear();
        self.search.hits.clear();
        self.selected_entry_index = 0;
        self.scroll.reset_entry();
    }

    pub(crate) fn update_search_results(&mut self) {
        self.search.hits = self.search_results();
        self.selected_entry_index = 0;
        self.scroll.reset_entry();
    }

    pub(crate) fn search_scope_label(&self) -> String {
        match &self.search.scope {
            SearchScope::AllJournals => "all".to_string(),
            SearchScope::CurrentJournal(journal) => journal.clone(),
        }
    }

    pub(crate) fn search_hit_label(&self, hit: &SearchHit) -> String {
        match self.search.scope {
            SearchScope::AllJournals => format!("{}/{}", hit.journal, hit.title),
            SearchScope::CurrentJournal(_) => hit.title.clone(),
        }
    }

    fn search_results(&self) -> Vec<SearchHit> {
        if let Some(tag) = self.search.query.strip_prefix("tags:") {
            self.search_results_by_tag(tag.trim())
        } else {
            search_loaded_entries(
                &self.entries,
                &self.search.query,
                self.search.scope.filter(),
            )
        }
    }

    fn search_results_by_tag(&self, tag: &str) -> Vec<SearchHit> {
        let tag_lower = tag.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                entry.encryption_state != EntryEncryptionState::EncryptedLocked
                    && entry
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&tag_lower))
            })
            .filter(|entry| match self.search.scope {
                SearchScope::AllJournals => true,
                SearchScope::CurrentJournal(ref journal) => entry.journal == *journal,
            })
            .map(|entry| SearchHit {
                path: entry.path.clone(),
                journal: entry.journal.clone(),
                title: entry.title.clone(),
                preview: entry.preview.clone(),
            })
            .collect()
    }

    pub(crate) fn scroll_entry_view(&mut self, delta: i16) {
        if delta.is_negative() {
            self.scroll.entry_view = self.scroll.entry_view.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll.entry_view = self.scroll.entry_view.saturating_add(delta as u16);
        }
    }

    pub(crate) fn page_entry_view(&mut self, delta: i16) {
        self.scroll_entry_view(delta.saturating_mul(10));
    }

    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.status_bar.set(message);
    }

    pub(crate) fn clear_status(&mut self) {
        self.status_bar.clear();
    }

    pub(crate) fn status(&self) -> &str {
        self.status_bar.text()
    }

    pub(crate) fn status_timeout(&self) -> Option<Duration> {
        self.status_bar.timeout()
    }

    pub(crate) fn expire_status(&mut self) -> bool {
        self.status_bar.expire()
    }
}

impl SearchScope {
    fn filter(&self) -> SearchScopeFilter<'_> {
        match self {
            SearchScope::AllJournals => SearchScopeFilter::AllJournals,
            SearchScope::CurrentJournal(journal) => SearchScopeFilter::Journal(journal),
        }
    }
}

/// Helper for [`App::all_tags_sorted`]: counts per lowercased tag and per
/// original-casing form so we can consolidate case variants.
#[derive(Default)]
struct CasingCount {
    total: usize,
    forms: std::collections::BTreeMap<String, usize>,
}

pub(crate) fn markdown_body(content: &str) -> String {
    let (_, body) = split_front_matter(content);
    body.trim_start().to_string()
}

pub(crate) fn inline_entry_view_is_visible(width: u16) -> bool {
    width >= INLINE_ENTRY_VIEW_MIN_WIDTH
}

pub(crate) fn entry_view_is_available(width: u16) -> bool {
    width >= TWO_PANEL_MIN_WIDTH
}

pub(crate) fn single_panel_is_active(width: u16) -> bool {
    width < TWO_PANEL_MIN_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn new_app(config: Config) -> App {
        let encryption_paths = crypto::EncryptionPaths::for_config(
            &config.journal_root.join("config.toml"),
            &config.journal_root,
        )
        .unwrap();
        App::new(config, encryption_paths).unwrap()
    }

    #[test]
    fn changing_selected_entry_resets_entry_view_scroll() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n...\n\n# A\n").unwrap();
        fs::write(entry_dir.join("b.md"), "---\ntags: []\n...\n\n# B\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;
        app.scroll.entry_view = 20;

        app.move_selection(1);

        assert_eq!(app.scroll.entry_view, 0);
    }

    #[test]
    fn markdown_body_strips_front_matter_for_entry_view() {
        let content = "---\ntags: []\n...\n\n# Title\nBody\n";

        assert_eq!(markdown_body(content), "# Title\nBody\n");
    }

    #[test]
    fn selected_entry_view_title_uses_entry_timestamp() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:23:00+02:00\"\n...\n\n# A\nBody\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        let (title, content) = app.selected_entry_view().unwrap();

        assert_eq!(title, "2026-07-01 10:23");
        assert_eq!(content, "# A\nBody\n");
    }

    #[test]
    fn search_entry_view_title_uses_entry_timestamp() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:23:00+02:00\"\n...\n\n# A\nneedle\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.begin_search();
        app.search.query = "needle".to_string();
        app.update_search_results();

        let (title, content) = app.selected_entry_view().unwrap();

        assert_eq!(title, "2026-07-01 10:23");
        assert_eq!(content, "# A\nneedle\n");
    }

    #[test]
    fn journal_focus_does_not_make_entry_targets_actionable() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n...\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        app.focus = Focus::Journals;
        assert!(!app.can_act_on_selected_entry());

        app.focus = Focus::Entries;
        assert!(app.can_act_on_selected_entry());
    }

    #[test]
    fn hidden_entry_view_focus_falls_back_to_entries() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.focus = Focus::EntryView;

        app.normalize_focus(false);

        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn available_entry_view_focus_is_preserved() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.focus = Focus::EntryView;

        app.normalize_focus(true);

        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn compact_width_uses_single_panel_without_inline_entry_view() {
        assert!(single_panel_is_active(TWO_PANEL_MIN_WIDTH - 1));
        assert!(!inline_entry_view_is_visible(TWO_PANEL_MIN_WIDTH - 1));
        assert!(!entry_view_is_available(TWO_PANEL_MIN_WIDTH - 1));
        assert!(entry_view_is_available(TWO_PANEL_MIN_WIDTH));
    }

    #[test]
    fn inline_entry_view_uses_minimum_three_column_width() {
        assert!(!inline_entry_view_is_visible(
            INLINE_ENTRY_VIEW_MIN_WIDTH - 1
        ));
        assert!(inline_entry_view_is_visible(INLINE_ENTRY_VIEW_MIN_WIDTH));
    }

    #[test]
    fn search_from_journal_focus_is_global() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.focus = Focus::Journals;

        app.begin_search();

        assert_eq!(app.search.scope, SearchScope::AllJournals);
    }

    #[test]
    fn search_from_entries_focus_is_scoped_to_selected_journal() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        app.begin_search();

        assert_eq!(
            app.search.scope,
            SearchScope::CurrentJournal("work".to_string())
        );
    }

    #[test]
    fn status_timeout_is_none_without_active_status() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let app = new_app(config);

        assert!(app.status_timeout().is_none());
    }

    #[test]
    fn status_timeout_is_some_with_active_status() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = new_app(config);

        app.set_status("Saved");

        assert!(app.status_timeout().is_some());
    }

    #[test]
    fn expire_status_reports_visible_change_once() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.status_bar.set_expired("Saved");

        assert!(app.expire_status());
        assert!(app.status().is_empty());
        assert!(!app.expire_status());
    }
}
