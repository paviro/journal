//! The footer hint bar: the per-view hint sets, the wrap/justify grid the
//! hints render through, and the hit-testing that maps clicks back to hints.

use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::{App, Focus, Mode};
use crate::tui::theme::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HintId {
    /// Select all text in whichever single-line field owns the caret.
    InputSelectAll,
    NewJournal,
    ToggleArchiveJournal,
    NewEntry,
    BeginSearch,
    Quit,
    EditSelected,
    ViewSelected,
    BeginDelete,
    ToggleStarred,
    ExitSearch,
    CancelOverlay,
    CloseEntryView,
    ExpandEntryView,
    MetadataToggle,
    MetadataSwitchFocus,
    MetadataAddFromInput,
    MetadataSave,
    FeelingsToggle,
    FeelingsExpand,
    FeelingsCollapse,
    FeelingsSwitchFocus,
    FeelingsSave,
    MoodDecrease,
    MoodIncrease,
    MoodSave,
    MoodClear,
    LocationSwitchFocus,
    LocationResolve,
    LocationGrabDevice,
    LocationSelectRow,
    LocationSave,
    LocationClear,
    OpenImageViewer,
    OpenMetadataMenu,
    OpenSettings,
    ThemePickerApply,
    ThemePickerRevert,
    ThemePickerChrome,
    ThemePickerMode,
    HintsToggle,
    ToggleJournals,
    InsightsTab,
    InsightsScope,
    InsightsTimeframe,
    ExpandInsights,
    CloseInsights,
    EditorSave,
    EditorDiscard,
    EditorFullscreen,
    EditorMetadata,
    EditorHelp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hint {
    pub(crate) label: &'static str,
    pub(crate) key_hint: &'static str,
    pub(crate) id: HintId,
}

impl Hint {
    pub(crate) const fn new(label: &'static str, key_hint: &'static str, id: HintId) -> Self {
        Self {
            label,
            key_hint,
            id,
        }
    }

    fn text(self) -> String {
        format!("{} {}", key_chip_text(self.key_hint), self.label)
    }
}

/// Minimum blank columns kept around and between hints when a row is justified.
const HINT_MIN_GAP: usize = 2;

/// Saturating `usize`→`u16`, for column math that can never realistically overflow
/// a terminal but must stay in bounds.
pub(super) fn clamp_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// Space-around gap distribution: with `content_total` columns of content and
/// `gap_count` gaps (each already reserving [`HINT_MIN_GAP`]), spread the leftover
/// width evenly. Returns `(base, remainder)` — every gap grows by `base`, and the
/// first `remainder` gaps grow by one more.
fn spread_gaps(area: usize, content_total: usize, gap_count: usize) -> (usize, usize) {
    let extra = area.saturating_sub(content_total + gap_count * HINT_MIN_GAP);
    (extra / gap_count, extra % gap_count)
}

/// The key portion of a hint as plain text: a space on each side so the reversed
/// chip reads as a padded button. Kept in one place so the styled rendering and
/// the width/hit-test math stay in lockstep.
pub(super) fn key_chip_text(key: &str) -> String {
    format!(" {key} ")
}

/// The style for a hint's key chip. The token's default is the classic
/// inverted chip, so themes that never touch `key_hint` keep the pre-theme
/// footer on both chromes.
pub(super) fn key_chip_style() -> Style {
    theme().key_hint()
}

#[derive(Debug, Clone)]
struct RenderedHintLine {
    /// The row's full visual text of justified hints, identical to what is
    /// drawn — so `find`-based column lookups line up with hit-testing.
    text: String,
    /// `(start column, hint)` for each hint, columns absolute within the row.
    placements: Vec<(u16, Hint)>,
}

fn hint_width(hint: &Hint) -> usize {
    UnicodeWidthStr::width(hint.text().as_str())
}

/// The id of the hint whose justified span contains `col` (relative to `origin_x`).
fn placement_at(placements: &[(u16, Hint)], origin_x: u16, col: u16) -> Option<HintId> {
    let rel = col.checked_sub(origin_x)?;
    placements.iter().find_map(|(start, hint)| {
        let width = hint_width(hint) as u16;
        (rel >= *start && rel < start.saturating_add(width)).then_some(hint.id)
    })
}

/// Render a laid-out hint row as styled spans: the gaps stay plain and each key
/// chip is drawn reversed + bold. Columns match [`RenderedHintLine::text`]
/// exactly, so the visual output lines up with hit-testing. The hovered hint's
/// label lifts out of the muted row as the click affordance.
fn styled_hint_line(rendered: &RenderedHintLine, hovered: Option<HintId>) -> Line<'static> {
    if rendered.placements.is_empty() {
        return Line::from(rendered.text.clone());
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut col = 0u16;
    for (start, hint) in &rendered.placements {
        if *start > col {
            spans.push(Span::raw(" ".repeat((*start - col) as usize)));
            col = *start;
        }
        let chip = key_chip_text(hint.key_hint);
        col += clamp_u16(UnicodeWidthStr::width(chip.as_str()));
        spans.push(Span::styled(chip, key_chip_style()));
        let label = format!(" {}", hint.label);
        col += clamp_u16(UnicodeWidthStr::width(label.as_str()));
        spans.push(if hovered == Some(hint.id) {
            Span::styled(label, theme().text())
        } else {
            Span::styled(label, theme().muted())
        });
    }
    Line::from(spans)
}

pub(crate) fn hint_lines(
    hints: &[Hint],
    width: u16,
    hovered: Option<HintId>,
) -> Vec<Line<'static>> {
    rendered_hint_lines(hints, width)
        .iter()
        .map(|line| styled_hint_line(line, hovered))
        .collect()
}

pub(crate) fn hint_height(hints: &[Hint], width: u16) -> u16 {
    clamp_u16(rendered_hint_lines(hints, width).len().max(1))
}

/// The hint grid's rows joined by newlines, for tests to locate hints by text.
#[cfg(test)]
pub(crate) fn hint_grid_text(hints: &[Hint], width: u16) -> String {
    rendered_hint_lines(hints, width)
        .iter()
        .map(|row| row.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn hint_id_at_wrapped(
    hints: &[Hint],
    origin_x: u16,
    origin_y: u16,
    width: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    let relative_row = row.checked_sub(origin_y)? as usize;
    let lines = rendered_hint_lines(hints, width);
    let line = lines.get(relative_row)?;
    placement_at(&line.placements, origin_x, col)
}

/// Lay the hints out as a column grid: pick a column count that fits, then align
/// every row to the same column x-positions (each hint left-aligned in its column)
/// so wrapped rows line up vertically. Leftover width is spread across the gaps so
/// the grid still fills the row.
fn rendered_hint_lines(hints: &[Hint], width: u16) -> Vec<RenderedHintLine> {
    if hints.is_empty() {
        return Vec::new();
    }
    let mut columns = columns_that_fit(hints, width);
    let (col_x, rows) = loop {
        let rows: Vec<&[Hint]> = hints.chunks(columns).collect();
        let mut col_widths = vec![0usize; columns];
        for row in &rows {
            for (index, hint) in row.iter().enumerate() {
                col_widths[index] = col_widths[index].max(hint_width(hint));
            }
        }
        let total: usize = col_widths.iter().sum();
        let gap_count = columns + 1;
        if columns == 1 || total + gap_count * HINT_MIN_GAP <= width as usize {
            let (base, remainder) = spread_gaps(width as usize, total, gap_count);
            let mut col_x = Vec::with_capacity(columns);
            let mut x = 0usize;
            for (index, col_width) in col_widths.iter().enumerate() {
                x += HINT_MIN_GAP + base + usize::from(index < remainder);
                col_x.push(clamp_u16(x));
                x += col_width;
            }
            break (col_x, rows);
        }
        columns -= 1;
    };
    rows.iter().map(|row| build_grid_row(&col_x, row)).collect()
}

/// How many equal grid columns the hints can use: greedily fit as many as possible
/// on the first row (with minimum gaps), at least one.
fn columns_that_fit(hints: &[Hint], width: u16) -> usize {
    let width = width as usize;
    let mut used = HINT_MIN_GAP; // trailing edge gap
    let mut columns = 0;
    for hint in hints {
        let need = HINT_MIN_GAP + hint_width(hint);
        if columns > 0 && used + need > width {
            break;
        }
        used += need;
        columns += 1;
    }
    columns.max(1)
}

/// Place a row's hints at the shared column x-positions, left-aligned in each column.
fn build_grid_row(col_x: &[u16], hints: &[Hint]) -> RenderedHintLine {
    let mut text = String::new();
    let mut col = 0u16;
    let mut placements = Vec::with_capacity(hints.len());
    for (index, hint) in hints.iter().enumerate() {
        let start = col_x[index];
        while col < start {
            text.push(' ');
            col += 1;
        }
        placements.push((start, *hint));
        text.push_str(&hint.text());
        col += hint_width(hint) as u16;
    }
    RenderedHintLine { text, placements }
}

/// The footer's justified rows joined by newlines, for tests to inspect.
#[cfg(test)]
pub(crate) fn footer_text(app: &App, width: u16) -> String {
    if app.editor.is_some() {
        return editor_footer_line()
            .rendered_lines(width)
            .iter()
            .map(|row| row.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }
    let line = match app.nav.mode {
        Mode::Search => search_footer_line(app),
        Mode::Browse => browse_footer_line(app),
    };
    line.rendered_lines(width)
        .iter()
        .map(|row| row.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Footer hints shown while the internal editor is open, in both its in-pane and
/// full-screen forms.
fn editor_footer_line() -> HintLine {
    HintLine {
        hints: vec![
            Hint::new("save", "ctrl+s", HintId::EditorSave),
            Hint::new("discard", "esc", HintId::EditorDiscard),
            Hint::new("fullscreen", "ctrl+o", HintId::EditorFullscreen),
            Hint::new("metadata", "ctrl+g", HintId::EditorMetadata),
            Hint::new("shortcuts", "ctrl+t", HintId::EditorHelp),
        ],
    }
}

pub(crate) fn footer_lines(app: &App, width: u16) -> Text<'static> {
    let hovered = app.hovered_footer_hint();
    if app.editor.is_some() {
        return Text::from(editor_footer_line().lines(width, hovered));
    }
    if !app.state.ui.show_hints {
        return Text::default();
    }

    let lines = match app.nav.mode {
        Mode::Search => search_footer_line(app).lines(width, hovered),
        Mode::Browse => browse_footer_line(app).lines(width, hovered),
    };
    Text::from(lines)
}

pub(crate) fn footer_height(app: &App, width: u16) -> u16 {
    if app.editor.is_some() {
        return editor_footer_line().height(width);
    }
    if !app.state.ui.show_hints {
        return 0;
    }

    match app.nav.mode {
        Mode::Search => search_footer_line(app).height(width),
        Mode::Browse => browse_footer_line(app).height(width),
    }
}

#[cfg(test)]
pub(crate) fn footer_hint_id_at(app: &App, origin_x: u16, width: u16, col: u16) -> Option<HintId> {
    if app.editor.is_some() {
        return editor_footer_line()
            .rendered_lines(width)
            .first()
            .and_then(|row| placement_at(&row.placements, origin_x, col));
    }
    let line = match app.nav.mode {
        Mode::Search => search_footer_line(app),
        Mode::Browse => browse_footer_line(app),
    };
    line.rendered_lines(width)
        .first()
        .and_then(|row| placement_at(&row.placements, origin_x, col))
}

pub(crate) fn footer_hint_id_at_point(
    app: &App,
    origin_x: u16,
    origin_y: u16,
    width: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    if app.editor.is_some() {
        return editor_footer_line().hint_id_at_point(origin_x, origin_y, width, col, row);
    }
    if !app.state.ui.show_hints {
        return None;
    }

    match app.nav.mode {
        Mode::Search => {
            search_footer_line(app).hint_id_at_point(origin_x, origin_y, width, col, row)
        }
        Mode::Browse => {
            browse_footer_line(app).hint_id_at_point(origin_x, origin_y, width, col, row)
        }
    }
}

/// The expanded footer's justified rows joined by newlines, for tests.
#[cfg(test)]
pub(crate) fn expanded_footer_text(app: &App, width: u16) -> String {
    if app.editor.is_some() {
        return editor_footer_line()
            .rendered_lines(width)
            .iter()
            .map(|row| row.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }
    rendered_hint_lines(&expanded_footer_hints(app), width.saturating_sub(1))
        .iter()
        .map(|row| row.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn expanded_footer_lines(app: &App, width: u16) -> Text<'static> {
    let hovered = app.hovered_footer_hint();
    if app.editor.is_some() {
        return Text::from(editor_footer_line().lines(width, hovered));
    }
    if !app.state.ui.show_hints {
        return Text::default();
    }
    Text::from(hint_lines(
        &expanded_footer_hints(app),
        width.saturating_sub(1),
        hovered,
    ))
}

pub(crate) fn expanded_footer_height(app: &App, width: u16) -> u16 {
    if app.editor.is_some() {
        return editor_footer_line().height(width);
    }
    if !app.state.ui.show_hints {
        return 0;
    }
    hint_height(&expanded_footer_hints(app), width.saturating_sub(1))
}

pub(crate) fn expanded_footer_hint_id_at_point(
    app: &App,
    origin_x: u16,
    origin_y: u16,
    width: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    if app.editor.is_some() {
        return editor_footer_line().hint_id_at_point(origin_x, origin_y, width, col, row);
    }
    if !app.state.ui.show_hints {
        return None;
    }
    hint_id_at_wrapped(
        &expanded_footer_hints(app),
        origin_x.saturating_add(1),
        origin_y,
        width.saturating_sub(1),
        col,
        row,
    )
}

#[derive(Debug, Clone)]
struct HintLine {
    hints: Vec<Hint>,
}

impl HintLine {
    fn rendered_lines(&self, width: u16) -> Vec<RenderedHintLine> {
        rendered_hint_lines(&self.hints, width)
    }

    fn lines(&self, width: u16, hovered: Option<HintId>) -> Vec<Line<'static>> {
        self.rendered_lines(width)
            .iter()
            .map(|line| styled_hint_line(line, hovered))
            .collect()
    }

    fn height(&self, width: u16) -> u16 {
        clamp_u16(self.rendered_lines(width).len().max(1))
    }

    fn hint_id_at_point(
        &self,
        origin_x: u16,
        origin_y: u16,
        width: u16,
        col: u16,
        row: u16,
    ) -> Option<HintId> {
        let relative_row = row.checked_sub(origin_y)? as usize;
        let lines = self.rendered_lines(width);
        let line = lines.get(relative_row)?;
        placement_at(&line.placements, origin_x, col)
    }
}

fn search_footer_line(app: &App) -> HintLine {
    // The query now lives on the entry panel's top-right border (see
    // `draw_entry_list`), so the footer only carries the action hints.
    let hints = match app.nav.focus {
        Focus::EntryView if app.has_selected_entry_target() => {
            let mut hints = selected_entry_action_hints(Some(EXPAND_ENTER_HINT));
            hints.extend(image_hint(app));
            hints.push(Hint::new("exit search", "esc", HintId::ExitSearch));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::EntryView => vec![
            Hint::new("exit search", "esc", HintId::ExitSearch),
            Hint::new("quit", "q", HintId::Quit),
        ],
        _ => {
            let mut hints = Vec::new();
            if app.has_selected_entry_target() {
                hints.push(Hint::new("view", "enter", HintId::ViewSelected));
            }
            hints.push(Hint::new("exit search", "esc", HintId::ExitSearch));
            hints
        }
    };

    HintLine { hints }
}

fn browse_footer_line(app: &App) -> HintLine {
    let hints = match app.nav.focus {
        Focus::Journals => {
            let mut hints = vec![Hint::new("new journal", "n", HintId::NewJournal)];
            hints.extend(archive_hint(app));
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(SETTINGS_HINT);
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::Insights => {
            let mut hints = vec![
                Hint::new("tabs", "←/→", HintId::InsightsTab),
                Hint::new("scope", "g", HintId::InsightsScope),
            ];
            if app.nav.insights_tab.uses_timeframe() {
                hints.push(Hint::new("window", "w", HintId::InsightsTimeframe));
            }
            if app.nav.insights_fullscreen {
                hints.push(Hint::new("close", "enter/esc", HintId::CloseInsights));
            } else {
                hints.push(Hint::new("expand", "enter", HintId::ExpandInsights));
            }
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(SETTINGS_HINT);
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::Entries => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.extend(selected_entry_action_hints(Some(VIEW_ENTER_HINT)));
            }
            // The image viewer opens only from a focused entry view, so no
            // `images` hint here.
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(SETTINGS_HINT);
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::EntryView => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.extend(selected_entry_action_hints(Some(EXPAND_ENTER_HINT)));
            }
            hints.extend(image_hint(app));
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(SETTINGS_HINT);
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
    };

    HintLine { hints }
}

/// The `images (i)` hint, shown only when the selected entry has images.
fn image_hint(app: &App) -> Option<Hint> {
    (app.selected_entry_image_count() > 0).then_some(Hint::new(
        "images",
        "i",
        HintId::OpenImageViewer,
    ))
}

/// The `archive`/`unarchive (a)` hint, shown only when a journal is selected. The
/// label reflects the selected journal's current state.
fn archive_hint(app: &App) -> Option<Hint> {
    app.selected_journal().map(|journal| {
        let label = if journal.archived {
            "unarchive"
        } else {
            "archive"
        };
        Hint::new(label, "a", HintId::ToggleArchiveJournal)
    })
}

/// The `settings (,)` hint, shown in every Browse-mode footer.
const SETTINGS_HINT: Hint = Hint::new("settings", ",", HintId::OpenSettings);

fn journals_hint(app: &App) -> Hint {
    let label = if app.state.ui.show_journals {
        "hide journals"
    } else {
        "journals"
    };
    Hint::new(label, "j", HintId::ToggleJournals)
}

/// The edit/enter/del/metadata action hints for a selected entry. `enter` is the
/// hint for the Enter key, which differs by focus: in the list it views the entry,
/// in the focused viewer it expands to full screen. `None` omits it.
fn selected_entry_action_hints(enter: Option<Hint>) -> Vec<Hint> {
    let mut hints = vec![Hint::new("edit", "e", HintId::EditSelected)];
    hints.extend(enter);
    hints.push(Hint::new("del", "d", HintId::BeginDelete));
    hints.push(Hint::new("metadata", "ctrl+g", HintId::OpenMetadataMenu));
    hints
}

/// Enter views the entry from the list.
const VIEW_ENTER_HINT: Hint = Hint::new("view", "enter", HintId::ViewSelected);

/// Enter expands the entry from the focused (multi-column) viewer.
const EXPAND_ENTER_HINT: Hint = Hint::new("expand", "enter", HintId::ExpandEntryView);

/// The `close` hint for the full-screen viewer. Enter and Esc always close it; in
/// multi-column full screen Left is inert, while single-column also exits on Left.
fn close_entry_view_hint(app: &App) -> Hint {
    let keys = if app.nav.entry_view_fullscreen {
        "enter/esc"
    } else {
        "enter/esc/←"
    };
    Hint::new("close", keys, HintId::CloseEntryView)
}

fn expanded_footer_hints(app: &App) -> Vec<Hint> {
    let mut hints = Vec::new();
    if app.nav.mode == Mode::Browse {
        hints.push(Hint::new("new entry", "n", HintId::NewEntry));
    }
    if app.has_selected_entry_target() {
        hints.push(Hint::new("edit", "e", HintId::EditSelected));
        hints.push(close_entry_view_hint(app));
        hints.push(Hint::new("del", "d", HintId::BeginDelete));
        hints.push(Hint::new("metadata", "ctrl+g", HintId::OpenMetadataMenu));
        hints.push(Hint::new("star", "s", HintId::ToggleStarred));
        hints.extend(image_hint(app));
    } else {
        hints.push(close_entry_view_hint(app));
    }
    if app.nav.mode == Mode::Browse {
        hints.push(Hint::new("search", "/", HintId::BeginSearch));
    }
    hints.push(Hint::new("hints", "h", HintId::HintsToggle));
    hints.push(Hint::new("quit", "q", HintId::Quit));
    hints
}

// ── Chrome style: flat (bg-layered) vs bordered ───────────────────────────────
