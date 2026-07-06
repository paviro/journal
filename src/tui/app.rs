use crate::{AppResult, config::Config};
use journal_core::feelings::{FEELINGS, normalize_feeling};
use journal_storage::{
    Entry, EntryEncryptionState, EntryPath, Journal, JournalStore, SearchHit, SearchScopeFilter,
    entry_timestamp_label, is_entry_file, search_loaded_entries,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use ratatui::{
    layout::{Rect, Size},
    text::Line,
    widgets::ListState,
};

use super::state::{
    DeleteContext, EditFeelingState, EditMoodState, EditTagState, ImageViewerState, MetadataKind,
    Overlay, ScrollState, SearchState, StatusBar, ensure_selected_visible, move_list_selection,
    normalize_list_state, scroll_list_offset,
};
use crate::tui::entry_rows::{EntryRowCache, EntryRowMeta, build_entry_row_cache};
use crate::tui::image::{ImageAsset, ImageRuntime, entry_images, viewer_image_size};
use crate::tui::render::stats::JournalStats;

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

/// Identifies the inputs that fully determine the entry-list rows, so a matching
/// key means the cached [`EntryRowCache`] can be reused. Notably excludes the
/// scroll offset and selected index — those are applied when drawing, not baked
/// into the rows.
#[derive(Clone, PartialEq, Eq)]
struct EntryRowKey {
    /// [`RenderCaches::rows_version`] — bumped whenever `entries` or
    /// `search.hits` change, since the rows are built from the hits in Search
    /// mode.
    version: u64,
    mode: Mode,
    journal: Option<String>,
    text_width: u16,
}

/// Rendered entry-body lines plus the clickable `(body line, image index)` label
/// positions — the output of the markdown parse/render pipeline, memoized because
/// it is the dominant per-frame cost of the preview pane.
pub(crate) type RenderedEntryBody = (Vec<Line<'static>>, Vec<(usize, usize)>);

/// Cache key for [`App::cached_entry_body`]: the rendered body is fully
/// determined by which entry is shown (`path` + `version`) and the wrap width.
/// The `version` is [`RenderCaches::entries_version`] — not the rows version —
/// because the body depends only on entry content, not on which search hits are
/// showing (a hit change that swaps the shown entry already changes `path`).
#[derive(Clone, PartialEq, Eq)]
struct EntryBodyKey {
    version: u64,
    path: Option<PathBuf>,
    width: usize,
}

/// The per-frame render memo caches and the version counters that invalidate
/// them. Grouped so `App` carries one field instead of four and the versions
/// have a single home. All three caches are read on the `&self` render/hit-test
/// paths, so each is a `RefCell`.
///
/// Two counters, because the caches have different dependencies:
/// - [`Self::entries_version`] bumps only when the `entries` Vec changes. It
///   keys the body and stats caches, which depend on entry content alone.
/// - [`Self::rows_version`] bumps when entries **or** search hits change. It
///   keys the row cache, which is built from the hits in Search mode.
///
/// A search recompute therefore bumps only `rows_version`, so the (more
/// expensive) rendered-body and journal-stats memos survive keystroke-driven
/// query edits instead of rebuilding every time.
#[derive(Default)]
struct RenderCaches {
    /// Memoized entry-list rows, keyed by [`EntryRowKey`].
    entry_row_cache: RefCell<Option<(EntryRowKey, Rc<EntryRowCache>)>>,
    /// Memoized rendered body lines for the entry preview, keyed by
    /// [`EntryBodyKey`]. Rebuilt only when the shown entry or wrap width changes,
    /// so scroll/blink/image ticks reuse it.
    entry_body_cache: RefCell<Option<(EntryBodyKey, Rc<RenderedEntryBody>)>>,
    /// Memoized journal-stats aggregate for the `(entries_version, journal)` it
    /// was computed for, so the stats preview doesn't rescan the journal's
    /// entries (with a date parse each) every frame.
    journal_stats_cache: RefCell<Option<(u64, String, Rc<JournalStats>)>>,
    entries_version: u64,
    rows_version: u64,
}

impl RenderCaches {
    /// The `entries` Vec changed: both the entries-keyed (body, stats) and
    /// rows-keyed caches are stale.
    fn bump_entries(&mut self) {
        self.entries_version = self.entries_version.wrapping_add(1);
        self.rows_version = self.rows_version.wrapping_add(1);
    }

    /// Only the entry-list rows changed (a search recompute); the body and stats
    /// caches, keyed on [`Self::entries_version`], stay valid.
    fn bump_rows(&mut self) {
        self.rows_version = self.rows_version.wrapping_add(1);
    }

    /// Return the memoized rows for `key`, building them with `build` on a miss.
    fn rows(&self, key: EntryRowKey, build: impl FnOnce() -> EntryRowCache) -> Rc<EntryRowCache> {
        if let Some((cached_key, cache)) = self.entry_row_cache.borrow().as_ref()
            && *cached_key == key
        {
            return cache.clone();
        }
        let cache = Rc::new(build());
        *self.entry_row_cache.borrow_mut() = Some((key, cache.clone()));
        cache
    }

    /// Return the memoized rendered body for `key`, building it with `build` on a
    /// miss (entry or width changed, or the store reloaded).
    fn body(
        &self,
        key: EntryBodyKey,
        build: impl FnOnce() -> RenderedEntryBody,
    ) -> Rc<RenderedEntryBody> {
        if let Some((cached_key, body)) = self.entry_body_cache.borrow().as_ref()
            && *cached_key == key
        {
            return body.clone();
        }
        let body = Rc::new(build());
        *self.entry_body_cache.borrow_mut() = Some((key, body.clone()));
        body
    }

    /// Return the memoized stats for `journal` at `version`, building them with
    /// `build` on a miss (different journal selected, or the store reloaded).
    fn stats(
        &self,
        version: u64,
        journal: &str,
        build: impl FnOnce() -> JournalStats,
    ) -> Rc<JournalStats> {
        if let Some((cached_version, name, stats)) = self.journal_stats_cache.borrow().as_ref()
            && *cached_version == version
            && name == journal
        {
            return stats.clone();
        }
        let stats = Rc::new(build());
        *self.journal_stats_cache.borrow_mut() =
            Some((version, journal.to_string(), stats.clone()));
        stats
    }
}

pub(crate) struct App {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Config,
    pub(crate) store: JournalStore,
    pub(crate) journals: Vec<Journal>,
    pub(crate) entries: Vec<Entry>,
    /// Journal name → contiguous index range into `entries`. Entries are sorted
    /// by path (so a journal's entries are adjacent); this lets `selected_entries`
    /// and the entry count avoid re-scanning the whole `entries` Vec each call.
    journal_ranges: HashMap<String, Range<usize>>,
    /// Entry id → index into `entries`, rebuilt whenever `entries` is reloaded.
    /// In Search mode the preview getters resolve a hit's `&Entry` through this
    /// instead of an O(entries) `iter().find`, so a single frame no longer does
    /// several full linear scans.
    entry_index_by_id: HashMap<String, usize>,
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
    /// Blink phase of the search caret; toggled on a timer by the event loop and
    /// read when rendering the search field. `true` = caret block shown.
    pub(crate) search_cursor_visible: bool,
    pub(crate) overlay: Overlay,
    pub(crate) status_bar: StatusBar,
    pub(crate) images: ImageRuntime,
    /// `(entry_path, viewer_size)` the image cache is warmed for, or `None` when
    /// no entry view is open. Compared against the desired context each tick.
    image_warm: Option<(PathBuf, Size)>,
    /// Clickable `[Image N …]` label positions from the last entry-view render.
    pub(crate) entry_view_image_hits: EntryViewImageHits,
    /// Which pane's scrollbar is currently being dragged, if any. Set on press,
    /// cleared on release; lets a drag keep scrolling even after the cursor drifts
    /// off the one-column bar.
    pub(crate) scrollbar_drag: Option<ScrollbarDrag>,
    /// Rows between the top of the thumb and the point where it was grabbed, so the
    /// grabbed point tracks the cursor during a scrollbar drag.
    pub(crate) scrollbar_grab: u16,
    /// Selected entry's in-folder images, memoized by path; `RefCell` so `&self`
    /// render/hint/shortcut paths can read it. Re-parsed on a path change or when
    /// `refresh` clears it. Part of the image subsystem (grouped with the fields
    /// above), not [`RenderCaches`]: it is path-keyed with manual invalidation
    /// (`.take()` on reload) and tied to the `ImageRuntime` lifecycle, rather than
    /// invalidated by a version counter.
    selected_images_cache: RefCell<Option<(PathBuf, Rc<Vec<ImageAsset>>)>>,
    /// Per-frame render memo caches (rows, rendered body, journal stats) and the
    /// version counters that invalidate them. See [`RenderCaches`].
    caches: RenderCaches,
    /// Set when the search query changed but the (expensive) hit recompute has
    /// been deferred; the event loop runs it once typing pauses (debounce).
    pub(crate) search_dirty: bool,
    /// Timestamp of the last search keystroke, for the debounce window.
    pub(crate) search_last_edit: Option<Instant>,
}

/// Clickable image label positions in the entry view, captured at render time so
/// the mouse handler can map a click back to an image index.
#[derive(Default)]
pub(crate) struct EntryViewImageHits {
    pub(crate) content_rect: Rect,
    pub(crate) scroll: u16,
    /// Total rendered body line count, for mapping a scrollbar drag to a scroll offset.
    pub(crate) line_count: usize,
    /// `(body line index, image index)` per label line.
    pub(crate) labels: Vec<(usize, usize)>,
}

/// Which pane's vertical scrollbar a mouse drag is currently manipulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollbarDrag {
    Journals,
    EntryList,
    EntryView,
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
            journal_ranges: HashMap::new(),
            entry_index_by_id: HashMap::new(),
            journal_list: ListState::default(),
            selected_entry_index: None,
            entry_list: ListState::default(),
            scroll: ScrollState::default(),
            focus: Focus::Journals,
            mode: Mode::Browse,
            search: SearchState::default(),
            search_cursor_visible: true,
            overlay: Overlay::None,
            status_bar: StatusBar::default(),
            images: ImageRuntime::default(),
            image_warm: None,
            entry_view_image_hits: EntryViewImageHits::default(),
            scrollbar_drag: None,
            scrollbar_grab: 0,
            selected_images_cache: RefCell::new(None),
            caches: RenderCaches::default(),
            search_dirty: false,
            search_last_edit: None,
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
        self.after_entries_changed();
        Ok(())
    }

    /// Rebuild derived state after `entries` is replaced or edited: the entry
    /// indexes, the cache-invalidating data version, the search hits (when a
    /// query is active), and the clamped selection. Shared by the full load and
    /// the incremental [`Self::refresh_paths`] path.
    fn after_entries_changed(&mut self) {
        self.rebuild_entry_indexes();
        // Entries (and possibly hits) changed: invalidate every version-keyed
        // cache — the body/stats caches (entries_version) and the row cache
        // (rows_version).
        self.caches.bump_entries();
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
    }

    /// Reload only the entries under the changed `paths` when every change is an
    /// entry-file upsert/remove inside an existing journal. Anything else — a
    /// new or removed journal, an asset, or a directory event — falls back to a
    /// full [`Self::refresh`], since the journal list or grouping may have moved.
    pub(crate) fn refresh_paths(&mut self, paths: &[PathBuf]) -> AppResult<()> {
        self.store.ensure()?;
        let root = self.store.paths().journal_root.clone();

        // notify frequently reports the same path several times per change.
        let mut changed = paths.to_vec();
        changed.sort();
        changed.dedup();

        let mut targets: Vec<(String, PathBuf)> = Vec::new();
        for path in &changed {
            let Some(journal) = journal_for_path(&root, path) else {
                return self.refresh();
            };
            if !is_entry_file(path) || !self.journals.iter().any(|j| j.name == journal) {
                return self.refresh();
            }
            targets.push((journal, path.clone()));
        }
        if targets.is_empty() {
            return Ok(());
        }

        // The viewed entry's body/images may have changed: drop the image caches
        // (the version-keyed row/body/stats caches self-invalidate on the bump).
        self.images.clear();
        self.image_warm = None;
        self.selected_images_cache.borrow_mut().take();

        for (journal, path) in targets {
            if path.exists() {
                let entry = self.store.read_entry(&journal, &path)?;
                self.upsert_entry(entry);
            } else {
                self.remove_entry_by_path(&path);
            }
        }
        self.after_entries_changed();
        Ok(())
    }

    /// Insert or replace `entry` in the path-sorted (descending) `entries` Vec,
    /// keeping the ordering so `journal_ranges` stays contiguous per journal.
    fn upsert_entry(&mut self, entry: Entry) {
        match self
            .entries
            .binary_search_by(|existing| entry.path.cmp(&existing.path))
        {
            Ok(index) => self.entries[index] = entry,
            Err(index) => self.entries.insert(index, entry),
        }
    }

    /// Remove the entry at `path`, if present, preserving the sorted order.
    fn remove_entry_by_path(&mut self, path: &Path) {
        if let Ok(index) = self
            .entries
            .binary_search_by(|existing| path.cmp(&existing.path))
        {
            self.entries.remove(index);
        }
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
        let mut scroll = self.entry_list.offset();
        crate::tui::entry_rows::ensure_entry_visible(
            &mut scroll,
            rows,
            self.selected_entry_index,
            viewport_height,
        );
        *self.entry_list.offset_mut() = scroll;
    }

    /// Contiguous index range into `entries` for the selected journal, or `None`
    /// when no journal is selected or it has no entries.
    fn selected_entry_range(&self) -> Option<Range<usize>> {
        let journal = self.selected_journal()?;
        self.journal_ranges.get(&journal.name).cloned()
    }

    pub(crate) fn selected_entries(&self) -> Vec<&Entry> {
        match self.selected_entry_range() {
            Some(range) => self.entries[range].iter().collect(),
            None => Vec::new(),
        }
    }

    pub(crate) fn current_entry_list_len(&self) -> usize {
        match self.mode {
            Mode::Search => self.search.hits.len(),
            Mode::Browse => self.selected_entry_range().map_or(0, |range| range.len()),
        }
    }

    /// Rebuild the derived entry indexes after `entries` is (re)loaded: the
    /// journal → contiguous-range map (entries are sorted by path, so each
    /// journal's entries form one run) and the entry-id → index map used to
    /// resolve search hits without scanning.
    fn rebuild_entry_indexes(&mut self) {
        // Both indexes rely on `entries` being sorted descending by path (see
        // `read_entries`), which keeps each journal's entries in one contiguous
        // run. Trip loudly if a future code path ever breaks that ordering.
        debug_assert!(
            self.entries.windows(2).all(|w| w[0].path >= w[1].path),
            "entries must stay sorted descending by path for journal_ranges to be contiguous"
        );
        self.journal_ranges.clear();
        self.entry_index_by_id.clear();
        self.entry_index_by_id.reserve(self.entries.len());
        let mut start = 0;
        while start < self.entries.len() {
            let name = &self.entries[start].journal;
            let mut end = start + 1;
            while end < self.entries.len() && &self.entries[end].journal == name {
                end += 1;
            }
            self.journal_ranges.insert(name.clone(), start..end);
            start = end;
        }
        for (index, entry) in self.entries.iter().enumerate() {
            self.entry_index_by_id.insert(entry.id.clone(), index);
        }
    }

    /// Resolve an entry by id in O(1) via [`Self::entry_index_by_id`].
    fn entry_by_id(&self, id: &str) -> Option<&Entry> {
        self.entries.get(*self.entry_index_by_id.get(id)?)
    }

    /// The entry backing the current selection, resolving a search hit through
    /// the id index. Unifies the Search/Browse branches the preview getters share.
    fn resolved_selected_entry(&self) -> Option<&Entry> {
        match self.mode {
            Mode::Search => self.entry_by_id(&self.selected_search_hit()?.id),
            Mode::Browse => self.selected_entry(),
        }
    }

    /// The memoized entry-list rows for the current state, rebuilt only when the
    /// row-determining inputs (rows version, mode, journal, width) change. Returns
    /// an `Rc` so callers can read it while holding a `&mut App` borrow elsewhere.
    pub(crate) fn entry_rows(&self, text_width: u16) -> Rc<EntryRowCache> {
        let key = EntryRowKey {
            version: self.caches.rows_version,
            mode: self.mode.clone(),
            journal: self.selected_journal().map(|journal| journal.name.clone()),
            text_width,
        };
        self.caches
            .rows(key, || build_entry_row_cache(self, text_width))
    }

    /// Return the memoized rendered body for the entry at `path`/`width`, building
    /// it with `build` only on a cache miss (entry or width changed, or the store
    /// reloaded). The markdown parse+render `build` runs is the preview pane's
    /// dominant per-frame cost, so this keeps blink/scroll/image-tick redraws cheap.
    pub(crate) fn cached_entry_body(
        &self,
        path: Option<&Path>,
        width: usize,
        build: impl FnOnce() -> RenderedEntryBody,
    ) -> Rc<RenderedEntryBody> {
        let key = EntryBodyKey {
            version: self.caches.entries_version,
            path: path.map(Path::to_path_buf),
            width,
        };
        self.caches.body(key, build)
    }

    /// Precomputed word count of the entry currently shown in the preview, or 0
    /// when none is selected.
    pub(crate) fn selected_entry_word_count(&self) -> usize {
        self.resolved_selected_entry()
            .map_or(0, |entry| entry.word_count)
    }

    /// Return the memoized stats for `journal`, building them with `build` only on
    /// a cache miss (different journal selected, or the store reloaded).
    pub(crate) fn cached_journal_stats(
        &self,
        journal: &str,
        build: impl FnOnce() -> JournalStats,
    ) -> Rc<JournalStats> {
        self.caches
            .stats(self.caches.entries_version, journal, build)
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
        let range = self.selected_entry_range()?;
        (index < range.len()).then(|| &self.entries[range.start + index])
    }

    pub(crate) fn selected_search_hit(&self) -> Option<&SearchHit> {
        self.search.hits.get(self.selected_entry_index?)
    }

    pub(crate) fn selected_entry_target(&self) -> Option<EntryTarget> {
        // In Search mode the title comes from the hit (journal-prefixed label),
        // otherwise from the entry itself; the rest is shared.
        let title = match self.mode {
            Mode::Search => self.search_hit_label(self.selected_search_hit()?),
            Mode::Browse => self.selected_entry()?.display_label(),
        };
        let entry = self.resolved_selected_entry()?;
        Some(EntryTarget {
            id: entry.id.clone(),
            path: entry.path.clone(),
            title,
            locked: entry.encryption_state == EntryEncryptionState::EncryptedLocked,
        })
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
        self.resolved_selected_entry()
            .map(|entry| metadata_values(entry, kind).to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn selected_entry_feelings(&self) -> Vec<String> {
        self.resolved_selected_entry()
            .map(|entry| entry.feelings.clone())
            .unwrap_or_default()
    }

    pub(crate) fn has_selected_entry_target(&self) -> bool {
        self.selected_entry_target().is_some()
    }

    pub(crate) fn can_act_on_selected_entry(&self) -> bool {
        matches!(self.focus, Focus::Entries | Focus::EntryView) && self.has_selected_entry_target()
    }

    pub(crate) fn selected_entry_view(&self) -> Option<(String, String)> {
        let entry = self.resolved_selected_entry()?;
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
        self.resolved_selected_entry().and_then(|entry| entry.mood)
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
        self.search.cursor = self.search.query.chars().count();
        self.search.hits = self.search_results_by_metadata(MetadataKind::Tags, tag);
        self.search_dirty = false;
        self.search_last_edit = None;
        self.caches.bump_rows();
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
        self.search.cursor = self.search.query.chars().count();
        self.search.hits = self.search_results_by_metadata(kind, value);
        self.search_dirty = false;
        self.search_last_edit = None;
        self.caches.bump_rows();
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
        self.search.cursor = self.search.query.chars().count();
        self.search.hits = self.search_results_by_feeling(feeling);
        self.search_dirty = false;
        self.search_last_edit = None;
        self.caches.bump_rows();
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
        self.search.cursor = 0;
        self.search.hits.clear();
        self.search_dirty = false;
        self.search_last_edit = None;
        self.caches.bump_rows();
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    pub(crate) fn exit_search(&mut self) {
        self.mode = Mode::Browse;
        self.search.scope = SearchScope::AllJournals;
        self.search.query.clear();
        self.search.cursor = 0;
        self.search.hits.clear();
        self.search_dirty = false;
        self.search_last_edit = None;
        self.caches.bump_rows();
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    pub(crate) fn update_search_results(&mut self) {
        self.search.hits = self.search_results();
        self.search_dirty = false;
        self.search_last_edit = None;
        self.caches.bump_rows();
        self.selected_entry_index = Some(0);
        self.reset_entry_scroll();
    }

    /// Mark the search query as changed without running the (expensive) hit
    /// recompute yet. The event loop calls [`Self::update_search_results`] once
    /// typing pauses, so a fast typist doesn't re-scan the whole corpus per key.
    fn mark_search_dirty(&mut self) {
        self.search_dirty = true;
        self.search_last_edit = Some(Instant::now());
    }

    /// The search caret is active (blinking) only while typing in the field.
    pub(crate) fn is_search_input_active(&self) -> bool {
        self.mode == Mode::Search && self.focus == Focus::Entries
    }

    /// Byte offset in `query` for the current caret char index, clamped to the end.
    fn search_cursor_byte(&self) -> usize {
        self.search
            .query
            .char_indices()
            .nth(self.search.cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(self.search.query.len())
    }

    /// Insert a typed char at the caret and advance it.
    pub(crate) fn search_insert(&mut self, ch: char) {
        let byte = self.search_cursor_byte();
        self.search.query.insert(byte, ch);
        self.search.cursor += 1;
        self.mark_search_dirty();
    }

    /// Delete the char before the caret (Backspace).
    pub(crate) fn search_backspace(&mut self) {
        if self.search.cursor == 0 {
            return;
        }
        self.search.cursor -= 1;
        let byte = self.search_cursor_byte();
        self.search.query.remove(byte);
        self.mark_search_dirty();
    }

    pub(crate) fn search_cursor_left(&mut self) {
        self.search.cursor = self.search.cursor.saturating_sub(1);
    }

    pub(crate) fn search_cursor_right(&mut self) {
        let max = self.search.query.chars().count();
        self.search.cursor = (self.search.cursor + 1).min(max);
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

/// The journal owning `path`: the first path component beneath `root`. `None`
/// when `path` is not under `root` or has no such component (e.g. the root
/// itself), which signals the incremental path to fall back to a full reload.
fn journal_for_path(root: &Path, path: &Path) -> Option<String> {
    match path.strip_prefix(root).ok()?.components().next()? {
        std::path::Component::Normal(name) => name.to_str().map(str::to_string),
        _ => None,
    }
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

    #[test]
    fn entry_rows_cache_is_reused_until_inputs_change() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();
        fs::write(
            entry_dir.join("b.md"),
            "+++\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nBody\n",
        )
        .unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        let first = app.entry_rows(30);
        // Same inputs → same cached rows (identity, not just equality).
        assert!(Rc::ptr_eq(&first, &app.entry_rows(30)));
        // Moving the selection does not change the rows, so the cache holds.
        app.move_selection(1);
        assert!(Rc::ptr_eq(&first, &app.entry_rows(30)));
        // A different width rebuilds.
        assert!(!Rc::ptr_eq(&first, &app.entry_rows(20)));
        // Reloading the store rebuilds.
        app.refresh().unwrap();
        assert!(!Rc::ptr_eq(&first, &app.entry_rows(30)));
    }

    #[test]
    fn search_insert_defers_hit_recompute_until_committed() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nneedle\n",
        )
        .unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.begin_search();

        for ch in "needle".chars() {
            app.search_insert(ch);
        }
        // The query echoes immediately, but the whole-corpus scan is deferred.
        assert_eq!(app.search.query, "needle");
        assert!(app.search_dirty);
        assert!(app.search.hits.is_empty());

        // Committing (what the event loop does after the debounce) runs the scan.
        app.update_search_results();
        assert!(!app.search_dirty);
        assert_eq!(app.search.hits.len(), 1);
    }

    fn write_entry(dir: &std::path::Path, name: &str, created: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(
            &path,
            format!("+++\ncreated_at = \"{created}\"\n+++\n\n{body}\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn refresh_paths_updates_only_the_changed_entry() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let a = write_entry(
            &entry_dir,
            "a.md",
            "2026-07-01T10:00:00+02:00",
            "# A\nold body",
        );
        write_entry(&entry_dir, "b.md", "2026-07-01T11:00:00+02:00", "# B\nbee");
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        assert_eq!(app.entries.len(), 2);

        // Edit a.md on disk, then reload just that path.
        write_entry(
            &entry_dir,
            "a.md",
            "2026-07-01T10:00:00+02:00",
            "# A\nnew body here",
        );
        app.refresh_paths(&[a]).unwrap();

        assert_eq!(app.entries.len(), 2);
        let updated = app.entry_by_id("a").unwrap();
        assert!(updated.content.contains("new body here"));
        // Precomputed word count is rebuilt from the fresh body on re-read.
        assert_eq!(
            updated.word_count,
            updated.content.split_whitespace().count()
        );
        assert!(!updated.search_haystack.is_empty());
        // `entries` stays sorted by path (descending) so `journal_ranges` holds.
        assert!(app.entries.windows(2).all(|w| w[0].path > w[1].path));
        assert_eq!(app.selected_entries().len(), 2);
    }

    #[test]
    fn refresh_paths_handles_create_and_delete() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        let a = write_entry(
            &entry_dir,
            "a.md",
            "2026-07-01T10:00:00+02:00",
            "# A\nalpha",
        );
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        assert_eq!(app.entries.len(), 1);

        // A newly written file is picked up by its path alone.
        let c = write_entry(&entry_dir, "c.md", "2026-07-01T12:00:00+02:00", "# C\nsea");
        app.refresh_paths(std::slice::from_ref(&c)).unwrap();
        assert_eq!(app.entries.len(), 2);
        assert!(app.entry_by_id("c").is_some());

        // Deleting the file on disk removes it on the next targeted reload.
        fs::remove_file(&a).unwrap();
        app.refresh_paths(&[a]).unwrap();
        assert_eq!(app.entries.len(), 1);
        assert!(app.entry_by_id("a").is_none());
        assert_eq!(app.selected_entries().len(), 1);
    }

    #[test]
    fn refresh_paths_falls_back_to_full_reload_for_a_new_journal() {
        let dir = tempdir().unwrap();
        let work = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&work).unwrap();
        write_entry(&work, "a.md", "2026-07-01T10:00:00+02:00", "# A\nalpha");
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        // A path under a brand-new journal isn't attributable to a known journal,
        // so the incremental path must fall back to a full reload that also picks
        // up the new journal in the list.
        let personal = dir.path().join("personal").join("2026-07-01");
        fs::create_dir_all(&personal).unwrap();
        let z = write_entry(&personal, "z.md", "2026-07-02T10:00:00+02:00", "# Z\nzed");
        app.refresh_paths(&[z]).unwrap();

        assert!(
            app.journals
                .iter()
                .any(|journal| journal.name == "personal")
        );
        assert!(app.entry_by_id("z").is_some());
    }

    #[test]
    fn entry_body_cache_is_reused_until_entry_or_width_changes() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        write_entry(&entry_dir, "a.md", "2026-07-01T10:00:00+02:00", "# A\nBody");
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        let path = app.selected_entry_target().map(|target| target.path);

        let first = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("x")], vec![]));
        // Same entry + width → cached rows returned, the builder isn't re-run.
        let same = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("y")], vec![]));
        assert!(Rc::ptr_eq(&first, &same));
        // A different width rebuilds.
        let narrower =
            app.cached_entry_body(path.as_deref(), 20, || (vec![Line::from("z")], vec![]));
        assert!(!Rc::ptr_eq(&first, &narrower));
        // Reloading the store bumps entries_version, invalidating the cache.
        app.refresh().unwrap();
        let after = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("w")], vec![]));
        assert!(!Rc::ptr_eq(&first, &after));
    }

    #[test]
    fn search_recompute_keeps_body_and_stats_caches_but_rebuilds_rows() {
        use crate::tui::render::stats::JournalStats;

        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        write_entry(&entry_dir, "a.md", "2026-07-01T10:00:00+02:00", "# A\nbody");
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        let path = app.selected_entry_target().map(|target| target.path);

        // Prime all three caches.
        let body = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("x")], vec![]));
        let stats = app.cached_journal_stats("work", || JournalStats {
            name: "work".to_string(),
            entry_count: 0,
            active_days: 0,
            year_range: String::new(),
        });
        let rows = app.entry_rows(30);

        // A search recompute changes the hits but not the entries, so it bumps
        // only rows_version.
        app.begin_search();
        for ch in "body".chars() {
            app.search_insert(ch);
        }
        app.update_search_results();

        // Body and stats caches key on entries_version, which is untouched:
        // requerying with identical inputs returns the same Rc (builder skipped).
        let body_after =
            app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("y")], vec![]));
        assert!(Rc::ptr_eq(&body, &body_after));
        let stats_after = app.cached_journal_stats("work", || JournalStats {
            name: "work".to_string(),
            entry_count: 99,
            active_days: 99,
            year_range: "changed".to_string(),
        });
        assert!(Rc::ptr_eq(&stats, &stats_after));

        // The row cache keys on rows_version, which the recompute bumped, so it
        // rebuilt.
        let rows_after = app.entry_rows(30);
        assert!(!Rc::ptr_eq(&rows, &rows_after));
    }
}
