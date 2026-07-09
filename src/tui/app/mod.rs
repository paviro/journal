use crate::{
    AppResult,
    config::{Config, State},
};
use journal_core::feelings::{FEELING_GROUPS, normalize_feeling};
use journal_core::{
    Entry, EntryEncryptionState, EntryPath, SearchHit, entry_group_date, search_loaded_entries,
};
use journal_storage::{Journal, JournalStore, entry_timestamp_label, is_entry_file};
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
    DeleteContext, EditFeelingState, EditMetadataState, EditMoodState, ImageViewerState,
    MetadataKind, Overlay, ScrollState, SearchState, StatusBar, move_list_selection,
};
use crate::tui::entry_rows::{EntryRowCache, RowMeta, build_entry_row_cache};
use crate::tui::image::{ImageAsset, ImageRuntime, entry_images, viewer_image_size};
use crate::tui::render::insights::{InsightsScope, InsightsTab, InsightsTimeframe};
use journal_analytics::{Analytics, Correlations, analyze, build_correlations};

pub(crate) const JOURNAL_LIST_WIDTH: u16 = 27;
pub(crate) const ENTRY_LIST_INLINE_WIDTH: u16 = 47;
pub(crate) const ENTRY_LIST_MIN_WIDTH: u16 = 40;
pub(crate) const TWO_PANEL_MIN_WIDTH: u16 = 87;
pub(crate) const INLINE_ENTRY_VIEW_MIN_WIDTH: u16 = 125;

/// Rows moved per PageUp/PageDown, as a multiple of a single-line scroll.
const PAGE_STEP: i16 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Journals,
    Entries,
    EntryView,
    /// The journal insights panel — the right-hand column when no entry is
    /// selected. Reached with Right past Entries; its Left/Right cycle tabs.
    Insights,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Browse,
    Search,
}

pub(crate) use journal_core::SearchScope;

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
///   keys the body and analytics caches, which depend on entry content alone.
/// - [`Self::rows_version`] bumps when entries **or** search hits change. It
///   keys the row cache, which is built from the hits in Search mode.
///
/// A search recompute therefore bumps only `rows_version`, so the (more
/// expensive) rendered-body and journal-insights memos survive keystroke-driven
/// query edits instead of rebuilding every time.
#[derive(Default)]
struct RenderCaches {
    /// Memoized entry-list rows, keyed by [`EntryRowKey`].
    entry_row_cache: RefCell<Option<(EntryRowKey, Rc<EntryRowCache>)>>,
    /// Memoized rendered body lines for the entry preview, keyed by
    /// [`EntryBodyKey`]. Rebuilt only when the shown entry or wrap width changes,
    /// so scroll/blink/image ticks reuse it.
    entry_body_cache: RefCell<Option<(EntryBodyKey, Rc<RenderedEntryBody>)>>,
    /// Memoized analytics for the `(entries_version, scope key)` they were
    /// computed for. The scope key is the journal name for `Journal` scope or a
    /// sentinel for `All`, so switching tab/scope reuses the build instead of
    /// rescanning every entry each frame.
    analytics_cache: RefCell<Option<(u64, String, Rc<Analytics>)>>,
    /// Memoized correlations for a *windowed* slice of the scope, keyed by
    /// `(entries_version, scope key, timeframe)`. Separate from `analytics_cache`
    /// because it recomputes against the window's own baseline mean (so `mood_delta`
    /// answers "what lifts/drains me *this week*"), and only when the Drivers tab
    /// needs it.
    windowed_cache: RefCell<WindowedCache>,
    entries_version: u64,
    rows_version: u64,
}

/// The windowed-correlations memo: `(entries_version, scope key, timeframe)` and
/// the correlations built for them.
type WindowedCache = Option<(u64, String, InsightsTimeframe, Rc<Correlations>)>;

impl RenderCaches {
    /// The `entries` Vec changed: both the entries-keyed (body, analytics) and
    /// rows-keyed caches are stale.
    fn bump_entries(&mut self) {
        self.entries_version = self.entries_version.wrapping_add(1);
        self.rows_version = self.rows_version.wrapping_add(1);
    }

    /// Only the entry-list rows changed (a search recompute); the body and
    /// analytics caches, keyed on [`Self::entries_version`], stay valid.
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

    /// Return the memoized analytics for `scope_key` at `version`, building them
    /// with `build` on a miss (scope/journal changed, or the store reloaded).
    fn analytics(
        &self,
        version: u64,
        scope_key: &str,
        build: impl FnOnce() -> Analytics,
    ) -> Rc<Analytics> {
        if let Some((cached_version, key, analytics)) = self.analytics_cache.borrow().as_ref()
            && *cached_version == version
            && key == scope_key
        {
            return analytics.clone();
        }
        let analytics = Rc::new(build());
        *self.analytics_cache.borrow_mut() =
            Some((version, scope_key.to_string(), analytics.clone()));
        analytics
    }

    /// Return the memoized windowed correlations for `(version, scope_key,
    /// timeframe)`, building them with `build` on a miss (window, scope, or
    /// entries changed).
    fn windowed(
        &self,
        version: u64,
        scope_key: &str,
        timeframe: InsightsTimeframe,
        build: impl FnOnce() -> Correlations,
    ) -> Rc<Correlations> {
        if let Some((cached_version, key, cached_tf, correlations)) =
            self.windowed_cache.borrow().as_ref()
            && *cached_version == version
            && key == scope_key
            && *cached_tf == timeframe
        {
            return correlations.clone();
        }
        let correlations = Rc::new(build());
        *self.windowed_cache.borrow_mut() = Some((
            version,
            scope_key.to_string(),
            timeframe,
            correlations.clone(),
        ));
        correlations
    }
}

/// The entry-view image subsystem: the terminal-image runtime plus the caches
/// keyed by entry path (rather than by a [`RenderCaches`] version counter),
/// invalidated together when the open entry changes or the store reloads.
#[derive(Default)]
pub(crate) struct ImageState {
    pub(crate) runtime: ImageRuntime,
    /// `(entry_path, viewer_size)` the runtime is warmed for, or `None` when no
    /// entry view is open. Compared against the desired context each tick.
    warm: Option<(PathBuf, Size)>,
    /// Selected entry's in-folder images, memoized by path; `RefCell` so `&self`
    /// render/hint/shortcut paths can read it. Re-parsed on a path change or when
    /// `refresh` clears it.
    selected_cache: RefCell<Option<(PathBuf, Rc<Vec<ImageAsset>>)>>,
}

/// The loaded journals and their entries, plus the two derived lookup indexes
/// that must stay in sync with `entries`. Grouped so the sync invariant lives
/// behind [`Library::rebuild_indexes`] rather than being spread across `App` —
/// the whole in-memory reading collection.
#[derive(Default)]
pub(crate) struct Library {
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
}

impl Library {
    /// Rebuild the derived entry indexes after `entries` is (re)loaded: the
    /// journal → contiguous-range map (entries are sorted by path, so each
    /// journal's entries form one run) and the entry-id → index map used to
    /// resolve search hits without scanning.
    fn rebuild_indexes(&mut self) {
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

    /// Contiguous index range into `entries` for `journal`, or `None` when it has
    /// no entries.
    fn range(&self, journal: &str) -> Option<Range<usize>> {
        self.journal_ranges.get(journal).cloned()
    }
}

/// Where the reader is in the loaded [`Library`]: the two list selections, the
/// preview scroll, which pane has keyboard focus, and Browse-vs-Search mode.
/// Transient UI position (not content) — it survives a store reload, unlike the
/// data in `Library`.
pub(crate) struct Nav {
    pub(crate) journal_list: ListState,
    /// The selected entry (or search hit) index, or `None` when no entry is
    /// selected. In Browse mode `None` shows the journal insights in the preview
    /// pane instead of an entry — reached by scrolling up past the first entry
    /// or clicking empty space in the list.
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) entry_list: ListState,
    pub(crate) scroll: ScrollState,
    pub(crate) focus: Focus,
    /// Whether the focused entry viewer is expanded to the full screen, hiding the
    /// other columns. Only ever set in multi-column layouts (single-column already
    /// renders the viewer full-screen); reset when focus leaves the viewer.
    pub(crate) entry_view_fullscreen: bool,
    /// Whether the focused insights panel is expanded to the full screen. Like
    /// [`Self::entry_view_fullscreen`] it only matters in multi-column layouts
    /// (single-column already renders the panel full-screen) and is reset when
    /// focus leaves the panel.
    pub(crate) insights_fullscreen: bool,
    pub(crate) mode: Mode,
    /// Which tab the journal-insights panel shows, and whether its analytic tabs
    /// aggregate the selected journal or every journal. Only interactive while
    /// browsing with the Journals column focused.
    pub(crate) insights_tab: InsightsTab,
    pub(crate) insights_scope: InsightsScope,
    /// The rolling window the mood-driver tabs (Drivers, Feelings) aggregate over.
    /// Orthogonal to `insights_scope`: scope picks *which* entries, timeframe picks
    /// *which slice of time* within them.
    pub(crate) insights_timeframe: InsightsTimeframe,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            journal_list: ListState::default(),
            selected_entry_index: None,
            entry_list: ListState::default(),
            scroll: ScrollState::default(),
            focus: Focus::Journals,
            entry_view_fullscreen: false,
            insights_fullscreen: false,
            mode: Mode::Browse,
            insights_tab: InsightsTab::default(),
            insights_scope: InsightsScope::default(),
            insights_timeframe: InsightsTimeframe::default(),
        }
    }
}

pub(crate) struct App {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Config,
    /// Per-device UI state persisted to `state.toml` (e.g. the last-open journal).
    pub(crate) state: State,
    pub(crate) store: JournalStore,
    pub(crate) library: Library,
    pub(crate) nav: Nav,
    pub(crate) search: SearchState,
    pub(crate) overlay: Overlay,
    pub(crate) status_bar: StatusBar,
    pub(crate) image: ImageState,
    /// Background geocoding for the location dialog; spawned on first lookup.
    pub(crate) geocode: crate::tui::geocode::GeocodeWorker,
    /// Background environment lookups (weather + air quality), fired when an
    /// entry's location is set/changed; spawned on first request.
    pub(crate) environment: crate::tui::environment::EnvironmentWorker,
    /// Clickable `[Image N …]` label positions from the last entry-view render.
    pub(crate) entry_view_image_hits: EntryViewImageHits,
    /// The insights list scrollbar geometry from the last render, so a mouse drag
    /// can map cursor rows back to a scroll offset. `total == 0` means the current
    /// tab has no scrollable list (no bar drawn).
    pub(crate) insights_scroll: InsightsScrollGeometry,
    pub(crate) scrollbar: ScrollbarDragState,
    /// Per-frame render memo caches (rows, rendered body, journal insights) and the
    /// version counters that invalidate them. See [`RenderCaches`].
    caches: RenderCaches,
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

/// The insights list scrollbar geometry captured at render time, so the mouse
/// handler can map a drag on the panel's bar back to a row offset.
#[derive(Default)]
pub(crate) struct InsightsScrollGeometry {
    /// The outer panel rect the bar is drawn on.
    pub(crate) area: Rect,
    /// Total rows in the current list.
    pub(crate) total: usize,
    /// Rows visible at once.
    pub(crate) viewport: u16,
}

/// Which pane's vertical scrollbar a mouse drag is currently manipulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollbarDrag {
    Journals,
    EntryList,
    EntryView,
    Insights,
}

/// The in-progress scrollbar drag, if any. A drag keeps scrolling even after the
/// cursor drifts off the one-column bar, so the target pane and the grab offset
/// outlive the initial press.
#[derive(Default)]
pub(crate) struct ScrollbarDragState {
    /// Which pane's scrollbar is being dragged; set on press, cleared on release.
    pub(crate) active: Option<ScrollbarDrag>,
    /// Rows between the top of the thumb and the point where it was grabbed, so the
    /// grabbed point tracks the cursor during the drag.
    pub(crate) grab: u16,
}

impl App {
    pub(crate) fn new(
        config_path: PathBuf,
        config: Config,
        store: JournalStore,
    ) -> AppResult<Self> {
        store.ensure()?;
        let state = crate::config::load_state(&config_path)?;
        let entry_paths = store.collect_entry_paths()?;
        let mut app = Self {
            config_path,
            config,
            state,
            store,
            library: Library::default(),
            nav: Nav::default(),
            search: SearchState::default(),
            overlay: Overlay::None,
            status_bar: StatusBar::default(),
            image: ImageState::default(),
            geocode: crate::tui::geocode::GeocodeWorker::default(),
            environment: crate::tui::environment::EnvironmentWorker::default(),
            entry_view_image_hits: EntryViewImageHits::default(),
            insights_scroll: InsightsScrollGeometry::default(),
            scrollbar: ScrollbarDragState::default(),
            caches: RenderCaches::default(),
        };
        app.load_entries(entry_paths)?;
        // Restore the journal selected in the previous session without disturbing
        // the default startup focus (Journals).
        if let Some(name) = app.state.last_journal.clone()
            && let Some(index) = app
                .library
                .journals
                .iter()
                .position(|journal| journal.name == name)
        {
            app.nav.journal_list.select(Some(index));
            *app.nav.journal_list.offset_mut() = app.journal_row_top(index);
        }
        // Don't start focused on the journal list if it's been hidden.
        if !app.state.ui.show_journals {
            app.nav.focus = Focus::Entries;
        }
        Ok(app)
    }

    /// A journal rename (archive/unarchive) changes its identity, so any config
    /// or per-device state pointing at the old name must follow it — otherwise the
    /// remembered default/last journal silently stops resolving.
    pub(crate) fn retarget_journal_in_config(
        &mut self,
        old_name: &str,
        new_name: &str,
    ) -> AppResult<()> {
        if self.config.journal.default.as_deref() == Some(old_name) {
            self.config.journal.default = Some(new_name.to_string());
            crate::config::save_config(&self.config_path, &self.config)?;
        }
        if self.state.last_journal.as_deref() == Some(old_name) {
            self.state.last_journal = Some(new_name.to_string());
            crate::config::save_state(&self.config_path, &self.state)?;
        }
        Ok(())
    }

    pub(crate) fn refresh(&mut self) -> AppResult<()> {
        self.store.ensure()?;
        self.image.runtime.clear();
        // Content may have changed: force `sync_image_warm` to rebuild next tick
        // and drop the memo so images are re-parsed from the reloaded body.
        self.image.warm = None;
        self.image.selected_cache.borrow_mut().take();
        let entry_paths = self.store.collect_entry_paths()?;
        self.load_entries(entry_paths)
    }

    fn load_entries(&mut self, entry_paths: Vec<EntryPath>) -> AppResult<()> {
        self.library.journals = self.store.list_journals()?;
        self.library.entries = self.store.read_entries(entry_paths)?;
        self.normalize_journal_selection();
        self.after_entries_changed();
        Ok(())
    }

    /// Rebuild derived state after `entries` is replaced or edited: the entry
    /// indexes, the cache-invalidating data version, the search hits (when a
    /// query is active), and the clamped selection. Shared by the full load and
    /// the incremental [`Self::refresh_paths`] path.
    fn after_entries_changed(&mut self) {
        self.library.rebuild_indexes();
        // Entries (and possibly hits) changed: invalidate every version-keyed
        // cache — the body/analytics caches (entries_version) and the row cache
        // (rows_version).
        self.caches.bump_entries();
        if !self.search.query.is_empty() {
            self.search.hits = self.search_results();
        }
        let previous_entry_index = self.nav.selected_entry_index;
        let len = self.current_entry_list_len();
        self.nav.selected_entry_index = self
            .nav
            .selected_entry_index
            .and_then(|index| (len > 0).then(|| index.min(len - 1)));
        if self.nav.selected_entry_index != previous_entry_index {
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
            if !is_entry_file(path) || !self.library.journals.iter().any(|j| j.name == journal) {
                return self.refresh();
            }
            targets.push((journal, path.clone()));
        }
        if targets.is_empty() {
            return Ok(());
        }

        // The viewed entry's body/images may have changed: drop the image caches
        // (the version-keyed row/body/analytics caches self-invalidate on the bump).
        self.image.runtime.clear();
        self.image.warm = None;
        self.image.selected_cache.borrow_mut().take();

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
            .library
            .entries
            .binary_search_by(|existing| entry.path.cmp(&existing.path))
        {
            Ok(index) => self.library.entries[index] = entry,
            Err(index) => self.library.entries.insert(index, entry),
        }
    }

    /// Remove the entry at `path`, if present, preserving the sorted order.
    fn remove_entry_by_path(&mut self, path: &Path) {
        if let Ok(index) = self
            .library
            .entries
            .binary_search_by(|existing| path.cmp(&existing.path))
        {
            self.library.entries.remove(index);
        }
    }

    /// The memoized entry-list rows for the current state, rebuilt only when the
    /// row-determining inputs (rows version, mode, journal, width) change. Returns
    /// an `Rc` so callers can read it while holding a `&mut App` borrow elsewhere.
    pub(crate) fn entry_rows(&self, text_width: u16) -> Rc<EntryRowCache> {
        let key = EntryRowKey {
            version: self.caches.rows_version,
            mode: self.nav.mode.clone(),
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

    /// The memoized analytics for the current scope, or `None` in `Journal`
    /// scope when no journal is selected. `All` scope always yields a value
    /// (aggregating every loaded entry).
    ///
    /// `today` (for the current-streak calculation) is read from the wall clock
    /// but deliberately kept out of the cache key: only `current_streak` depends
    /// on it, so a streak that goes stale across a midnight boundary with no
    /// reload is acceptable and self-heals on the next entry change. Keying on
    /// the date instead would rebuild the whole aggregate every frame after
    /// midnight for no real benefit.
    pub(crate) fn cached_analytics(&self) -> Option<Rc<Analytics>> {
        let today = chrono::Local::now().date_naive();
        match self.nav.insights_scope {
            InsightsScope::Journal => {
                let name = self.selected_journal()?.name.clone();
                Some(
                    self.caches
                        .analytics(self.caches.entries_version, &name, || {
                            analyze(&self.selected_entries(), today)
                        }),
                )
            }
            InsightsScope::All => {
                // A NUL-prefixed key can't collide with a journal name.
                Some(
                    self.caches
                        .analytics(self.caches.entries_version, "\u{0}all", || {
                            let entries: Vec<&Entry> = self.library.entries.iter().collect();
                            analyze(&entries, today)
                        }),
                )
            }
        }
    }

    /// The memoized lift/drain correlations for the current scope, windowed to
    /// `nav.insights_timeframe`. `None` in `Journal` scope with no journal selected.
    /// Powers the Drivers ranking.
    pub(crate) fn cached_windowed_correlations(&self) -> Option<Rc<Correlations>> {
        let today = chrono::Local::now().date_naive();
        let timeframe = self.nav.insights_timeframe;
        let (scope_key, entries): (String, Vec<&Entry>) = match self.nav.insights_scope {
            InsightsScope::Journal => (
                self.selected_journal()?.name.clone(),
                self.selected_entries(),
            ),
            InsightsScope::All => (
                "\u{0}all".to_string(),
                self.library.entries.iter().collect(),
            ),
        };
        Some(
            self.caches
                .windowed(self.caches.entries_version, &scope_key, timeframe, || {
                    let windowed: Vec<&Entry> = match timeframe.window(today) {
                        None => entries.clone(),
                        Some((start, end)) => entries
                            .iter()
                            .copied()
                            .filter(|entry| {
                                entry_group_date(entry)
                                    .is_some_and(|date| start <= date && date <= end)
                            })
                            .collect(),
                    };
                    build_correlations(&windowed)
                }),
        )
    }

    pub(crate) fn scroll_entry_view(&mut self, delta: i16) {
        if delta.is_negative() {
            self.nav.scroll.entry_view = self
                .nav
                .scroll
                .entry_view
                .saturating_sub(delta.unsigned_abs());
        } else {
            self.nav.scroll.entry_view = self.nav.scroll.entry_view.saturating_add(delta as u16);
        }
    }

    pub(crate) fn page_entry_view(&mut self, delta: i16) {
        self.scroll_entry_view(delta.saturating_mul(PAGE_STEP));
    }

    /// Scroll the insights list by `delta` rows. The offset saturates here and is
    /// clamped to the list's length when the panel renders, mirroring the entry
    /// view — so `i16::MAX` from an End key just lands on the last page.
    pub(crate) fn scroll_insights(&mut self, delta: i16) {
        if delta.is_negative() {
            self.nav.scroll.insights = self
                .nav
                .scroll
                .insights
                .saturating_sub(delta.unsigned_abs());
        } else {
            self.nav.scroll.insights = self.nav.scroll.insights.saturating_add(delta as u16);
        }
    }

    pub(crate) fn page_insights(&mut self, delta: i16) {
        self.scroll_insights(delta.saturating_mul(PAGE_STEP));
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

/// Helper for [`App::metadata_partitioned`]: counts per lowercased tag and per
/// original-casing form so we can consolidate case variants.
#[derive(Default)]
struct CasingCount {
    total: usize,
    forms: std::collections::BTreeMap<String, usize>,
}

/// Consolidate a lowercased-key → [`CasingCount`] map into `(display, count)`
/// pairs sorted by count descending then alphabetically. The displayed casing is
/// the most frequent form (ties → first alphabetically).
fn sort_casing(map: std::collections::BTreeMap<String, CasingCount>) -> Vec<(String, usize)> {
    let mut pairs: Vec<_> = map
        .into_values()
        .map(|cc| {
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
        MetadataKind::Tags => &entry.metadata.tags,
        MetadataKind::People => &entry.metadata.people,
        MetadataKind::Activities => &entry.metadata.activities,
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

mod environment;
mod images;
mod location;
mod metadata;
mod overlays;
mod search;
mod selection;

#[cfg(test)]
mod tests;
