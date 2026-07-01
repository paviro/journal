use crate::{
    AppResult,
    config::Config,
    markdown::split_front_matter,
    storage::{self, Entry, Journal, SearchHit, SearchScopeFilter, scan_entries, search_entries},
};
use chrono::{DateTime, Local, NaiveDate};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const STATUS_DURATION: Duration = Duration::from_secs(3);
pub(crate) const JOURNAL_LIST_WIDTH: u16 = 18;
pub(crate) const ENTRY_LIST_MIN_WIDTH: u16 = 40;
pub(crate) const TWO_PANEL_MIN_WIDTH: u16 = JOURNAL_LIST_WIDTH + ENTRY_LIST_MIN_WIDTH;
pub(crate) const PREVIEW_MIN_WIDTH: u16 = 118;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Journals,
    Entries,
    Preview,
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
    pub(crate) journals: Vec<Journal>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) search_hits: Vec<SearchHit>,
    pub(crate) selected_journal: usize,
    pub(crate) selected_entry_index: usize,
    pub(crate) preview_scroll: u16,
    pub(crate) focus: Focus,
    pub(crate) mode: Mode,
    pub(crate) new_journal_input: Option<String>,
    pub(crate) viewer: Option<MarkdownView>,
    pub(crate) search_query: String,
    pub(crate) search_scope: SearchScope,
    pub(crate) confirm_delete: bool,
    pub(crate) status: String,
    status_until: Option<Instant>,
}

pub(crate) struct MarkdownView {
    pub(crate) title: String,
    pub(crate) path: PathBuf,
    pub(crate) content: String,
    pub(crate) scroll: u16,
}

impl App {
    pub(crate) fn new(config: Config) -> AppResult<Self> {
        let mut app = Self {
            config,
            journals: Vec::new(),
            entries: Vec::new(),
            search_hits: Vec::new(),
            selected_journal: 0,
            selected_entry_index: 0,
            preview_scroll: 0,
            focus: Focus::Journals,
            mode: Mode::Browse,
            new_journal_input: None,
            viewer: None,
            search_query: String::new(),
            search_scope: SearchScope::AllJournals,
            confirm_delete: false,
            status: String::new(),
            status_until: None,
        };
        app.refresh()?;
        Ok(app)
    }

    pub(crate) fn refresh(&mut self) -> AppResult<()> {
        storage::ensure_workspace(&self.config.journal_root)?;
        self.journals = storage::list_journals(&self.config.journal_root)?;
        self.entries = scan_entries(&self.config.journal_root)?;
        if self.selected_journal >= self.journals.len() {
            self.selected_journal = self.journals.len().saturating_sub(1);
            self.preview_scroll = 0;
        }
        if !self.search_query.is_empty() {
            self.search_hits = self.search_results()?;
        }
        let previous_entry_index = self.selected_entry_index;
        self.selected_entry_index = self
            .selected_entry_index
            .min(self.current_entry_list_len().saturating_sub(1));
        if self.selected_entry_index != previous_entry_index {
            self.preview_scroll = 0;
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
            Mode::Search => self.search_hits.len(),
            Mode::Browse => self.selected_entries().len(),
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let len = match self.focus {
            Focus::Journals if self.mode == Mode::Browse => self.journals.len(),
            Focus::Entries | Focus::Preview | Focus::Journals => self.current_entry_list_len(),
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
        }
        if self.selected_entry_index != previous_entry_index {
            self.preview_scroll = 0;
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let entries = self.selected_entries();
        entries.get(self.selected_entry_index).copied()
    }

    pub(crate) fn selected_search_hit(&self) -> Option<&SearchHit> {
        self.search_hits.get(self.selected_entry_index)
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

    pub(crate) fn has_selected_entry_target(&self) -> bool {
        self.selected_entry_target().is_some()
    }

    pub(crate) fn can_act_on_selected_entry(&self) -> bool {
        matches!(self.focus, Focus::Entries | Focus::Preview) && self.has_selected_entry_target()
    }

    pub(crate) fn normalize_focus(&mut self, preview_available: bool) {
        if self.focus == Focus::Preview && !preview_available {
            self.focus = Focus::Entries;
        }
    }

    pub(crate) fn selected_entry_preview(&self) -> Option<(String, String)> {
        match self.mode {
            Mode::Search => {
                let hit = self.selected_search_hit()?;
                let entry = storage::read_entry(&hit.journal, &hit.path).ok()?;
                Some((entry_timestamp_label(&entry), markdown_body(&entry.content)))
            }
            Mode::Browse => {
                let entry = self.selected_entry()?;
                Some((entry_timestamp_label(entry), markdown_body(&entry.content)))
            }
        }
    }

    pub(crate) fn begin_new_journal_input(&mut self) {
        self.new_journal_input = Some(String::new());
        self.clear_status();
    }

    pub(crate) fn select_journal_by_name(&mut self, name: &str) {
        if let Some(index) = self
            .journals
            .iter()
            .position(|journal| journal.name == name)
        {
            self.selected_journal = index;
            self.selected_entry_index = 0;
            self.preview_scroll = 0;
            self.focus = Focus::Entries;
        }
    }

    pub(crate) fn begin_search(&mut self) {
        self.search_scope = if self.focus == Focus::Journals {
            SearchScope::AllJournals
        } else {
            self.selected_journal()
                .map(|journal| SearchScope::CurrentJournal(journal.name.clone()))
                .unwrap_or(SearchScope::AllJournals)
        };
        self.mode = Mode::Search;
        self.focus = Focus::Entries;
        self.search_query.clear();
        self.search_hits.clear();
        self.selected_entry_index = 0;
        self.preview_scroll = 0;
    }

    pub(crate) fn exit_search(&mut self) {
        self.mode = Mode::Browse;
        self.search_scope = SearchScope::AllJournals;
        self.search_query.clear();
        self.search_hits.clear();
        self.selected_entry_index = 0;
        self.preview_scroll = 0;
    }

    pub(crate) fn update_search_results(&mut self) -> AppResult<()> {
        self.search_hits = self.search_results()?;
        self.selected_entry_index = 0;
        self.preview_scroll = 0;
        Ok(())
    }

    pub(crate) fn search_scope_label(&self) -> String {
        match &self.search_scope {
            SearchScope::AllJournals => "all".to_string(),
            SearchScope::CurrentJournal(journal) => journal.clone(),
        }
    }

    pub(crate) fn search_hit_label(&self, hit: &SearchHit) -> String {
        match self.search_scope {
            SearchScope::AllJournals => format!("{}/{}", hit.journal, hit.title),
            SearchScope::CurrentJournal(_) => hit.title.clone(),
        }
    }

    fn search_results(&self) -> AppResult<Vec<SearchHit>> {
        search_entries(
            &self.config.journal_root,
            &self.search_query,
            self.search_scope.filter(),
        )
    }

    pub(crate) fn scroll_preview(&mut self, delta: i16) {
        if delta.is_negative() {
            self.preview_scroll = self.preview_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.preview_scroll = self.preview_scroll.saturating_add(delta as u16);
        }
    }

    pub(crate) fn page_preview(&mut self, delta: i16) {
        self.scroll_preview(delta.saturating_mul(10));
    }

    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_until = Some(Instant::now() + STATUS_DURATION);
    }

    pub(crate) fn clear_status(&mut self) {
        self.status.clear();
        self.status_until = None;
    }

    pub(crate) fn expire_status(&mut self) {
        if self
            .status_until
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.clear_status();
        }
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

pub(crate) fn markdown_body(content: &str) -> String {
    let (_, body) = split_front_matter(content);
    body.trim_start().to_string()
}

pub(crate) fn entry_timestamp_label(entry: &Entry) -> String {
    entry
        .created_at
        .as_deref()
        .and_then(parse_entry_timestamp)
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M").to_string())
        .or_else(|| {
            entry_date_from_path(&entry.path).map(|date| date.format("%Y-%m-%d").to_string())
        })
        .unwrap_or_else(|| "Preview".to_string())
}

fn parse_entry_timestamp(value: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Local))
}

fn entry_date_from_path(path: &std::path::Path) -> Option<NaiveDate> {
    let date = path.parent()?.file_name()?.to_str()?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

pub(crate) fn preview_is_visible(width: u16) -> bool {
    width >= PREVIEW_MIN_WIDTH
}

pub(crate) fn single_panel_is_active(width: u16) -> bool {
    width < TWO_PANEL_MIN_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn changing_selected_entry_resets_preview_scroll() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n---\n\n# A\n").unwrap();
        fs::write(entry_dir.join("b.md"), "---\ntags: []\n---\n\n# B\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;
        app.preview_scroll = 20;

        app.move_selection(1);

        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn markdown_body_strips_front_matter_for_preview() {
        let content = "---\ntags: []\n---\n\n# Title\nBody\n";

        assert_eq!(markdown_body(content), "# Title\nBody\n");
    }

    #[test]
    fn selected_preview_title_uses_entry_timestamp() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:23:00+02:00\"\n---\n\n# A\nBody\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");

        let (title, content) = app.selected_entry_preview().unwrap();

        assert_eq!(title, "2026-07-01 10:23");
        assert_eq!(content, "# A\nBody\n");
    }

    #[test]
    fn search_preview_title_uses_entry_timestamp() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:23:00+02:00\"\n---\n\n# A\nneedle\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app.begin_search();
        app.search_query = "needle".to_string();
        app.update_search_results().unwrap();

        let (title, content) = app.selected_entry_preview().unwrap();

        assert_eq!(title, "2026-07-01 10:23");
        assert_eq!(content, "# A\nneedle\n");
    }

    #[test]
    fn journal_focus_does_not_make_entry_targets_actionable() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n---\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");

        app.focus = Focus::Journals;
        assert!(!app.can_act_on_selected_entry());

        app.focus = Focus::Entries;
        assert!(app.can_act_on_selected_entry());
    }

    #[test]
    fn hidden_preview_focus_falls_back_to_entries() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.focus = Focus::Preview;

        app.normalize_focus(false);

        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn compact_width_uses_single_panel_without_inline_preview() {
        assert!(single_panel_is_active(TWO_PANEL_MIN_WIDTH - 1));
        assert!(!preview_is_visible(TWO_PANEL_MIN_WIDTH - 1));
    }

    #[test]
    fn search_from_journal_focus_is_global() {
        let config = Config::new(tempdir().unwrap().path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.focus = Focus::Journals;

        app.begin_search();

        assert_eq!(app.search_scope, SearchScope::AllJournals);
    }

    #[test]
    fn search_from_entries_focus_is_scoped_to_selected_journal() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        app.begin_search();

        assert_eq!(
            app.search_scope,
            SearchScope::CurrentJournal("work".to_string())
        );
    }
}
