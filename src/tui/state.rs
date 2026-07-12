//! Focused state containers held by [`App`](super::app::App), split out so the
//! reset/lifecycle logic for each concern lives in one place.

use std::time::{Duration, Instant};

use notema_domain::SearchHit;
use ratatui::widgets::ListState;

use super::app::{EditFeelingState, EditLocationState, EditMetadataState, SearchScope};
use super::image::ImageAsset;
use super::text_input::TextInput;

/// Shortest a toast stays up — brief confirmations ("Saved", "Theme set to …")
/// sit here; longer messages linger proportionally to their reading time.
const TOAST_MIN_LIFETIME: Duration = Duration::from_secs(5);

/// Cap on a toast's lifetime, so even a long error clears itself.
const TOAST_MAX_LIFETIME: Duration = Duration::from_secs(10);

/// Reading budget granted per character; a message's lifetime is its length
/// times this, clamped to [`TOAST_MIN_LIFETIME`]..=[`TOAST_MAX_LIFETIME`].
const TOAST_MS_PER_CHAR: u64 = 100;

/// How long a toast carrying `message` stays up: proportional to its length so
/// longer text gets more reading time, clamped to the min/max window.
fn toast_lifetime(message: &str) -> Duration {
    let reading = Duration::from_millis(message.chars().count() as u64 * TOAST_MS_PER_CHAR);
    reading.clamp(TOAST_MIN_LIFETIME, TOAST_MAX_LIFETIME)
}

/// Newest toasts kept when the queue overflows.
const TOAST_CAP: usize = 4;

/// Vertical scroll offsets for the panels that scroll their own body: the entry
/// reader, and the insights panel's ranked-list tabs (People / Activities / Tags).
#[derive(Default)]
pub(crate) struct ScrollState {
    pub(crate) reader: u16,
    /// First visible row of the insights list tabs, in row units (not pixels).
    pub(crate) insights: u16,
}

impl ScrollState {
    /// Reset the entry reader scroll.
    pub(crate) fn reset_reader(&mut self) {
        self.reader = 0;
    }

    /// Reset the insights list scroll — called when the tab, scope, or journal
    /// changes so a new list starts at the top.
    pub(crate) fn reset_insights(&mut self) {
        self.insights = 0;
    }
}

/// What the mouse cursor is over, for hover highlights. Any key event clears
/// it back to `None` — that single rule is the whole keyboard/mouse input-mode
/// machine: a parked cursor must not keep glowing while the user arrows
/// around, and the next mouse move restores it. Hovering never moves the main
/// panels' selection (selecting has side effects — journal switch, reader
/// swap — that stay click-only); overlay menus do follow the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HoverTarget {
    #[default]
    None,
    Journal(usize),
    Entry(usize),
    InsightsTab(crate::tui::render::insights::InsightsTab),
    FooterHint(crate::tui::render::HintId),
    /// A row in whichever list/menu dialog is open (settings menu, metadata
    /// menu, edit-metadata/feelings/location lists, theme picker) — only one is
    /// ever open, so the index needs no dialog discriminant.
    DialogRow(usize),
    /// A confirm dialog's yes (`true`) / no (`false`) button.
    ConfirmButton(bool),
    /// A clickable link name in the reader, by its body line and column span.
    ReaderLink {
        line: usize,
        start: usize,
        end: usize,
    },
    /// A clickable `[Image N …]` label in the reader, by its body line.
    ReaderImage(usize),
    /// A single-line text field, identified by the rect it was last drawn
    /// into (fields carry no other identity; only one can be hovered).
    TextField(ratatui::layout::Rect),
    Toast(usize),
}

/// The kind of event a toast reports, driving its accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastVariant {
    Info,
    Success,
    Warning,
    Error,
}

/// A transient notification stacked in the screen's top-right corner, with an
/// auto-expiry deadline.
pub(crate) struct Toast {
    pub(crate) message: String,
    pub(crate) variant: ToastVariant,
    deadline: Instant,
    /// How long this toast was granted, so the countdown line scales to its own
    /// (length-dependent) lifetime rather than a fixed constant.
    lifetime: Duration,
}

impl Toast {
    /// Fraction of this toast's lifetime still remaining, `1.0` at push down to
    /// `0.0` at the deadline. Drives the shrinking dismissal countdown line.
    pub(crate) fn remaining_fraction(&self) -> f32 {
        let left = self
            .deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f32();
        (left / self.lifetime.as_secs_f32()).clamp(0.0, 1.0)
    }
}

/// The toast queue: capped to the newest [`TOAST_CAP`], each toast expiring on
/// its own deadline.
#[derive(Default)]
pub(crate) struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub(crate) fn push(&mut self, variant: ToastVariant, message: impl Into<String>) {
        let message = message.into();
        let lifetime = toast_lifetime(&message);
        self.items.push(Toast {
            message,
            variant,
            deadline: Instant::now() + lifetime,
            lifetime,
        });
        if self.items.len() > TOAST_CAP {
            self.items.drain(..self.items.len() - TOAST_CAP);
        }
    }

    /// The queued toasts, oldest first.
    pub(crate) fn items(&self) -> &[Toast] {
        &self.items
    }

    /// Time until the nearest deadline, for the event loop's poll timeout.
    pub(crate) fn deadline(&self) -> Option<Duration> {
        self.items
            .iter()
            .map(|toast| toast.deadline.saturating_duration_since(Instant::now()))
            .min()
    }

    /// Time until the countdown line loses its next column for whichever toast
    /// is closest to a step, given the shared inner column count `cols`. Waking
    /// exactly at this instant makes the shrink step evenly, rather than beating
    /// against a fixed poll rate (which stalls a frame, then jumps). `None` when
    /// nothing is animating (no toasts, no room, or all lines already empty).
    pub(crate) fn next_countdown_step(&self, cols: u16) -> Option<Duration> {
        if cols == 0 {
            return None;
        }
        let columns = f32::from(cols);
        let now = Instant::now();
        self.items
            .iter()
            .filter_map(|toast| {
                let lifetime = toast.lifetime.as_secs_f32();
                let remaining = toast.deadline.saturating_duration_since(now).as_secs_f32();
                let filled = (columns * remaining / lifetime).ceil();
                if filled <= 0.0 {
                    return None;
                }
                // Remaining life at which `filled` drops by one column.
                let next = (filled - 1.0) * lifetime / columns;
                Some(Duration::from_secs_f32((remaining - next).max(0.0)))
            })
            .min()
    }

    /// Remove the toast at `index` — a click dismissed it early.
    pub(crate) fn dismiss(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// Drop expired toasts, reporting whether any were removed (so the event
    /// loop knows a repaint is due).
    pub(crate) fn expire(&mut self) -> bool {
        let now = Instant::now();
        let before = self.items.len();
        self.items.retain(|toast| toast.deadline > now);
        self.items.len() < before
    }

    /// Push a toast whose deadline is already in the past (test helper).
    #[cfg(test)]
    pub(crate) fn push_expired(&mut self, variant: ToastVariant, message: impl Into<String>) {
        self.items.push(Toast {
            message: message.into(),
            variant,
            deadline: Instant::now() - Duration::from_secs(1),
            lifetime: TOAST_MIN_LIFETIME,
        });
    }
}

/// Search query, scope and the hits it currently matches.
pub(crate) struct SearchState {
    pub(crate) query: TextInput,
    pub(crate) scope: SearchScope,
    pub(crate) hits: Vec<SearchHit>,
    /// Set when the query changed but the (expensive) hit recompute has been
    /// deferred; the event loop runs it once typing pauses (debounce).
    pub(crate) dirty: bool,
    /// Timestamp of the last search keystroke, for the debounce window.
    pub(crate) last_edit: Option<Instant>,
}

impl Default for SearchState {
    fn default() -> Self {
        let mut query = TextInput::default();
        query.set_placeholder_text("type to search");
        Self {
            query,
            scope: SearchScope::AllJournals,
            hits: Vec::new(),
            dirty: false,
            last_edit: None,
        }
    }
}

/// A `ListState` with the app's shared keyboard/scroll navigation, so overlay
/// list states don't each re-wire selection and offset handling. The item count
/// (`len`) is supplied per call because it lives on the owning state (a filtered
/// view for tags, the full vocabulary for feelings).
#[derive(Default)]
pub(crate) struct SelectableList {
    state: ListState,
}

impl SelectableList {
    pub(crate) fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub(crate) fn offset(&self) -> usize {
        self.state.offset()
    }

    pub(crate) fn set_offset(&mut self, offset: usize) {
        *self.state.offset_mut() = offset;
    }

    pub(crate) fn normalize(&mut self, len: usize) {
        normalize_list_state(&mut self.state, len);
    }

    pub(crate) fn select(&mut self, index: usize, len: usize) {
        if index < len {
            self.state.select(Some(index));
        }
    }

    pub(crate) fn move_by(&mut self, len: usize, delta: isize) {
        move_list_selection(&mut self.state, len, delta);
    }

    pub(crate) fn scroll_by(&mut self, delta: i16, len: usize, viewport_height: u16) {
        scroll_list_offset(&mut self.state, delta, len, viewport_height);
    }

    pub(crate) fn ensure_visible(&mut self, len: usize, viewport_height: u16) {
        ensure_selected_visible(&mut self.state, len, viewport_height);
    }
}

/// Keyboard/scroll navigation shared by the overlay list states. An implementor
/// exposes its [`SelectableList`] and current item count; the navigation methods
/// come for free, so `EditMetadataState` and `EditFeelingState` don't each re-forward
/// them with their own length source.
pub(crate) trait ListNav {
    fn list(&self) -> &SelectableList;
    fn list_mut(&mut self) -> &mut SelectableList;
    fn item_count(&self) -> usize;

    fn selected_index(&self) -> Option<usize> {
        self.list().selected()
    }

    fn offset(&self) -> usize {
        self.list().offset()
    }

    fn normalize_list_state(&mut self) {
        let len = self.item_count();
        self.list_mut().normalize(len);
    }

    fn select_index(&mut self, index: usize) {
        let len = self.item_count();
        self.list_mut().select(index, len);
    }

    fn move_up(&mut self) {
        let len = self.item_count();
        self.list_mut().move_by(len, -1);
    }

    fn move_down(&mut self) {
        let len = self.item_count();
        self.list_mut().move_by(len, 1);
    }

    fn scroll_by(&mut self, delta: i16, viewport_height: u16) {
        let len = self.item_count();
        self.list_mut().scroll_by(delta, len, viewport_height);
    }

    fn ensure_selected_visible(&mut self, viewport_height: u16) {
        let len = self.item_count();
        self.list_mut().ensure_visible(len, viewport_height);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataKind {
    Tags,
    People,
    Activities,
}

impl MetadataKind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            MetadataKind::Tags => "Tags",
            MetadataKind::People => "People",
            MetadataKind::Activities => "Activities",
        }
    }

    pub(crate) fn search_prefix(self) -> &'static str {
        match self {
            MetadataKind::Tags => "tags",
            MetadataKind::People => "people",
            MetadataKind::Activities => "activities",
        }
    }
}

pub(crate) fn normalize_list_state(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }

    let selected = state.selected().unwrap_or(0).min(len - 1);
    state.select(Some(selected));
    if state.offset() >= len {
        *state.offset_mut() = len - 1;
    }
}

pub(crate) fn move_list_selection(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        state.select(None);
        return;
    }

    let selected = state.selected().unwrap_or(0);
    let next = (selected as isize + delta).clamp(0, len as isize - 1) as usize;
    state.select(Some(next));
}

pub(crate) fn scroll_list_offset(
    state: &mut ListState,
    delta: i16,
    len: usize,
    viewport_height: u16,
) {
    if len == 0 || viewport_height == 0 {
        *state.offset_mut() = 0;
        return;
    }
    // Item-index space here (`len` items, one row each), but the clamp is the same
    // shape as the pixel lists', so share it.
    *state.offset_mut() =
        crate::tui::scroll::scroll_pixels(state.offset(), delta, len, viewport_height);
}

pub(crate) fn ensure_selected_visible(state: &mut ListState, len: usize, viewport_height: u16) {
    if len == 0 || viewport_height == 0 {
        *state.offset_mut() = 0;
        return;
    }

    let Some(selected) = state.selected().map(|index| index.min(len - 1)) else {
        return;
    };
    let viewport_height = viewport_height as usize;
    let offset = state.offset();
    let max_offset = len.saturating_sub(viewport_height);
    let next_offset = if selected < offset {
        selected
    } else if selected >= offset.saturating_add(viewport_height) {
        selected.saturating_add(1).saturating_sub(viewport_height)
    } else {
        offset
    };

    *state.offset_mut() = next_offset.min(max_offset);
}

/// One row of the theme picker: a theme file's stem and its parse result,
/// cached when the picker opens so selection moves don't re-read the disk.
pub(crate) struct ThemePickerEntry {
    pub(crate) name: String,
    /// The resolved theme, or `None` when the file failed to parse (rendered
    /// as broken and never installed).
    pub(crate) theme: Option<crate::tui::theme::Theme>,
    /// Whether the file resolves identically in dark and light mode (classic,
    /// broken files) — the picker hides its mode switch on such rows.
    pub(crate) mode_agnostic: bool,
}

/// State for the theme-picker overlay. Selection moves preview by installing
/// the highlighted theme; Esc restores [`Self::previous`].
pub(crate) struct ThemePickerState {
    pub(crate) entries: Vec<ThemePickerEntry>,
    pub(crate) list: SelectableList,
    /// The theme installed when the picker opened, restored on cancel.
    pub(crate) previous: crate::tui::theme::Theme,
    /// The configured theme name at open, marking the active row.
    pub(crate) previous_name: String,
    /// The chrome override at open, restored on cancel (the picker cycles it
    /// live for preview).
    pub(crate) previous_chrome: Option<crate::tui::theme::ChromeStyle>,
    /// The color mode at open, restored on cancel (the picker cycles it live
    /// for preview).
    pub(crate) previous_color_mode: crate::config::ColorMode,
}

impl ThemePickerState {
    /// The highlighted entry, if any.
    pub(crate) fn selected_entry(&self) -> Option<&ThemePickerEntry> {
        self.entries.get(self.selected_index()?)
    }

    /// Whether the mode switch applies to the highlighted row: hidden when
    /// the theme resolves the same in both modes, so the picker never offers
    /// a control that does nothing.
    pub(crate) fn mode_switchable(&self) -> bool {
        self.selected_entry()
            .is_some_and(|entry| !entry.mode_agnostic)
    }
}

impl ListNav for ThemePickerState {
    fn list(&self) -> &SelectableList {
        &self.list
    }

    fn list_mut(&mut self) -> &mut SelectableList {
        &mut self.list
    }

    fn item_count(&self) -> usize {
        self.entries.len()
    }
}

/// State for the edit-mood overlay.
pub(crate) struct EditMoodState {
    /// The mood score currently saved on the entry (None = not set).
    pub(crate) saved: Option<i8>,
    /// The score being edited (-5..=5).
    pub(crate) draft: i8,
}

/// Fullscreen image viewer overlay: the entry's images in body order and the
/// one currently shown.
pub(crate) struct ImageViewerState {
    pub(crate) assets: Vec<ImageAsset>,
    pub(crate) index: usize,
}

pub(crate) enum DeleteContext {
    Entry {
        has_body: bool,
    },
    Journal {
        name: String,
        trash_count: usize,
        delete_count: usize,
    },
}

/// The single modal overlay that can be active over the browse view. Making
/// this an enum keeps the modals mutually exclusive by construction.
#[derive(Default)]
pub(crate) enum Overlay {
    #[default]
    None,
    /// Reference popup listing the metadata shortcut keys. The keys work whether or
    /// not it is shown, so this only aids discovery.
    MetadataMenu,
    /// The settings menu: a small chooser whose rows open the settings dialogs
    /// (currently just the theme picker).
    SettingsMenu,
    /// The theme picker list, live-previewing the highlighted theme.
    ThemePicker(ThemePickerState),
    ConfirmDelete(DeleteContext),
    NewJournal(TextInput),
    EditMetadata(EditMetadataState),
    EditFeelings(EditFeelingState),
    EditMood(EditMoodState),
    // Boxed: this state is much larger than the other variants (candidate/preset
    // lists), so keeping it behind a pointer keeps `Overlay` small.
    EditLocation(Box<EditLocationState>),
    ImageViewer(ImageViewerState),
    /// Shown over the editor while a save waits on the still-in-flight weather/air
    /// fetch. The `Instant` is when it opened, driving both the animated dots and
    /// the timeout after which the save proceeds without the data.
    FetchingEnvironment(Instant),
}
