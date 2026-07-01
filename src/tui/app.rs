use crate::{
    AppResult,
    config::Config,
    markdown::split_front_matter,
    storage::{self, Entry, Journal, SearchHit, scan_entries, search_all},
};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const STATUS_DURATION: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Journals,
    Items,
    Preview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Browse,
    Search,
}

pub(crate) struct App {
    pub(crate) config: Config,
    pub(crate) journals: Vec<Journal>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) search_hits: Vec<SearchHit>,
    pub(crate) selected_journal: usize,
    pub(crate) selected_item: usize,
    pub(crate) preview_scroll: u16,
    pub(crate) focus: Focus,
    pub(crate) mode: Mode,
    pub(crate) new_journal_input: Option<String>,
    pub(crate) viewer: Option<MarkdownView>,
    pub(crate) search_query: String,
    pub(crate) confirm_delete: bool,
    pub(crate) status: String,
    status_until: Option<Instant>,
}

pub(crate) struct MarkdownView {
    pub(crate) title: String,
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
            selected_item: 0,
            preview_scroll: 0,
            focus: Focus::Journals,
            mode: Mode::Browse,
            new_journal_input: None,
            viewer: None,
            search_query: String::new(),
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
        let previous_item = self.selected_item;
        self.selected_item = self
            .selected_item
            .min(self.current_item_count().saturating_sub(1));
        if self.selected_item != previous_item {
            self.preview_scroll = 0;
        }
        if !self.search_query.is_empty() {
            self.search_hits = search_all(&self.config.journal_root, &self.search_query)?;
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

    pub(crate) fn current_item_count(&self) -> usize {
        match self.mode {
            Mode::Search => self.search_hits.len(),
            Mode::Browse => self.selected_entries().len(),
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let len = match self.focus {
            Focus::Journals if self.mode == Mode::Browse => self.journals.len(),
            Focus::Items | Focus::Preview | Focus::Journals => self.current_item_count(),
        };
        if len == 0 {
            return;
        }

        let previous_item = self.selected_item;
        let index = match self.focus {
            Focus::Journals if self.mode == Mode::Browse => &mut self.selected_journal,
            _ => &mut self.selected_item,
        };
        let next = (*index as isize + delta).clamp(0, len as isize - 1);
        *index = next as usize;
        if self.focus == Focus::Journals {
            self.selected_item = 0;
        }
        if self.selected_item != previous_item {
            self.preview_scroll = 0;
        }
    }

    pub(crate) fn selected_entry_path(&self) -> Option<PathBuf> {
        let entries = self.selected_entries();
        entries
            .get(self.selected_item)
            .map(|entry| entry.path.to_path_buf())
    }

    pub(crate) fn selected_search_hit(&self) -> Option<SearchHit> {
        self.search_hits.get(self.selected_item).cloned()
    }

    pub(crate) fn selected_markdown_path(&self) -> Option<PathBuf> {
        match self.mode {
            Mode::Search => self.selected_search_hit().map(|hit| hit.path),
            Mode::Browse => self.selected_entry_path(),
        }
    }

    pub(crate) fn selected_markdown_preview(&self) -> Option<(String, String)> {
        match self.mode {
            Mode::Search => {
                let hit = self.selected_search_hit()?;
                let content = fs::read_to_string(&hit.path).ok()?;
                Some((hit.label, markdown_body(&content)))
            }
            Mode::Browse => {
                let entries = self.selected_entries();
                let entry = entries.get(self.selected_item)?;
                Some((entry.title.clone(), markdown_body(&entry.content)))
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
            self.selected_item = 0;
            self.preview_scroll = 0;
            self.focus = Focus::Items;
        }
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

pub(crate) fn markdown_body(content: &str) -> String {
    let (_, body) = split_front_matter(content);
    body.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
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
        app.focus = Focus::Items;
        app.preview_scroll = 20;

        app.move_selection(1);

        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn markdown_body_strips_front_matter_for_preview() {
        let content = "---\ntags: []\n---\n\n# Title\nBody\n";

        assert_eq!(markdown_body(content), "# Title\nBody\n");
    }
}
