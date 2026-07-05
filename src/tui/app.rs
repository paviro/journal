use crate::{AppResult, config::Config};
use journal_core::feelings::{FEELINGS, normalize_feeling};
use journal_storage::{
    Entry, EntryEncryptionState, EntryPath, Journal, JournalStore, SearchHit, SearchScopeFilter,
    entry_timestamp_label, search_loaded_entries,
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

use ratatui::{
    layout::{Rect, Size},
    widgets::ListState,
};

use super::state::{
    DeleteContext, EditFeelingState, EditMoodState, EditTagState, ImageViewerState, MetadataKind,
    Overlay, ScrollState, SearchState, StatusBar, ensure_selected_visible, move_list_selection,
    normalize_list_state, scroll_list_offset,
};
use crate::tui::entry_rows::EntryRowMeta;
use crate::tui::image::{ImageAsset, ImageRuntime, entry_images, viewer_image_size};

pub(crate) const JOURNAL_LIST_WIDTH: u16 = 22;
pub(crate) const ENTRY_LIST_INLINE_WIDTH: u16 = 42;
pub(crate) const ENTRY_LIST_MIN_WIDTH: u16 = 40;
pub(crate) const TWO_PANEL_MIN_WIDTH: u16 = 87;
pub(crate) const INLINE_ENTRY_VIEW_MIN_WIDTH: u16 = 125;

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
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) title: String,
    /// Encrypted entry whose identity is not loaded — cannot be read or written.
    pub(crate) locked: bool,
}

pub(crate) struct App {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Config,
    pub(crate) store: JournalStore,
    pub(crate) journals: Vec<Journal>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) journal_list: ListState,
    /// The selected entry (or search hit) index, or `None` when no entry is
    /// selected. In Browse mode `None` shows the journal stats in the preview
    /// pane instead of an entry — reached by scrolling up past the first entry
    /// or clicking empty space in the list.
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) entry_list: ListState,
    pub(crate) scroll: ScrollState,
    pub(crate) focus: Focus,
    pub(crate) mode: Mode,
    pub(crate) search: SearchState,
    pub(crate) overlay: Overlay,
    pub(crate) status_bar: StatusBar,
    pub(crate) images: ImageRuntime,
    /// `(entry_path, viewer_size)` the image cache is warmed for, or `None` when
    /// no entry view is open. Compared against the desired context each tick.
    image_warm: Option<(PathBuf, Size)>,
    /// Clickable `[Image N …]` label positions from the last entry-view render.
    pub(crate) entry_view_image_hits: EntryViewImageHits,
    /// Selected entry's in-folder images, memoized by path; `RefCell` so `&self`
    /// render/hint/shortcut paths can read it. Re-parsed on a path change or when
    /// `refresh` clears it.
    selected_images_cache: RefCell<Option<(PathBuf, Rc<Vec<ImageAsset>>)>>,
}

/// Clickable image label positions in the entry view, captured at render time so
/// the mouse handler can map a click back to an image index.
#[derive(Default)]
pub(crate) struct EntryViewImageHits {
    pub(crate) content_rect: Rect,
    pub(crate) scroll: u16,
    /// `(body line index, image index)` per label line.
    pub(crate) labels: Vec<(usize, usize)>,
}

impl App {
    pub(crate) fn new(
        config_path: PathBuf,
        config: Config,
        mut store: JournalStore,
    ) -> AppResult<Self> {
        store.ensure()?;
        let entry_paths = store.collect_entry_paths()?;
        if store.unlock_available() {
            let passphrase = crate::migrate::prompt_unlock_passphrase()?;
            store.unlock(&passphrase)?;
        }
        let mut app = Self {
            config_path,
            config,
            store,
            journals: Vec::new(),
            entries: Vec::new(),
            journal_list: ListState::default(),
            selected_entry_index: None,
            entry_list: ListState::default(),
            scroll: ScrollState::default(),
            focus: Focus::Journals,
            mode: Mode::Browse,
            search: SearchState::default(),
            overlay: Overlay::None,
            status_bar: StatusBar::default(),
            images: ImageRuntime::default(),
            image_warm: None,
            entry_view_image_hits: EntryViewImageHits::default(),
            selected_images_cache: RefCell::new(None),
        };
        app.load_entries(entry_paths)?;
        // Restore the journal selected in the previous session without disturbing
        // the default startup focus (Journals).
        if let Some(name) = app.config.last_journal.clone()
            && let Some(index) = app.journals.iter().position(|journal| journal.name == name)
        {
            app.journal_list.select(Some(index));
            *app.journal_list.offset_mut() = index;
        }
        // Don't start focused on the journal list if it's been hidden.
        if !app.config.show_journals {
            app.focus = Focus::Entries;
        }
        Ok(app)
    }

    pub(crate) fn refresh(&mut self) -> AppResult<()> {
        self.store.ensure()?;
        self.images.clear();
        // Content may have changed: force `sync_image_warm` to rebuild next tick
        // and drop the memo so images are re-parsed from the reloaded body.
        self.image_warm = None;
        self.selected_images_cache.borrow_mut().take();
        let entry_paths = self.store.collect_entry_paths()?;
        self.load_entries(entry_paths)
    }

    fn load_entries(&mut self, entry_paths: Vec<EntryPath>) -> AppResult<()> {
        self.journals = self.store.list_journals()?;
        self.entries = self.store.read_entries(entry_paths)?;
        normalize_list_state(&mut self.journal_list, self.journals.len());
        if !self.search.query.is_empty() {
            self.search.hits = self.search_results();
        }
        let previous_entry_index = self.selected_entry_index;
        let len = self.current_entry_list_len();
        self.selected_entry_index = self
            .selected_entry_index
            .and_then(|index| (len > 0).then(|| index.min(len - 1)));
        if self.selected_entry_index != previous_entry_index {
            self.reset_entry_scroll();
        }
        Ok(())
    }

    pub(crate) fn selected_journal_index(&self) -> usize {
        self.journal_list.selected().unwrap_or(0)
    }

    pub(crate) fn selected_journal(&self) -> Option<&Journal> {
        self.journals.get(self.selected_journal_index())
    }

    /// The preview pane shows journal stats (instead of an entry) when browsing
    /// with no entry selected.
    pub(crate) fn show_journal_stats_preview(&self) -> bool {
        self.mode == Mode::Browse && self.selected_entry_index.is_none()
    }

    /// Whether the entries list should draw a highlighted selection row.
    pub(crate) fn entries_highlighted(&self) -> bool {
        self.focus != Focus::Journals && self.selected_entry_index.is_some()
    }

    pub(crate) fn journal_list_ensure_visible(&mut self, viewport_height: u16) {
        ensure_selected_visible(&mut self.journal_list, self.journals.len(), viewport_height);
    }

    pub(crate) fn journal_list_scroll(&mut self, delta: i16, viewport_height: u16) {
        scroll_list_offset(
            &mut self.journal_list,
            delta,
            self.journals.len(),
            viewport_height,
        );
    }

    fn reset_entry_scroll(&mut self) {
        *self.entry_list.offset_mut() = 0;
        self.scroll.reset_entry_view();
    }

    pub(crate) fn entry_list_scroll(
        &mut self,
        delta: i16,
        total_height: usize,
        viewport_height: u16,
    ) {
        let max = total_height.saturating_sub(viewport_height as usize);
        let offset = if delta < 0 {
            self.entry_list
                .offset()
                .saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.entry_list.offset().saturating_add(delta as usize)
        };
        *self.entry_list.offset_mut() = offset.min(max);
    }

    pub(crate) fn entry_list_ensure_visible(
        &mut self,
        rows: &[EntryRowMeta],
        viewport_height: u16,
    ) {
        let mut scroll = self.entry_list.offset() as u16;
        crate::tui::entry_rows::ensure_entry_visible(
            &mut scroll,
            rows,
            self.selected_entry_index,
            viewport_height,
        );
        *self.entry_list.offset_mut() = scroll as usize;
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
        if self.focus == Focus::Journals && self.mode == Mode::Browse {
            move_list_selection(&mut self.journal_list, len, delta);
            self.selected_entry_index = Some(0);
            *self.entry_list.offset_mut() = 0;
        } else {
            match self.selected_entry_index {
                // Deselected (Browse shows journal stats): a downward move selects
                // the first entry; an upward move stays on the stats view.
                None if self.mode == Mode::Browse => {
                    if delta > 0 {
                        self.selected_entry_index = Some(0);
                    }
                }
                // Scrolling up past the first entry deselects, revealing journal stats.
                Some(0) if self.mode == Mode::Browse && delta < 0 => {
                    self.selected_entry_index = None;
                }
                current => {
                    let base = current.unwrap_or(0) as isize;
                    let next = (base + delta).clamp(0, len as isize - 1) as usize;
                    self.selected_entry_index = Some(next);
                }
            }
        }
        if self.selected_entry_index != previous_entry_index {
            self.scroll.entry_view = 0;
        }
    }

    pub(crate) fn select_journal(&mut self, index: usize) {
        if index >= self.journals.len() {
            return;
        }

        if self.selected_journal_index() != index {
            self.journal_list.select(Some(index));
            self.selected_entry_index = Some(0);
            self.reset_entry_scroll();
        }
    }

    pub(crate) fn select_entry_index(&mut self, index: usize) {
        if index >= self.current_entry_list_len() {
            return;
        }

        if self.selected_entry_index != Some(index) {
            self.selected_entry_index = Some(index);
            self.scroll.entry_view = 0;
        }
    }

    pub(crate) fn select_entry_by_id(&mut self, id: &str, reset_entry_scroll: bool) -> bool {
        let index = match self.mode {
            Mode::Search => self.search.hits.iter().position(|hit| hit.id == id),
            Mode::Browse => self.journal_name_for_entry_id(id).and_then(|journal_name| {
                self.entries
                    .iter()
                    .filter(|entry| entry.journal == journal_name)
                    .position(|entry| entry.id == id)
            }),
        };
        let Some(index) = index else { return false };

        if self.selected_entry_index != Some(index) {
            self.selected_entry_index = Some(index);
        }
        if reset_entry_scroll {
            self.scroll.entry_view = 0;
        }
        true
    }

    fn journal_name_for_entry_id(&mut self, id: &str) -> Option<String> {
        let journal_name = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.journal.clone())?;
        let journal_index = self
            .journals
            .iter()
            .position(|journal| journal.name == journal_name)?;
        if self.selected_journal_index() != journal_index {
            self.journal_list.select(Some(journal_index));
            *self.entry_list.offset_mut() = 0;
        }
        Some(journal_name)
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let index = self.selected_entry_index?;
        let entries = self.selected_entries();
        entries.get(index).copied()
    }

    pub(crate) fn selected_search_hit(&self) -> Option<&SearchHit> {
        self.search.hits.get(self.selected_entry_index?)
    }

    pub(crate) fn selected_entry_target(&self) -> Option<EntryTarget> {
        match self.mode {
            Mode::Search => {
                let hit = self.selected_search_hit()?;
                let entry = self.entries.iter().find(|entry| entry.id == hit.id)?;
                Some(EntryTarget {
                    id: entry.id.clone(),
                    path: entry.path.clone(),
                    title: self.search_hit_label(hit),
                    locked: entry.encryption_state == EntryEncryptionState::EncryptedLocked,
                })
            }
            Mode::Browse => {
                let entry = self.selected_entry()?;
                Some(EntryTarget {
                    id: entry.id.clone(),
                    path: entry.path.clone(),
                    title: entry.display_label(),
                    locked: entry.encryption_state == EntryEncryptionState::EncryptedLocked,
                })
            }
        }
    }

    pub(crate) fn selected_entry_tags(&self) -> Vec<String> {
        self.selected_entry_metadata(MetadataKind::Tags)
    }

    pub(crate) fn selected_entry_people(&self) -> Vec<String> {
        self.selected_entry_metadata(MetadataKind::People)
    }

    pub(crate) fn selected_entry_activities(&self) -> Vec<String> {
        self.selected_entry_metadata(MetadataKind::Activities)
    }

    fn selected_entry_metadata(&self, kind: MetadataKind) -> Vec<String> {
        match self.mode {
            Mode::Search => self
                .selected_search_hit()
                .and_then(|hit| {
                    self.entries
                        .iter()
                        .find(|entry| entry.id == hit.id)
                        .map(|entry| metadata_values(entry, kind).to_vec())
                })
                .unwrap_or_default(),
            Mode::Browse => self
                .selected_entry()
                .map(|entry| metadata_values(entry, kind).to_vec())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn selected_entry_feelings(&self) -> Vec<String> {
        match self.mode {
            Mode::Search => self
                .selected_search_hit()
                .and_then(|hit| {
                    self.entries
                        .iter()
                        .find(|entry| entry.id == hit.id)
                        .map(|entry| entry.feelings.clone())
                })
                .unwrap_or_default(),
            Mode::Browse => self
                .selected_entry()
                .map(|entry| entry.feelings.clone())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn has_selected_entry_target(&self) -> bool {
        self.selected_entry_target().is_some()
    }

    pub(crate) fn can_act_on_selected_entry(&self) -> bool {
        matches!(self.focus, Focus::Entries | Focus::EntryView) && self.has_selected_entry_target()
    }

    pub(crate) fn selected_entry_view(&self) -> Option<(String, String)> {
        let entry = match self.mode {
            Mode::Search => {
                let hit = self.selected_search_hit()?;
                self.entries.iter().find(|entry| entry.id == hit.id)?
            }
            Mode::Browse => self.selected_entry()?,
        };
        if entry.encryption_state == EntryEncryptionState::EncryptedLocked {
            return Some((
                entry_timestamp_label(entry),
                "Encryption identity not available".to_string(),
            ));
        }
        Some((entry_timestamp_label(entry), entry.content.clone()))
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
        match self.mode {
            Mode::Search => self.selected_search_hit().and_then(|hit| {
                self.entries
                    .iter()
                    .find(|entry| entry.id == hit.id)
                    .and_then(|entry| entry.mood)
            }),
            Mode::Browse => self.selected_entry().and_then(|entry| entry.mood),
        }
    }

    pub(crate) fn begin_edit_mood(&mut self) {
        let saved = self.selected_entry_mood();
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
        match self.focus {
            Focus::Journals => self.begin_confirm_delete_journal(),
            Focus::Entries | Focus::EntryView => self.begin_confirm_delete_entry(),
        }
    }

    fn begin_confirm_delete_entry(&mut self) {
        let has_body = self
            .selected_entry()
            .map(|e| !e.content.trim().is_empty())
            .unwrap_or(false);
        self.overlay = Overlay::ConfirmDelete(DeleteContext::Entry { has_body });
    }

    fn begin_confirm_delete_journal(&mut self) {
        let Some(journal) = self.selected_journal() else {
            return;
        };
        let name = journal.name.clone();
        let trash_count = self
            .entries
            .iter()
            .filter(|e| e.journal == name && !e.content.trim().is_empty())
            .count();
        let delete_count = self
            .entries
            .iter()
            .filter(|e| e.journal == name && e.content.trim().is_empty())
            .count();
        self.overlay = Overlay::ConfirmDelete(DeleteContext::Journal {
            name,
            trash_count,
            delete_count,
        });
    }

    pub(crate) fn has_overlay(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    pub(crate) fn close_overlay(&mut self) {
        // Cache is scoped to the entry-viewing session, not the viewer overlay
        // (see `sync_image_warm`), so reopening within the same entry stays warm.
        self.overlay = Overlay::None;
    }

    /// Manage the image cache lifecycle. Called every tick. Warming is kicked off
    /// when the fullscreen viewer opens (not merely when the entry does), so we
    /// don't decode images the user may never look at. The cache then lives until
    /// the entry closes (or switches, or the target size changes), so reopening
    /// the viewer within the same entry stays instant.
    pub(crate) fn sync_image_warm(&mut self, terminal_size: Size) {
        let size = viewer_image_size(Rect::new(0, 0, terminal_size.width, terminal_size.height));
        // The entry currently open in the entry view, if any.
        let open_entry = self
            .selected_entry_target()
            .map(|target| target.path)
            .filter(|_| self.focus == Focus::EntryView);

        // Drop the cache when the entry that warmed it is no longer open (closed
        // or switched to another entry) or the viewer's target size changed.
        let stale = match &self.image_warm {
            Some((warmed_path, warmed_size)) => {
                open_entry.as_deref() != Some(warmed_path.as_path()) || *warmed_size != size
            }
            None => false,
        };
        if stale {
            self.images.clear();
            self.image_warm = None;
        }

        // Warm only once the viewer is actually opened. `image_warm` is `None`
        // here only when nothing valid is cached (a matching cache is never
        // stale), so this builds each entry's images at most once per session.
        if matches!(self.overlay, Overlay::ImageViewer(_))
            && self.image_warm.is_none()
            && let Some(path) = open_entry
        {
            let assets = self.selected_images();
            if !assets.is_empty() {
                self.images.warm(&assets, size);
                self.image_warm = Some((path, size));
            }
        }
    }

    /// Selected entry's referenced images in body order, memoized per entry path
    /// since hot callers hit it every render, keypress, and tick. Empty when no
    /// entry is selected or it has no in-folder images.
    fn selected_images(&self) -> Rc<Vec<ImageAsset>> {
        let target_path = self.selected_entry_target().map(|target| target.path);

        if let Some((path, images)) = self.selected_images_cache.borrow().as_ref()
            && target_path.as_deref() == Some(path.as_path())
        {
            return images.clone();
        }

        let images = Rc::new(match (&target_path, self.selected_entry_view()) {
            (Some(path), Some((_, content))) => entry_images(&content, path),
            _ => Vec::new(),
        });
        if let Some(path) = target_path {
            *self.selected_images_cache.borrow_mut() = Some((path, images.clone()));
        }
        images
    }

    /// Owned copy for the viewer overlay, which takes ownership. Prefer
    /// [`Self::selected_images`] on read-only paths.
    fn selected_entry_images(&self) -> Vec<ImageAsset> {
        (*self.selected_images()).clone()
    }

    /// In-folder image count for the selected entry; drives the `i` footer hint
    /// and the digit shortcuts.
    pub(crate) fn selected_entry_image_count(&self) -> usize {
        self.selected_images().len()
    }

    /// Open the fullscreen viewer on the selected entry's image at `index`
    /// (clamped); no-op when the entry has no images. Focuses the entry view
    /// first so the viewer is only ever open with `Focus::EntryView` — the
    /// invariant [`App::sync_image_warm`] relies on to own the cache lifecycle.
    pub(crate) fn begin_image_viewer(&mut self, index: usize) {
        let assets = self.selected_entry_images();
        if assets.is_empty() {
            return;
        }
        self.focus = Focus::EntryView;
        let index = index.min(assets.len() - 1);
        self.overlay = Overlay::ImageViewer(ImageViewerState { assets, index });
    }

    pub(crate) fn image_viewer_state(&self) -> Option<&ImageViewerState> {
        match &self.overlay {
            Overlay::ImageViewer(state) => Some(state),
            _ => None,
        }
    }

    /// Step the open viewer by `delta`, clamped at the ends.
    pub(crate) fn image_viewer_step(&mut self, delta: isize) {
        if let Overlay::ImageViewer(state) = &mut self.overlay {
            let len = state.assets.len();
            if len == 0 {
                return;
            }
            state.index = (state.index as isize + delta).clamp(0, len as isize - 1) as usize;
        }
    }

    /// Image index if `(col, row)` lands on a clickable image label in the entry
    /// view, using the positions captured at render time.
    pub(crate) fn image_label_at(&self, col: u16, row: u16) -> Option<usize> {
        let hits = &self.entry_view_image_hits;
        let rect = hits.content_rect;
        if rect.width == 0
            || rect.height == 0
            || col < rect.x
            || col >= rect.x + rect.width
            || row < rect.y
            || row >= rect.y + rect.height
        {
            return None;
        }
        let line_index = hits.scroll as usize + (row - rect.y) as usize;
        hits.labels
            .iter()
            .find(|(label_row, _)| *label_row == line_index)
            .map(|(_, image_index)| *image_index)
    }

    pub(crate) fn select_journal_by_name(&mut self, name: &str) {
        if let Some(index) = self
            .journals
            .iter()
            .position(|journal| journal.name == name)
        {
            self.journal_list.select(Some(index));
            *self.journal_list.offset_mut() = index;
            self.selected_entry_index = Some(0);
            self.reset_entry_scroll();
            self.focus = Focus::Entries;
        }
    }

    /// Collect metadata values across every loaded entry, sorted by usage count
    /// (most frequent first) and then alphabetically. Values differing only in
    /// case are consolidated: the most common casing wins (ties go to the
    /// first alphabetically).
    pub(crate) fn all_metadata_sorted(&self, kind: MetadataKind) -> Vec<(String, usize)> {
        // First pass — count per lowercased key, track casing frequency.
        let mut lower_to_casing: std::collections::BTreeMap<String, CasingCount> =
            std::collections::BTreeMap::new();
        for entry in &self.entries {
            for value in metadata_values(entry, kind) {
                let lower = value.to_lowercase();
                let entry = lower_to_casing.entry(lower).or_default();
                entry.total += 1;
                *entry.forms.entry(value.clone()).or_default() += 1;
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
        self.begin_edit_metadata(MetadataKind::Tags);
    }

    pub(crate) fn begin_edit_people(&mut self) {
        self.begin_edit_metadata(MetadataKind::People);
    }

    pub(crate) fn begin_edit_activities(&mut self) {
        self.begin_edit_metadata(MetadataKind::Activities);
    }

    fn begin_edit_metadata(&mut self, kind: MetadataKind) {
        let all_tags = self.all_metadata_sorted(kind);
        let filtered: Vec<usize> = (0..all_tags.len()).collect();
        let entry_tags: Vec<String> = self
            .selected_entry_metadata(kind)
            .into_iter()
            .map(|t| t.to_lowercase())
            .collect();
        self.overlay = Overlay::EditTags(EditTagState::new(kind, all_tags, filtered, entry_tags));
    }

    pub(crate) fn begin_edit_feelings(&mut self) {
        let selected = self.selected_entry_feelings();
        self.overlay = Overlay::EditFeelings(EditFeelingState::new(
            FEELINGS.iter().map(|feeling| feeling.to_string()).collect(),
            selected,
        ));
    }

    pub(crate) fn begin_tag_search(&mut self, tag: &str) {
        self.search.scope = self
            .selected_journal()
            .map(|journal| SearchScope::CurrentJournal(journal.name.clone()))
            .unwrap_or(SearchScope::AllJournals);
        self.mode = Mode::Search;
        self.focus = Focus::Entries;
        self.search.query = format!("tags:{tag}");
        self.search.hits = self.search_results_by_metadata(MetadataKind::Tags, tag);
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    pub(crate) fn begin_people_search(&mut self, person: &str) {
        self.begin_metadata_search(MetadataKind::People, person);
    }

    pub(crate) fn begin_activity_search(&mut self, activity: &str) {
        self.begin_metadata_search(MetadataKind::Activities, activity);
    }

    fn begin_metadata_search(&mut self, kind: MetadataKind, value: &str) {
        self.search.scope = self
            .selected_journal()
            .map(|journal| SearchScope::CurrentJournal(journal.name.clone()))
            .unwrap_or(SearchScope::AllJournals);
        self.mode = Mode::Search;
        self.focus = Focus::Entries;
        self.search.query = format!("{}:{value}", kind.search_prefix());
        self.search.hits = self.search_results_by_metadata(kind, value);
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    pub(crate) fn begin_feeling_search(&mut self, feeling: &str) {
        self.search.scope = self
            .selected_journal()
            .map(|journal| SearchScope::CurrentJournal(journal.name.clone()))
            .unwrap_or(SearchScope::AllJournals);
        self.mode = Mode::Search;
        self.focus = Focus::Entries;
        self.search.query = format!("feelings:{feeling}");
        self.search.hits = self.search_results_by_feeling(feeling);
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
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
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    pub(crate) fn exit_search(&mut self) {
        self.mode = Mode::Browse;
        self.search.scope = SearchScope::AllJournals;
        self.search.query.clear();
        self.search.hits.clear();
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    pub(crate) fn update_search_results(&mut self) {
        self.search.hits = self.search_results();
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
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
            self.search_results_by_metadata(MetadataKind::Tags, tag.trim())
        } else if let Some(person) = self.search.query.strip_prefix("people:") {
            self.search_results_by_metadata(MetadataKind::People, person.trim())
        } else if let Some(activity) = self.search.query.strip_prefix("activities:") {
            self.search_results_by_metadata(MetadataKind::Activities, activity.trim())
        } else if let Some(feeling) = self.search.query.strip_prefix("feelings:") {
            self.search_results_by_feeling(feeling.trim())
        } else {
            search_loaded_entries(
                &self.entries,
                &self.search.query,
                self.search.scope.filter(),
            )
        }
    }

    fn search_results_by_metadata(&self, kind: MetadataKind, query: &str) -> Vec<SearchHit> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                entry.encryption_state != EntryEncryptionState::EncryptedLocked
                    && metadata_values(entry, kind)
                        .iter()
                        .any(|value| value.to_lowercase().contains(&query_lower))
            })
            .filter(|entry| match self.search.scope {
                SearchScope::AllJournals => true,
                SearchScope::CurrentJournal(ref journal) => entry.journal == *journal,
            })
            .map(|entry| SearchHit {
                id: entry.id.clone(),
                journal: entry.journal.clone(),
                created_at: entry.created_at.clone(),
                title: entry.display_label(),
                preview: entry.preview.clone(),
            })
            .collect()
    }

    fn search_results_by_feeling(&self, feeling: &str) -> Vec<SearchHit> {
        let Some(feeling) = normalize_feeling(feeling) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|entry| {
                entry.encryption_state != EntryEncryptionState::EncryptedLocked
                    && entry
                        .feelings
                        .iter()
                        .any(|entry_feeling| entry_feeling == &feeling)
            })
            .filter(|entry| match self.search.scope {
                SearchScope::AllJournals => true,
                SearchScope::CurrentJournal(ref journal) => entry.journal == *journal,
            })
            .map(|entry| SearchHit {
                id: entry.id.clone(),
                journal: entry.journal.clone(),
                created_at: entry.created_at.clone(),
                title: entry.display_label(),
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

fn metadata_values(entry: &Entry, kind: MetadataKind) -> &[String] {
    match kind {
        MetadataKind::Tags => &entry.tags,
        MetadataKind::People => &entry.people,
        MetadataKind::Activities => &entry.activities,
    }
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
        let config_path = config.journal_root.join("config.toml");
        let store = JournalStore::for_config(&config_path, &config.journal_root).unwrap();
        App::new(config_path, config, store).unwrap()
    }

    #[test]
    fn changing_selected_entry_resets_entry_view_scroll() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();
        fs::write(entry_dir.join("b.md"), "+++\ntags = []\n+++\n\n# B\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;
        app.scroll.entry_view = 20;

        app.move_selection(1);

        assert_eq!(app.scroll.entry_view, 0);
    }

    #[test]
    fn scrolling_up_past_first_entry_deselects_and_shows_stats() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();
        fs::write(entry_dir.join("b.md"), "+++\ntags = []\n+++\n\n# B\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        assert_eq!(app.selected_entry_index, Some(0));

        // Up from the first entry deselects, revealing the journal stats preview.
        app.move_selection(-1);
        assert_eq!(app.selected_entry_index, None);
        assert!(app.show_journal_stats_preview());
        assert!(!app.entries_highlighted());
        assert!(app.selected_entry_target().is_none());

        // Down reselects the first entry.
        app.move_selection(1);
        assert_eq!(app.selected_entry_index, Some(0));
        assert!(!app.show_journal_stats_preview());
    }

    #[test]
    fn hidden_journals_launch_focuses_entries_with_stats_preview() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();

        let mut config = Config::new(dir.path().to_path_buf(), "true");
        config.show_journals = false;
        let app = new_app(config);

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, None);
        assert!(app.show_journal_stats_preview());
    }

    #[test]
    fn selected_entry_view_title_uses_entry_timestamp() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        let (title, content) = app.selected_entry_view().unwrap();

        assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
        assert_eq!(content, "# A\nBody\n");
    }

    #[test]
    fn search_entry_view_title_uses_entry_timestamp() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nneedle\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.begin_search();
        app.search.query = "needle".to_string();
        app.update_search_results();

        let (title, content) = app.selected_entry_view().unwrap();

        assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
        assert_eq!(content, "# A\nneedle\n");
    }

    #[test]
    fn journal_focus_does_not_make_entry_targets_actionable() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        app.focus = Focus::Journals;
        assert!(!app.can_act_on_selected_entry());

        app.focus = Focus::Entries;
        assert!(app.can_act_on_selected_entry());
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
    fn feelings_search_matches_exact_known_label() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\nfeelings = [\"calm\"]\n+++\n\n# A\n",
        )
        .unwrap();
        fs::write(
            entry_dir.join("b.md"),
            "+++\nfeelings = [\"anxious\"]\n+++\n\n# B\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.begin_search();
        app.search.query = "feelings:calm".to_string();
        app.update_search_results();

        assert_eq!(app.search.hits.len(), 1);
        assert_eq!(app.search.hits[0].title, "A");
    }

    #[test]
    fn begin_edit_feelings_uses_fixed_list_and_selected_entry_values() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\nfeelings = [\"calm\", \"focused\"]\n+++\n\n# A\n",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        app.begin_edit_feelings();

        let state = app.edit_feeling_state().unwrap();
        assert_eq!(state.all_feelings[0], "calm");
        assert_eq!(state.selected, vec!["calm", "focused"]);
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
