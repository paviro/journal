use crate::{
    AppResult,
    config::{Config, State},
};
use notema_domain::Entry;
use notema_storage::{
    CachePolicy, CachedLibrary, Journal, JournalStore, LibrarySnapshot, is_entry_file,
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
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

use super::state::{HoverTarget, Overlay, ScrollState, SearchState, ToastVariant, Toasts};
use crate::tui::editor_state::EntryEditor;
use crate::tui::features::insights::{InsightsScope, InsightsTab, InsightsTimeframe};
use crate::tui::image::{ImageAsset, ImageRuntime};

mod cache;
pub(crate) use cache::RenderCaches;

pub(crate) const JOURNAL_LIST_WIDTH: u16 = 27;
pub(crate) const ENTRY_LIST_INLINE_WIDTH: u16 = 47;
pub(crate) const ENTRY_LIST_MIN_WIDTH: u16 = 40;
pub(crate) const TWO_PANEL_MIN_WIDTH: u16 = 87;
pub(crate) const INLINE_READER_MIN_WIDTH: u16 = 125;

const INITIAL_LIBRARY_LOADING_TOAST: &str = "Loading journals from disk…";
const MANUAL_REFRESH_TOAST: &str = "Refreshing from disk…";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Journals,
    Entries,
    Reader,
    /// The journal insights panel — the right-hand column when no entry is
    /// selected. Reached with Right past Entries; its Left/Right cycle tabs.
    Insights,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Browse,
    Search,
}

pub(crate) use notema_domain::SearchScope;

/// The theme, color mode, and chrome to display, resolved from the context
/// journal's override and the `[ui]` config by [`AppModel::effective_selection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThemeSelection {
    pub(crate) name: String,
    pub(crate) color_mode: crate::config::ColorMode,
    pub(crate) chrome: crate::config::ChromeMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryTarget {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) title: String,
    /// Encrypted entry whose identity is not loaded — cannot be read or written.
    pub(crate) locked: bool,
}

/// The Reader output of the Markdown parse/render pipeline, memoized because it
/// is the dominant per-frame cost of the Reader pane.
#[derive(Default)]
pub(crate) struct RenderedEntryBody {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) images: Vec<(usize, usize)>,
    pub(crate) links: Vec<ReaderLinkHit>,
    pub(crate) headings: Vec<ReaderHeading>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReaderLinkHit {
    pub(crate) line: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) target: String,
    /// Document-unique id shared by every segment of one link. A link name that
    /// wraps across display lines yields several hits with the same `group`, so
    /// hovering any segment can highlight the whole name as one link.
    pub(crate) group: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReaderHeading {
    pub(crate) anchor: String,
    pub(crate) line: usize,
}

pub(crate) struct ReaderAnchorFlash {
    pub(crate) line: usize,
    pub(crate) until: Instant,
}

/// The entry-view image subsystem: the terminal-image runtime plus the caches
/// keyed by entry path (rather than by a [`RenderCaches`] version counter),
/// invalidated together when the open entry changes or the store reloads.
#[derive(Default)]
pub(crate) struct ImageState {
    pub(crate) runtime: ImageRuntime,
    /// `(entry_path, viewer_size)` the runtime is warmed for, or `None` when no
    /// entry view is open. Compared against the desired context each tick.
    pub(crate) warm: Option<(PathBuf, Size)>,
    /// Selected entry's in-folder images, memoized by path; `RefCell` so `&self`
    /// render/hint/shortcut paths can read it. Re-parsed on a path change or when
    /// `refresh` clears it.
    pub(crate) selected_cache: RefCell<Option<(PathBuf, Rc<Vec<ImageAsset>>)>>,
}

/// The loaded journals and their entries, plus the two derived lookup indexes
/// that must stay in sync with `entries`. Grouped so the sync invariant lives
/// behind [`Library::rebuild_indexes`] rather than being spread across `AppModel` —
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
    /// In Search mode the reader getters resolve a hit's `&Entry` through this
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
    pub(crate) fn entry_by_id(&self, id: &str) -> Option<&Entry> {
        self.entries.get(*self.entry_index_by_id.get(id)?)
    }

    /// Contiguous index range into `entries` for `journal`, or `None` when it has
    /// no entries.
    pub(crate) fn range(&self, journal: &str) -> Option<Range<usize>> {
        self.journal_ranges.get(journal).cloned()
    }
}

/// Where the reader is in the loaded [`Library`]: the two list selections, the
/// reader scroll, which pane has keyboard focus, and Browse-vs-Search mode.
/// Transient UI position (not content) — it survives a store reload, unlike the
/// data in `Library`.
pub(crate) struct Nav {
    pub(crate) journal_list: ListState,
    /// The selected entry (or search hit) index, or `None` when no entry is
    /// selected. In Browse mode `None` shows the journal insights in the reader
    /// pane instead of an entry — reached by scrolling up past the first entry
    /// or clicking empty space in the list.
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) entry_list: ListState,
    pub(crate) scroll: ScrollState,
    pub(crate) focus: Focus,
    /// Whether the focused entry viewer is expanded to the full screen, hiding the
    /// other columns. Only ever set in multi-column layouts (single-column already
    /// renders the viewer full-screen); reset when focus leaves the viewer.
    pub(crate) reader_fullscreen: bool,
    /// Whether the focused insights panel is expanded to the full screen. Like
    /// [`Self::reader_fullscreen`] it only matters in multi-column layouts
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
    /// A mouse drag is selecting text in a single-line field (search box or a
    /// dialog input); set on press in the field, cleared on release.
    pub(crate) input_selecting: bool,
    /// The last left-button press `(time, col, row)`, used to detect a
    /// double-click (a second press on the same cell within [`DOUBLE_CLICK`]).
    pub(crate) last_click: Option<(Instant, u16, u16)>,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            journal_list: ListState::default(),
            selected_entry_index: None,
            entry_list: ListState::default(),
            scroll: ScrollState::default(),
            focus: Focus::Journals,
            reader_fullscreen: false,
            insights_fullscreen: false,
            mode: Mode::Browse,
            insights_tab: InsightsTab::default(),
            insights_scope: InsightsScope::default(),
            insights_timeframe: InsightsTimeframe::default(),
            input_selecting: false,
            last_click: None,
        }
    }
}

/// How close in time two left presses on the same cell must land to count as a
/// double-click. Terminals send discrete press/release pairs, so a double-click
/// is just two presses on one cell within this window.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

impl Nav {
    /// Record a left-button press and report whether it completes a double-click:
    /// a prior press within [`DOUBLE_CLICK`] on the same `(col, row)`. A match
    /// clears the record so a third quick press starts a fresh single click
    /// rather than chaining into another double.
    pub(crate) fn register_left_click(&mut self, col: u16, row: u16) -> bool {
        let now = Instant::now();
        let is_double = self.last_click.is_some_and(|(at, c, r)| {
            c == col && r == row && now.duration_since(at) <= DOUBLE_CLICK
        });
        self.last_click = if is_double {
            None
        } else {
            Some((now, col, row))
        };
        is_double
    }
}

pub(crate) struct Services {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Config,
    pub(crate) store: JournalStore,
}

pub(crate) struct Appearance {
    pub(crate) theme: crate::tui::theme::Theme,
    pub(crate) color_mode: crate::config::ColorMode,
    pub(crate) chrome_override: Option<crate::tui::theme::ChromeStyle>,
    pub(crate) detected_mode: crate::tui::theme::Mode,
    warned_themes: BTreeSet<String>,
}

impl Appearance {
    pub(crate) fn mode(&self) -> crate::tui::theme::Mode {
        match self.color_mode {
            crate::config::ColorMode::Dark => crate::tui::theme::Mode::Dark,
            crate::config::ColorMode::Light => crate::tui::theme::Mode::Light,
            crate::config::ColorMode::Auto => self.detected_mode,
        }
    }

    pub(crate) fn resolve(&self, theme: crate::tui::theme::Theme) -> crate::tui::theme::Theme {
        theme.with_chrome_override(self.chrome_override)
    }

    fn warning(&mut self, name: &str, warning: Option<String>) -> Option<String> {
        match warning {
            Some(message) if self.warned_themes.insert(name.to_string()) => Some(message),
            Some(_) => None,
            None => {
                self.warned_themes.remove(name);
                None
            }
        }
    }
}

pub(crate) struct AppModel {
    pub(crate) services: Services,
    pub(crate) appearance: Appearance,
    /// Per-device UI state persisted to `state.toml` (e.g. the last-open journal).
    pub(crate) state: State,
    pub(crate) library: Library,
    /// Changes whenever source-backed library state is refreshed. Startup
    /// validation uses this to avoid installing a snapshot older than an edit
    /// or manual refresh completed while it was running.
    pub(crate) library_generation: u64,
    pub(crate) nav: Nav,
    pub(crate) search: SearchState,
    pub(crate) overlay: Overlay,
    /// The in-pane internal editor session, when one is open. Distinct from
    /// [`Overlay`] because it replaces the entry-view content rather than
    /// floating a modal over it.
    pub(crate) editor: Option<EntryEditor>,
    /// One-shot compose mode: the app launched straight into a fullscreen new-entry
    /// editor (`notema log` with no body) and quits once that entry is saved or
    /// discarded, rather than dropping back to the entry list.
    pub(crate) compose: bool,
    pub(crate) toasts: Toasts,
    pub(crate) image: ImageState,
    /// Background geocoding for the location dialog; spawned on first lookup.
    pub(crate) geocode: crate::tui::geocode::GeocodeWorker,
    /// Background weather/air-quality/celestial fetching; spawned on first use.
    /// Serves the editor prefetch and direct location-sets. Bulk enrichment of
    /// existing entries is the `notema backfill` CLI command's job, not the TUI's.
    pub(crate) environment: crate::tui::environment::EnvironmentWorker,
    /// Id counter for environment requests (editor fetches, direct
    /// location-set write-backs) — app-level so ids never repeat across editor
    /// sessions and a stale result can't be adopted by a later one.
    pub(crate) next_environment_id: u64,
    /// Id counter for geocode requests, app-level for the same reason: a dialog
    /// reopened while an earlier lookup is still in flight must not reuse its id.
    pub(crate) next_geocode_id: u64,
    pub(crate) reader_anchor_flash: Option<ReaderAnchorFlash>,
    pub(crate) scrollbar: ScrollbarDragState,
    /// The row/hint under the mouse cursor, for hover highlights. Set by mouse
    /// motion, cleared by any key event (see [`HoverTarget`]).
    pub(crate) hover: HoverTarget,
    /// Per-frame render memo caches (rows, rendered body, journal insights) and the
    /// version counters that invalidate them. See [`RenderCaches`].
    pub(crate) caches: RenderCaches,
}

/// Clickable image label positions in the entry view, captured at render time so
/// the mouse handler can map a click back to an image index.
#[derive(Default)]
pub(crate) struct ReaderImageHits {
    pub(crate) content_rect: Rect,
    pub(crate) scroll: u16,
    /// Total rendered body line count, for mapping a scrollbar drag to a scroll offset.
    pub(crate) line_count: usize,
    /// `(body line index, image index)` per label line.
    pub(crate) labels: Vec<(usize, usize)>,
    pub(crate) links: Vec<ReaderLinkHit>,
    pub(crate) headings: Vec<ReaderHeading>,
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
    /// Effective clamped offset used for this frame.
    pub(crate) scroll: u16,
}

/// Which pane's vertical scrollbar a mouse drag is currently manipulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollbarDrag {
    Journals,
    EntryList,
    Reader,
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

impl AppModel {
    #[cfg(any(test, feature = "bench"))]
    pub(crate) fn new(
        config_path: PathBuf,
        config: Config,
        store: JournalStore,
    ) -> AppResult<Self> {
        let snapshot = store.load_library(CachePolicy::Normal)?;
        Self::new_with_snapshot(
            config_path,
            config,
            store,
            snapshot,
            crate::tui::theme::Mode::Dark,
        )
    }

    /// Build from the decoded cache. The retained opaque cache is passed to the
    /// event loop's background validator; a miss reads only the cheap top-level
    /// journal list while entry parsing remains in the background.
    pub(crate) fn new_cached(
        config_path: PathBuf,
        config: Config,
        store: JournalStore,
        detected_mode: crate::tui::theme::Mode,
    ) -> AppResult<(Self, Option<CachedLibrary>)> {
        let cache = store.read_cached_library(CachePolicy::Normal)?;
        // A passphrase-locked store can't decrypt its entries, and no background
        // validation runs while locked — so don't walk the journal root (slow start)
        // and don't raise the "Loading journals…" toast we'd never dismiss.
        let locked = store.encryption_enabled() && !store.is_unlocked();
        let snapshot = match cache.cached.as_ref() {
            Some(cached) => cached.snapshot(),
            None => LibrarySnapshot {
                journals: if locked {
                    Vec::new()
                } else {
                    store.list_journals()?
                },
                entries: Vec::new(),
                report: cache.report.clone(),
            },
        };
        let mut app = Self::new_with_snapshot(config_path, config, store, snapshot, detected_mode)?;
        if cache.cached.is_none() && !locked {
            app.toasts
                .push_persistent(ToastVariant::Info, INITIAL_LIBRARY_LOADING_TOAST);
        }
        Ok((app, cache.cached))
    }

    fn new_with_snapshot(
        config_path: PathBuf,
        config: Config,
        store: JournalStore,
        snapshot: LibrarySnapshot,
        detected_mode: crate::tui::theme::Mode,
    ) -> AppResult<Self> {
        let state = crate::config::load_state(&config_path)?;
        let appearance = Appearance {
            theme: crate::tui::theme::Theme::terminal_default(),
            color_mode: config.ui.color_mode,
            chrome_override: crate::tui::theme::chrome_style(config.ui.chrome),
            detected_mode,
            warned_themes: BTreeSet::new(),
        };
        let mut app = Self {
            services: Services {
                config_path,
                config,
                store,
            },
            appearance,
            state,
            library: Library::default(),
            library_generation: 0,
            nav: Nav::default(),
            search: SearchState::default(),
            overlay: Overlay::None,
            editor: None,
            compose: false,
            toasts: Toasts::default(),
            image: ImageState::default(),
            geocode: crate::tui::geocode::GeocodeWorker::default(),
            environment: crate::tui::environment::EnvironmentWorker::default(),
            next_environment_id: 0,
            next_geocode_id: 0,
            reader_anchor_flash: None,
            scrollbar: ScrollbarDragState::default(),
            hover: HoverTarget::default(),
            caches: RenderCaches::default(),
        };
        app.library.journals = snapshot.journals;
        app.library.entries = snapshot.entries;
        app.normalize_journal_selection();
        app.after_entries_changed();
        // Restore the journal selected in the previous session (by stable id, so a
        // rename or archive doesn't lose it) without disturbing the default startup
        // focus (Journals).
        if let Some(id) = app.state.last_journal_id.clone()
            && let Some(index) = app
                .library
                .journals
                .iter()
                .position(|journal| !journal.id.is_empty() && journal.id == id)
        {
            app.nav.journal_list.select(Some(index));
            *app.nav.journal_list.offset_mut() = app.journal_row_top(index);
        }
        // Don't start focused on the journal list if it's been hidden.
        if !app.state.ui.show_journals {
            app.nav.focus = Focus::Entries;
        }
        // The startup journal is chosen only now, so the pre-AppModel theme install
        // couldn't account for it.
        app.apply_effective_theme();
        Ok(app)
    }

    /// Replace cache-backed library state with a reconciled source snapshot,
    /// preserving the user's current journal and entry where possible.
    pub(crate) fn install_library_snapshot(&mut self, snapshot: LibrarySnapshot) {
        self.finish_initial_library_loading();
        let journal_id = self.selected_journal().map(|journal| journal.id.clone());
        let entry_id = self.selected_entry_target().map(|entry| entry.id);
        self.clear_image_caches();
        self.library.journals = snapshot.journals;
        self.library.entries = snapshot.entries;
        self.normalize_journal_selection();
        if let Some(journal_id) = journal_id
            && let Some(index) = self
                .library
                .journals
                .iter()
                .position(|journal| journal.id == journal_id)
        {
            self.nav.journal_list.select(Some(index));
        }
        self.after_entries_changed();
        if let Some(entry_id) = entry_id {
            self.select_entry_by_id(&entry_id, false);
        }
        self.apply_effective_theme();
    }

    pub(crate) fn library_generation(&self) -> u64 {
        self.library_generation
    }

    pub(crate) fn finish_initial_library_loading(&mut self) {
        self.toasts.dismiss_message(INITIAL_LIBRARY_LOADING_TOAST);
    }

    pub(crate) fn begin_manual_refresh(&mut self) {
        self.toasts
            .push_persistent(ToastVariant::Info, MANUAL_REFRESH_TOAST);
    }

    pub(crate) fn finish_manual_refresh(&mut self) {
        self.toasts.dismiss_message(MANUAL_REFRESH_TOAST);
    }

    /// A journal rename (archive/unarchive) changes its folder name, so the
    /// hand-editable `config.journal.default` (which points at a name) must follow
    /// it — otherwise the remembered default silently stops resolving. Per-device
    /// `last_journal_id` needs no retargeting: it's keyed on the stable id, which
    /// the rename preserves.
    pub(crate) fn retarget_journal_in_config(
        &mut self,
        old_name: &str,
        new_name: &str,
    ) -> AppResult<()> {
        if self.services.config.journal.default.as_deref() == Some(old_name) {
            self.services.config.journal.default = Some(new_name.to_string());
            crate::config::save_config(&self.services.config_path, &self.services.config)?;
        }
        Ok(())
    }

    /// The journal whose theme applies right now: the compose target while
    /// composing, the scope journal of a journal-scoped search, otherwise the
    /// selected journal. An all-journals search has no context journal — it
    /// follows the global theme, so stepping through cross-journal hits doesn't
    /// re-theme per hit.
    pub(crate) fn context_journal(&self) -> Option<&Journal> {
        if self.compose
            && let Some(editor) = &self.editor
            && let crate::tui::editor_state::EditorTarget::New { journal } = &editor.target
        {
            return self.library.journals.iter().find(|j| &j.name == journal);
        }
        if self.nav.mode == Mode::Search {
            return match &self.search.scope {
                SearchScope::Journal(name) => {
                    self.library.journals.iter().find(|j| &j.name == name)
                }
                SearchScope::AllJournals => None,
            };
        }
        self.selected_journal()
    }

    /// The theme selection in effect: the context journal's own theme — with
    /// per-field fallback to the `[ui]` config for anything it doesn't set or
    /// this device doesn't recognize — unless the device ignores per-journal
    /// themes.
    pub(crate) fn effective_selection(&self) -> ThemeSelection {
        let global = ThemeSelection {
            name: self.services.config.ui.theme.clone(),
            color_mode: self.services.config.ui.color_mode,
            chrome: self.services.config.ui.chrome,
        };
        if self.services.config.ui.ignore_journal_themes {
            return global;
        }
        let Some(theme) = self.context_journal().and_then(|j| j.theme.as_ref()) else {
            return global;
        };
        ThemeSelection {
            name: theme.name.clone(),
            color_mode: theme
                .color_mode
                .as_deref()
                .and_then(crate::config::ColorMode::from_name)
                .unwrap_or(global.color_mode),
            chrome: theme
                .chrome
                .as_deref()
                .and_then(crate::config::ChromeMode::from_name)
                .unwrap_or(global.chrome),
        }
    }

    pub(crate) fn effective_theme_name(&self) -> String {
        self.effective_selection().name
    }

    /// Resolve the effective theme, color mode, and chrome. Called at
    /// startup and whenever the theme context changes (journal switch, search
    /// enter/exit, compose), so the UI re-themes as you move around.
    pub(crate) fn apply_effective_theme(&mut self) {
        // Tests assign owned themes directly when they need a specific palette.
        #[cfg(not(test))]
        {
            let selection = self.effective_selection();
            self.appearance.color_mode = selection.color_mode;
            self.appearance.chrome_override = crate::tui::theme::chrome_style(selection.chrome);
            let (theme, warn) = crate::tui::theme::load(
                &self.services.config_path,
                &selection.name,
                self.appearance.mode(),
            );
            self.appearance.theme = self.appearance.resolve(theme);
            let warning =
                warn.map(|err| crate::tui::theme::format_theme_warning(&selection.name, &err));
            if let Some(message) = self.appearance.warning(&selection.name, warning) {
                self.toast(ToastVariant::Warning, message);
            }
        }
        #[cfg(test)]
        {
            let selection = self.effective_selection();
            let _ = self.appearance.warning(&selection.name, None);
        }
    }

    pub(crate) fn refresh(&mut self) -> AppResult<()> {
        let snapshot = self.services.store.load_library(CachePolicy::Normal)?;
        self.install_library_snapshot(snapshot);
        Ok(())
    }

    /// Rebuild derived state after `entries` is replaced or edited: the entry
    /// indexes, the cache-invalidating data version, the search hits (when a
    /// query is active), and the clamped selection. Shared by the full load and
    /// the incremental [`Self::refresh_paths`] path.
    fn after_entries_changed(&mut self) {
        self.library_generation = self.library_generation.wrapping_add(1);
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
        self.services.store.ensure()?;
        let root = self.services.store.root().to_path_buf();

        // notify frequently reports the same path several times per change.
        let mut changed = paths.to_vec();
        changed.sort();
        changed.dedup();

        let mut targets: Vec<(String, PathBuf)> = Vec::new();
        let mut asset_changed = false;
        for path in &changed {
            if is_asset_path(path) {
                asset_changed = true;
                continue;
            }
            let Some(journal) = journal_for_path(&root, path) else {
                return self.refresh();
            };
            if !is_entry_file(path) || !self.library.journals.iter().any(|j| j.name == journal) {
                return self.refresh();
            }
            targets.push((journal, path.clone()));
        }
        if targets.is_empty() {
            if asset_changed {
                self.clear_image_caches();
            }
            return Ok(());
        }

        // The viewed entry's body/images may have changed: drop the image caches
        // (the version-keyed row/body/analytics caches self-invalidate on the bump).
        self.clear_image_caches();

        for (journal, path) in targets {
            if path.exists() {
                let entry = self.services.store.read_entry(&journal, &path)?;
                self.upsert_entry(entry);
            } else {
                self.remove_entry_by_path(&path);
            }
        }
        self.after_entries_changed();
        Ok(())
    }

    fn clear_image_caches(&mut self) {
        self.image.runtime.clear();
        self.image.warm = None;
        self.image.selected_cache.borrow_mut().take();
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

    /// Install an entry that was read directly from its source file, then
    /// rebuild every list/search derivative that may have come from the cache.
    pub(crate) fn replace_entry_from_disk(&mut self, entry: Entry) {
        let id = entry.id.clone();
        self.clear_image_caches();
        self.upsert_entry(entry);
        self.after_entries_changed();
        self.select_entry_by_id(&id, false);
    }

    /// Refresh the selected entry from its source file. Lists and search may be
    /// cache-backed; viewer and mutation paths call this before using content.
    pub(crate) fn reload_selected_entry_from_disk(&mut self) -> AppResult<bool> {
        let Some(entry) = self.resolved_selected_entry() else {
            return Ok(false);
        };
        let journal = entry.journal.clone();
        let path = entry.path.clone();
        let fresh = self.services.store.read_entry(&journal, &path)?;
        self.replace_entry_from_disk(fresh);
        Ok(true)
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

    /// Precomputed word count of the entry currently shown in the reader, or 0
    /// when none is selected.
    pub(crate) fn selected_entry_word_count(&self) -> usize {
        self.resolved_selected_entry()
            .map_or(0, |entry| entry.word_count)
    }

    pub(crate) fn toast(&mut self, variant: ToastVariant, message: impl Into<String>) {
        self.toasts.push(variant, message);
    }

    /// Drop expired toasts, reporting whether any were removed (a repaint is due).
    pub(crate) fn expire_toasts(&mut self) -> bool {
        self.toasts.expire()
    }

    /// Time until the nearest toast deadline, for the event loop's poll timeout.
    pub(crate) fn toast_deadline(&self) -> Option<Duration> {
        self.toasts.deadline()
    }

    /// The footer/editor hint under the cursor, for hover-styling its label.
    pub(crate) fn hovered_footer_hint(&self) -> Option<crate::tui::render::HintId> {
        match self.hover {
            HoverTarget::FooterHint(id) => Some(id),
            _ => None,
        }
    }
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

fn is_asset_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.ends_with(".assets"))
    })
}

pub(crate) fn inline_reader_is_visible(width: u16) -> bool {
    width >= INLINE_READER_MIN_WIDTH
}

pub(crate) fn reader_is_available(width: u16) -> bool {
    width >= TWO_PANEL_MIN_WIDTH
}

pub(crate) fn single_panel_is_active(width: u16) -> bool {
    width < TWO_PANEL_MIN_WIDTH
}

#[cfg(test)]
mod tests;
