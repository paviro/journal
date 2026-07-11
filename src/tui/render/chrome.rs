use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use unicode_width::UnicodeWidthStr;

use super::table;
use crate::tui::app::{App, Focus, Mode};
use crate::tui::state::{MetadataKind, ToastVariant};
use crate::tui::surface::point_in_rect;
use crate::tui::theme::{ChromeStyle, theme};

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
fn clamp_u16(n: usize) -> u16 {
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
fn key_chip_text(key: &str) -> String {
    format!(" {key} ")
}

/// The style for a hint's key chip.
fn key_chip_style() -> Style {
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
/// label lifts out of the muted/plain row (underlined in bordered chrome so it
/// reads without color) as the click affordance.
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
        // Flat chrome mutes the labels so the key chips carry the row, like a
        // status bar; bordered keeps the classic plain labels.
        spans.push(if hovered == Some(hint.id) {
            if flat_chrome() {
                Span::styled(label, theme().text())
            } else {
                Span::styled(label, Style::default().add_modifier(Modifier::UNDERLINED))
            }
        } else if flat_chrome() {
            Span::styled(label, theme().muted())
        } else {
            Span::raw(label)
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

/// True when the active theme separates surfaces by background layers instead
/// of drawn borders.
pub(crate) fn flat_chrome() -> bool {
    theme().chrome() == ChromeStyle::Flat
}

/// The style painted under a whole frame: the theme background plus its default
/// text color, so spans without an explicit fg stay readable on it. A no-op
/// under terminal-default themes (both components are `Reset`/absent).
pub(crate) fn base_style() -> Style {
    let mut style = Style::default().bg(theme().bg());
    if let Some(fg) = theme().text().fg {
        style = style.fg(fg);
    }
    style
}

/// Dim everything drawn so far, so an overlay rendered afterwards floats on a
/// darkened backdrop. True-color cells blend toward black by the theme's scrim
/// strength; palette/terminal-default cells (and strength 0) fall back to the
/// DIM modifier. Cells owned by terminal graphics protocols (`skip`) can't be
/// restyled and stay bright.
pub(crate) fn scrim(buf: &mut Buffer, area: Rect) {
    let keep = 1.0 - theme().scrim_strength().clamp(0.0, 1.0);
    let mul = |channel: u8| (f32::from(channel) * keep) as u8;
    for pos in area.positions() {
        let cell = &mut buf[pos];
        if cell.diff_option == ratatui::buffer::CellDiffOption::Skip {
            continue;
        }
        let mut blended = false;
        if keep < 1.0 {
            for color in [&mut cell.fg, &mut cell.bg] {
                if let Color::Rgb(r, g, b) = *color {
                    *color = Color::Rgb(mul(r), mul(g), mul(b));
                    blended = true;
                }
            }
        }
        if !blended {
            cell.modifier.insert(Modifier::DIM);
        }
    }
}

// ── Toasts ────────────────────────────────────────────────────────────────────

/// Widest a toast gets; narrower terminals shrink it further.
const TOAST_MAX_WIDTH: u16 = 44;
/// Longest a toast message renders before ellipsizing.
const TOAST_MAX_LINES: usize = 4;
/// Blank columns kept between a toast and the terminal's right edge.
const TOAST_RIGHT_INSET: u16 = 2;

fn toast_style(variant: ToastVariant) -> Style {
    match variant {
        ToastVariant::Info => theme().info(),
        ToastVariant::Success => theme().success(),
        ToastVariant::Warning => theme().warning(),
        ToastVariant::Error => theme().error(),
    }
}

/// The on-screen rect of each visible toast, oldest first. The draw and the
/// mouse hit-test both derive from this one geometry, so a click or hover can
/// never miss what's painted. Stacking stops once a toast no longer fits the
/// remaining height.
pub(crate) fn toast_rects(app: &App, area: Rect) -> Vec<Rect> {
    let width = TOAST_MAX_WIDTH.min(area.width.saturating_sub(6));
    if width <= 4 {
        return Vec::new();
    }
    let x = area.right().saturating_sub(TOAST_RIGHT_INSET + width);
    let mut y = area.y + 1;
    let mut rects = Vec::new();
    for toast in app.toasts.items() {
        let lines = crate::tui::entry_rows::wrap_text(
            &toast.message,
            (width - 4) as usize,
            TOAST_MAX_LINES,
        );
        let height = clamp_u16(lines.len()) + 2;
        if y + height > area.bottom() {
            break;
        }
        rects.push(Rect::new(x, y, width, height));
        y += height + 1;
    }
    rects
}

/// The index of the toast under `(col, row)`, if any.
pub(crate) fn toast_at_point(app: &App, area: Rect, col: u16, row: u16) -> Option<usize> {
    toast_rects(app, area)
        .into_iter()
        .position(|rect| point_in_rect(rect, col, row))
}

/// Draw the toast stack in the top-right corner, oldest at the top with a blank
/// row between toasts. Runs at the very end of the frame — after overlays and
/// the scrim — so notifications stay readable over everything.
pub(crate) fn draw_toasts(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    for (index, (toast, rect)) in app
        .toasts
        .items()
        .iter()
        .zip(toast_rects(app, area))
        .enumerate()
    {
        let lines = crate::tui::entry_rows::wrap_text(
            &toast.message,
            rect.width.saturating_sub(4) as usize,
            TOAST_MAX_LINES,
        );
        let hovered = app.hover == crate::tui::state::HoverTarget::Toast(index);
        draw_toast(frame, rect, toast.variant, &lines, hovered);
    }
}

/// One toast box. Flat chrome paints a panel-colored card with thick `┃` edge
/// columns in the variant's hue; bordered chrome draws a plain box with the
/// variant-colored border. Both keep one padding column inside the edges and
/// one padding row above and below the text. A hovered toast lifts to the
/// hover surface as the click-to-dismiss affordance.
fn draw_toast(
    frame: &mut Frame<'_>,
    area: Rect,
    variant: ToastVariant,
    lines: &[String],
    hovered: bool,
) {
    frame.render_widget(Clear, area);
    let accent = toast_style(variant);
    let text: Vec<Line<'static>> = lines
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), theme().text())))
        .collect();
    let content = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    };
    if flat_chrome() {
        // The element surface, not the panel one: toasts float over panels
        // that already carry `panel_bg`, so on the same color only the edge
        // stripes would separate them.
        let surface = if hovered {
            theme().hover()
        } else {
            Style::default().bg(theme().element_bg())
        };
        frame.render_widget(Block::new().style(surface), area);
        for edge_x in [area.x, area.right().saturating_sub(1)] {
            let stripe: Vec<Line<'static>> = (0..area.height)
                .map(|_| Line::from(Span::styled("┃", accent)))
                .collect();
            frame.render_widget(
                Paragraph::new(stripe),
                Rect {
                    x: edge_x,
                    width: 1,
                    ..area
                },
            );
        }
    } else {
        frame.render_widget(
            Block::default().borders(Borders::ALL).border_style(accent),
            area,
        );
    }
    frame.render_widget(Paragraph::new(text), content);
}

/// A dialog's content rect within its outer `area`. Draw functions and mouse
/// hit-tests both derive geometry from this one place, so they can never
/// drift apart. Bordered chrome insets by the border; flat chrome trades the
/// side borders for a wider breathing margin.
pub(crate) fn dialog_inner(area: Rect) -> Rect {
    // Saturating per-axis (unlike `Rect::inner`, which zeroes the whole rect):
    // sizing helpers probe with height-1 rects and still need the real width.
    let horizontal = if flat_chrome() { 2 } else { 1 };
    Rect {
        x: area.x.saturating_add(horizontal),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(horizontal * 2),
        height: area.height.saturating_sub(2),
    }
}

/// Clear and frame a dialog, returning its content rect (always
/// [`dialog_inner`] of `area`). Bordered chrome draws the classic titled box;
/// flat chrome paints a panel-colored surface with a bold title row and, when
/// `esc_hint` is set, a muted `esc` dismiss hint on the right.
pub(crate) fn draw_dialog_frame(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    esc_hint: bool,
) -> Rect {
    frame.render_widget(Clear, area);
    let title = title.trim();
    if flat_chrome() {
        frame.render_widget(
            Block::new().style(Style::default().bg(theme().panel_bg())),
            area,
        );
        let top = Rect {
            x: area.x + 2,
            y: area.y,
            width: area.width.saturating_sub(4),
            height: 1.min(area.height),
        };
        if !title.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(title.to_string(), theme().heading())),
                top,
            );
        }
        if esc_hint {
            frame.render_widget(
                Paragraph::new(Span::styled("esc", theme().muted())).alignment(Alignment::Right),
                top,
            );
        }
    } else {
        let mut block = Block::default().borders(Borders::ALL);
        if !title.is_empty() {
            block = block.title(format!(" {title} "));
        }
        frame.render_widget(block, area);
    }
    dialog_inner(area)
}

/// The marker shown before a selected list row: a bullet on flat chrome, the
/// classic `>` on bordered.
pub(crate) fn list_highlight_symbol() -> &'static str {
    if flat_chrome() { "● " } else { ">" }
}

/// The style for the thin `─` rules that subdivide dialogs.
pub(crate) fn separator_style() -> Style {
    if flat_chrome() {
        theme().faint_rule()
    } else {
        theme().muted()
    }
}

/// A titled content container inside a full-screen modal (unlock, pending
/// notices). Bordered chrome keeps the padded box; flat chrome swaps the
/// border for a panel background with the same inner geometry.
pub(crate) fn container_block(title: &str) -> Block<'static> {
    if flat_chrome() {
        Block::new()
            .style(Style::default().bg(theme().panel_bg()))
            .padding(Padding::new(3, 3, 2, 2))
            .title_top(Line::from(Span::styled(
                format!(" {title} "),
                theme().heading(),
            )))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title_top(Line::from(format!(" {title} ")))
            .padding(Padding::new(2, 2, 1, 1))
    }
}

/// In flat chrome the focused panel is marked by a `┃` stripe down its left
/// padding column — the borders that used to thicken are gone, so focus needs
/// its own ink. No-op on bordered chrome or unfocused panels.
pub(crate) fn panel_focus_stripe(frame: &mut Frame<'_>, area: Rect, focused: bool) {
    if !flat_chrome() || !focused || area.width == 0 {
        return;
    }
    let stripe: Vec<Line<'static>> = (0..area.height)
        .map(|_| Line::from(Span::styled("┃", theme().focus_border())))
        .collect();
    frame.render_widget(Paragraph::new(stripe), Rect { width: 1, ..area });
}

pub(crate) fn panel_block(
    title: &str,
    focused: bool,
    footer_label: Option<String>,
) -> Block<'static> {
    if flat_chrome() {
        let mut block = Block::new()
            .style(Style::default().bg(theme().panel_bg()))
            .padding(Padding::uniform(1))
            .title(panel_title(title, focused));
        if let Some(label) = footer_label {
            block = block.title_bottom(
                Line::from(Span::styled(format!(" {label} "), theme().muted())).right_aligned(),
            );
        }
        return block;
    }

    let mut block = Block::default()
        .title(panel_title(title, focused))
        .borders(Borders::ALL);

    if focused {
        block = block
            .border_type(BorderType::Thick)
            .border_style(theme().focus_border());
    }

    if let Some(label) = footer_label {
        block = block.title_bottom(Line::from(format!(" {label} ")).right_aligned());
    }

    block
}

/// Draw a dimmed message centered both horizontally and vertically within a
/// panel's content area — used for empty states like "No entry selected" and
/// "No results".
pub(crate) fn render_centered_notice(frame: &mut Frame<'_>, content: Rect, message: &str) {
    if content.width == 0 || content.height == 0 {
        return;
    }
    let line = Rect {
        y: content.y + content.height.saturating_sub(1) / 2,
        height: 1,
        ..content
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(theme().muted()),
        line,
    );
}

// ── Confirm-dialog buttons (shared by confirm-delete and editor discard) ─────

/// Width and gap of the two confirm buttons; sized for a comfortable click target
/// with room for the label and its key hint.
const CONFIRM_BUTTON_WIDTH: u16 = 16;
const CONFIRM_BUTTON_GAP: u16 = 2;

/// The `(yes, no)` button rects, centered on the last row of `inner`. Sizing and
/// hit-testing both derive from this, so the drawn buttons match the click targets.
pub(crate) fn confirm_button_rects(inner: Rect) -> (Rect, Rect) {
    let y = inner.y + inner.height.saturating_sub(1);
    let total = CONFIRM_BUTTON_WIDTH * 2 + CONFIRM_BUTTON_GAP;
    let start = inner.x + inner.width.saturating_sub(total) / 2;
    let yes = Rect {
        x: start,
        y,
        width: CONFIRM_BUTTON_WIDTH,
        height: 1,
    };
    let no = Rect {
        x: start + CONFIRM_BUTTON_WIDTH + CONFIRM_BUTTON_GAP,
        ..yes
    };
    (yes, no)
}

/// Draw the two confirm buttons as reversed + bold chips on the last row of
/// `inner`. The hovered button underlines as the click affordance — the chips
/// are already filled/reversed, so a surface change wouldn't read.
pub(crate) fn render_confirm_buttons(
    frame: &mut Frame<'_>,
    inner: Rect,
    yes_label: &str,
    no_label: &str,
    hovered: Option<bool>,
) {
    let (yes, no) = confirm_button_rects(inner);
    for (area, label, is_yes) in [(yes, yes_label, true), (no, no_label, false)] {
        // Flat chrome draws opencode-style filled chips; bordered keeps the
        // bracketed reversed buttons. Same rects either way, so the click
        // targets from `confirm_button_rects` stay valid.
        let (text, mut style) = if flat_chrome() {
            (format!(" {label} "), theme().button())
        } else {
            (format!("[ {label} ]"), key_chip_style())
        };
        if hovered == Some(is_yes) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        frame.render_widget(
            Paragraph::new(Span::styled(text, style)).alignment(Alignment::Center),
            area,
        );
    }
}

/// Map a click to a confirm button: `Some(true)` for yes, `Some(false)` for no.
pub(crate) fn confirm_button_at(inner: Rect, col: u16, row: u16) -> Option<bool> {
    let (yes, no) = confirm_button_rects(inner);
    if point_in_rect(yes, col, row) {
        Some(true)
    } else if point_in_rect(no, col, row) {
        Some(false)
    } else {
        None
    }
}

/// Draw the internal editor's "Discard changes?" confirmation as a centered
/// modal, matching the confirm-delete dialog's look.
pub(crate) fn draw_editor_discard_confirm(frame: &mut Frame<'_>, hovered_button: Option<bool>) {
    let area = editor_discard_confirm_area(frame.area());
    let inner = draw_dialog_frame(frame, area, "Discard Changes", true);
    let line = Rect {
        y: inner.y,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new("Discard unsaved changes?").alignment(Alignment::Center),
        line,
    );
    render_confirm_buttons(frame, inner, "Discard (y)", "Keep (n)", hovered_button);
}

pub(crate) fn editor_discard_confirm_area(frame_area: Rect) -> Rect {
    centered_rect_fixed_size(42, 5, frame_area)
}

pub(crate) fn editor_discard_choice_at_point(frame_area: Rect, col: u16, row: u16) -> Option<bool> {
    let inner = dialog_inner(editor_discard_confirm_area(frame_area));
    confirm_button_at(inner, col, row)
}

const METADATA_MENU_ITEMS: [(&str, &str); 6] = [
    ("t", "Tags"),
    ("p", "People"),
    ("a", "Activities"),
    ("f", "Feelings"),
    ("m", "Mood"),
    ("l", "Location"),
];

fn metadata_menu_rows() -> Vec<Vec<String>> {
    METADATA_MENU_ITEMS
        .iter()
        .map(|(key, label)| vec![key.to_string(), label.to_string()])
        .collect()
}

fn metadata_menu_dialog(rows: &[Vec<String>], mode: MetadataMenuMode) -> TableDialog<'_> {
    TableDialog {
        title: "Add Metadata",
        headers: &["Key", "Add"],
        rows,
        key_col: 0,
        footer: mode.footer(),
    }
}

/// Where the metadata chooser is shown. The editor gates its metadata keys behind
/// this popup ("press key"); the viewer's keys work at any time, so there the popup
/// is only a reference ("reference").
#[derive(Clone, Copy)]
pub(crate) enum MetadataMenuMode {
    Editor,
    Viewer,
}

impl MetadataMenuMode {
    fn footer(self) -> &'static str {
        match self {
            Self::Editor => "press key · esc",
            Self::Viewer => "reference · esc",
        }
    }
}

/// Draw the "Add metadata" chooser: a centered popup whose highlighted letters open
/// the tags/people/activities/feelings/mood dialogs, laid out as a table matching
/// the insights tabs. Shared by the internal editor and the entry viewer.
pub(crate) fn draw_metadata_menu(
    frame: &mut Frame<'_>,
    mode: MetadataMenuMode,
    hovered_row: Option<usize>,
) {
    let rows = metadata_menu_rows();
    // The chooser always fits, so it never scrolls.
    let mut scroll = 0;
    draw_table_dialog(
        frame,
        &metadata_menu_dialog(&rows, mode),
        &mut scroll,
        hovered_row,
    );
}

/// The metadata-menu data row under `(col, row)`, for hover highlighting.
pub(crate) fn metadata_menu_row_at_point(
    frame_area: Rect,
    mode: MetadataMenuMode,
    col: u16,
    row: u16,
) -> Option<usize> {
    let rows = metadata_menu_rows();
    table_dialog_row_at_point(frame_area, &metadata_menu_dialog(&rows, mode), 0, col, row)
}

pub(crate) enum MetadataChoice {
    Metadata(MetadataKind),
    Feelings,
    Mood,
    Location,
}

pub(crate) fn metadata_menu_choice_at_point(
    frame_area: Rect,
    mode: MetadataMenuMode,
    col: u16,
    row: u16,
) -> Option<MetadataChoice> {
    let rows = metadata_menu_rows();
    let index =
        table_dialog_row_at_point(frame_area, &metadata_menu_dialog(&rows, mode), 0, col, row)?;
    match index {
        0 => Some(MetadataChoice::Metadata(MetadataKind::Tags)),
        1 => Some(MetadataChoice::Metadata(MetadataKind::People)),
        2 => Some(MetadataChoice::Metadata(MetadataKind::Activities)),
        3 => Some(MetadataChoice::Feelings),
        4 => Some(MetadataChoice::Mood),
        5 => Some(MetadataChoice::Location),
        _ => None,
    }
}

pub(crate) fn metadata_menu_close_at_point(
    frame_area: Rect,
    mode: MetadataMenuMode,
    col: u16,
    row: u16,
) -> bool {
    let rows = metadata_menu_rows();
    table_dialog_footer_at_point(frame_area, &metadata_menu_dialog(&rows, mode), 0, col, row)
}

const SETTINGS_MENU_ITEMS: [(&str, &str); 1] = [("t", "Theme…")];

fn settings_menu_rows() -> Vec<Vec<String>> {
    SETTINGS_MENU_ITEMS
        .iter()
        .map(|(key, label)| vec![key.to_string(), label.to_string()])
        .collect()
}

fn settings_menu_dialog(rows: &[Vec<String>]) -> TableDialog<'_> {
    TableDialog {
        title: "Settings",
        headers: &["Key", "Setting"],
        rows,
        key_col: 0,
        footer: "enter select · esc close",
    }
}

/// A row of the settings menu, mapped from a click by
/// [`settings_menu_choice_at_point`].
pub(crate) enum SettingsChoice {
    Theme,
}

/// Draw the settings menu: a centered chooser whose rows open the settings
/// dialogs. Same table popup as the metadata menu.
pub(crate) fn draw_settings_menu(frame: &mut Frame<'_>, hovered_row: Option<usize>) {
    let rows = settings_menu_rows();
    // The menu always fits, so it never scrolls.
    let mut scroll = 0;
    draw_table_dialog(
        frame,
        &settings_menu_dialog(&rows),
        &mut scroll,
        hovered_row,
    );
}

/// The settings-menu data row under `(col, row)`, for hover highlighting.
pub(crate) fn settings_menu_row_at_point(frame_area: Rect, col: u16, row: u16) -> Option<usize> {
    let rows = settings_menu_rows();
    table_dialog_row_at_point(frame_area, &settings_menu_dialog(&rows), 0, col, row)
}

pub(crate) fn settings_menu_choice_at_point(
    frame_area: Rect,
    col: u16,
    row: u16,
) -> Option<SettingsChoice> {
    let rows = settings_menu_rows();
    let index = table_dialog_row_at_point(frame_area, &settings_menu_dialog(&rows), 0, col, row)?;
    match index {
        0 => Some(SettingsChoice::Theme),
        _ => None,
    }
}

pub(crate) fn settings_menu_close_at_point(frame_area: Rect, col: u16, row: u16) -> bool {
    let rows = settings_menu_rows();
    table_dialog_footer_at_point(frame_area, &settings_menu_dialog(&rows), 0, col, row)
}

const EDITOR_SHORTCUT_SECTIONS: [(&str, &[(&str, &str)]); 3] = [
    (
        "File",
        &[
            ("ctrl+s", "Save"),
            ("ctrl+o", "Fullscreen"),
            ("ctrl+g", "Metadata"),
            ("esc", "Discard"),
        ],
    ),
    (
        "Edit",
        &[
            ("ctrl+a", "Select all"),
            ("ctrl+u", "Undo"),
            ("ctrl+r", "Redo"),
            ("ctrl+x", "Cut"),
            ("ctrl+c", "Copy"),
            ("ctrl+y", "Paste"),
            ("ctrl+k", "Cut to line end"),
            ("ctrl+w", "Delete word"),
        ],
    ),
    (
        "Move",
        &[
            ("arrows", "Move"),
            ("shift+move", "Select"),
            ("ctrl+←/→", "Word"),
            ("home/end", "Line start/end"),
            ("ctrl+↑/↓", "Paragraph"),
            ("pgup/pgdn", "Page"),
        ],
    ),
];

fn editor_shortcut_rows() -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for (group, items) in EDITOR_SHORTCUT_SECTIONS {
        for (i, (keys, action)) in items.iter().enumerate() {
            let group = if i == 0 { group } else { "" };
            rows.push(vec![
                group.to_string(),
                keys.to_string(),
                action.to_string(),
            ]);
        }
    }
    rows
}

fn editor_shortcut_dialog(rows: &[Vec<String>]) -> TableDialog<'_> {
    TableDialog {
        title: "Editor Shortcuts",
        headers: &["Group", "Key", "Action"],
        rows,
        key_col: 1,
        footer: "reference · esc",
    }
}

/// Draw the internal editor's shortcut reference: a bordered table listing every
/// key the editor honors, grouped by purpose. Opened with Ctrl+T, scrolled with
/// the arrows/page keys, dismissed by any other key. Keeps the always-on footer
/// lean while staying fully discoverable.
pub(crate) fn draw_editor_shortcuts(frame: &mut Frame<'_>, scroll: &mut u16) {
    let rows = editor_shortcut_rows();
    draw_table_dialog(frame, &editor_shortcut_dialog(&rows), scroll, None);
}

pub(crate) fn editor_shortcut_hint_at_point(
    frame_area: Rect,
    scroll: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    let rows = editor_shortcut_rows();
    let index =
        table_dialog_row_at_point(frame_area, &editor_shortcut_dialog(&rows), scroll, col, row)?;
    match index {
        0 => Some(HintId::EditorSave),
        1 => Some(HintId::EditorFullscreen),
        2 => Some(HintId::EditorMetadata),
        3 => Some(HintId::EditorDiscard),
        _ => None,
    }
}

/// Column content widths for a dialog table, each fitting the widest cell (and the
/// header). The key column reserves the two spaces of its reversed key chip.
fn dialog_widths(headers: &[&str], rows: &[Vec<String>], key_col: usize) -> Vec<usize> {
    (0..headers.len())
        .map(|c| {
            rows.iter()
                .map(|row| {
                    let len = UnicodeWidthStr::width(row[c].as_str());
                    if c == key_col && !row[c].is_empty() {
                        len + 2
                    } else {
                        len
                    }
                })
                .chain(std::iter::once(UnicodeWidthStr::width(headers[c])))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

/// One data cell's spans, padded to `width`: the key column as a reversed key chip
/// (one space each side) left-aligned; a leading group column bold; the rest plain.
fn dialog_cell(text: &str, col: usize, key_col: usize, width: usize) -> Vec<Span<'static>> {
    if col == key_col && !text.is_empty() {
        let chip = key_chip_text(text);
        let padding = width.saturating_sub(UnicodeWidthStr::width(chip.as_str()));
        return vec![
            Span::styled(chip, key_chip_style()),
            Span::raw(" ".repeat(padding)),
        ];
    }
    let style = if col == 0 && col != key_col {
        theme().heading()
    } else {
        theme().text()
    };
    vec![Span::styled(pad_display(text, width), style)]
}

/// Left-pad `text` to `width` display columns.
fn pad_display(text: &str, width: usize) -> String {
    let pad = width.saturating_sub(UnicodeWidthStr::width(text));
    format!("{text}{}", " ".repeat(pad))
}

/// A faint inter-row rule. Columns whose cell in `row` is empty (a spanning group
/// cell continuing from the row above) are left blank so the group reads as one
/// merged cell rather than a stack of separately-ruled blanks.
fn row_separator(widths: &[usize], row: &[String], muted: Style) -> Line<'static> {
    let faint = table::faint_rule_style();
    let mut spans = vec![Span::styled("│".to_string(), muted)];
    for (c, w) in widths.iter().enumerate() {
        if c > 0 {
            spans.push(Span::styled("│".to_string(), muted));
        }
        if row[c].is_empty() {
            spans.push(Span::raw(" ".repeat(w + 2)));
        } else {
            spans.push(Span::styled("─".repeat(w + 2), faint));
        }
    }
    spans.push(Span::styled("│".to_string(), muted));
    Line::from(spans)
}

/// The full bordered grid (insights style): outer border, muted header, and a faint
/// rule between each row. Returns the lines and the table's total column width.
fn grid_table(headers: &[&str], rows: &[Vec<String>], key_col: usize) -> (Vec<Line<'static>>, u16) {
    let widths = dialog_widths(headers, rows, key_col);
    let muted = table::border_style();

    let mut lines = Vec::with_capacity(2 * rows.len() + 4);
    lines.push(table::rule(&widths, '┌', '┬', '┐', muted, muted));
    let mut header = vec![table::border()];
    for (c, label) in headers.iter().enumerate() {
        table::push_cell_spans(
            &mut header,
            vec![Span::styled(table::pad(label, widths[c], false), muted)],
        );
    }
    lines.push(Line::from(header));
    lines.push(table::rule(&widths, '├', '┼', '┤', muted, muted));
    for (r, row) in rows.iter().enumerate() {
        // A faint rule between rows, its column borders running straight through as
        // plain `│` so the verticals stay continuous — matching the insights table.
        // A spanning group cell (empty on continuation rows) keeps its rule blank so
        // it reads as one merged cell.
        if r > 0 {
            lines.push(row_separator(&widths, row, muted));
        }
        let mut spans = vec![table::border()];
        for (c, text) in row.iter().enumerate() {
            table::push_cell_spans(&mut spans, dialog_cell(text, c, key_col, widths[c]));
        }
        lines.push(Line::from(spans));
    }
    lines.push(table::rule(&widths, '└', '┴', '┘', muted, muted));

    // Each column renders as `│ <content> `; the last cell adds the closing `│`.
    let width = widths.iter().map(|w| w + 3).sum::<usize>() + 1;
    (lines, width as u16)
}

/// The chrome-less fallback: one data row per line (no borders or rules), columns
/// aligned and separated by two spaces — the same collapse the insights tabs use
/// when there isn't room for the full grid.
fn compact_table(
    headers: &[&str],
    rows: &[Vec<String>],
    key_col: usize,
) -> (Vec<Line<'static>>, u16) {
    let widths = dialog_widths(headers, rows, key_col);
    let lines: Vec<Line<'static>> = rows
        .iter()
        .map(|row| {
            let mut spans = Vec::new();
            for (c, text) in row.iter().enumerate() {
                if c > 0 {
                    spans.push(Span::raw("  "));
                }
                spans.extend(dialog_cell(text, c, key_col, widths[c]));
            }
            Line::from(spans)
        })
        .collect();
    let width = widths.iter().sum::<usize>() + 2 * headers.len().saturating_sub(1);
    (lines, width as u16)
}

/// A table popup's content, independent of where it lands on screen: title,
/// column headers, data rows, the key column to render as a chip, and the
/// bottom-border footer label. Built once per dialog and threaded through the
/// draw and both hit-tests so they can't disagree on what's being shown.
struct TableDialog<'a> {
    title: &'a str,
    headers: &'a [&'a str],
    rows: &'a [Vec<String>],
    key_col: usize,
    footer: &'a str,
}

/// The full geometry and rendered content of a table dialog, computed once and
/// shared by [`draw_table_dialog`] and the mouse hit-tests so the click map can
/// never drift from the pixels.
struct TableDialogMetrics {
    area: Rect,
    content: Rect,
    /// The rendered body: the full bordered grid, or the chrome-less collapse.
    lines: Vec<Line<'static>>,
    /// The bottom-border label, with the `↑↓ scroll` prefix already added when the
    /// content overflows.
    footer: String,
    /// Whether `lines` is the bordered grid (vs the compact collapse); the row
    /// hit-test needs this to map a line back to its data row.
    grid: bool,
    total: u16,
    scroll: u16,
}

fn table_dialog_metrics(frame_area: Rect, dialog: &TableDialog, scroll: u16) -> TableDialogMetrics {
    let (grid_lines, grid_w) = grid_table(dialog.headers, dialog.rows, dialog.key_col);
    let avail_h = frame_area.height.saturating_sub(2).max(3);
    let (lines, content_w, grid) = if grid_lines.len() as u16 + 2 <= avail_h {
        (grid_lines, grid_w, true)
    } else {
        let (compact_lines, compact_w) = compact_table(dialog.headers, dialog.rows, dialog.key_col);
        (compact_lines, compact_w, false)
    };
    let total = lines.len() as u16;
    let outer_h = (total + 2).min(avail_h);
    let footer = if total > outer_h.saturating_sub(2) {
        format!("↑↓ scroll · {}", dialog.footer)
    } else {
        dialog.footer.to_string()
    };
    let border_label = |text: &str| UnicodeWidthStr::width(text) as u16 + 4;
    let outer_w = (content_w + 4)
        .max(border_label(dialog.title))
        .max(border_label(&footer))
        .min(frame_area.width);
    let area = centered_rect_fixed_size(outer_w, outer_h, frame_area);
    let inner = dialog_inner(area);
    let content_w = content_w.min(inner.width);
    let content = Rect {
        x: inner.x + (inner.width - content_w) / 2,
        width: content_w,
        ..inner
    };
    let max_offset = total.saturating_sub(content.height);
    TableDialogMetrics {
        area,
        content,
        lines,
        footer,
        grid,
        total,
        scroll: scroll.min(max_offset),
    }
}

fn table_dialog_row_at_point(
    frame_area: Rect,
    dialog: &TableDialog,
    scroll: u16,
    col: u16,
    row: u16,
) -> Option<usize> {
    let metrics = table_dialog_metrics(frame_area, dialog, scroll);
    if !point_in_rect(metrics.content, col, row) {
        return None;
    }
    let visible_line = row - metrics.content.y;
    let content_line = visible_line + metrics.scroll;
    let index = if metrics.grid {
        if content_line < 3 || !(content_line - 3).is_multiple_of(2) {
            return None;
        }
        (content_line - 3) / 2
    } else {
        content_line
    };
    (index as usize)
        .lt(&dialog.rows.len())
        .then_some(index as usize)
}

fn table_dialog_footer_at_point(
    frame_area: Rect,
    dialog: &TableDialog,
    scroll: u16,
    col: u16,
    row: u16,
) -> bool {
    let metrics = table_dialog_metrics(frame_area, dialog, scroll);
    row == metrics.area.y + metrics.area.height.saturating_sub(1)
        && col >= metrics.area.x
        && col < metrics.area.x + metrics.area.width
}

pub(crate) fn editor_shortcut_close_at_point(
    frame_area: Rect,
    scroll: u16,
    col: u16,
    row: u16,
) -> bool {
    let rows = editor_shortcut_rows();
    table_dialog_footer_at_point(frame_area, &editor_shortcut_dialog(&rows), scroll, col, row)
}

/// Draw a centered dialog: the usual titled border box (with `footer` on its bottom
/// border) wrapping a table of `rows` with one column of space each side. Shows the
/// full bordered grid when the box is tall enough, otherwise collapses to chrome-
/// less rows — like the insights tabs. `scroll` drives a scrollbar when the content
/// still overflows.
fn draw_table_dialog(
    frame: &mut Frame<'_>,
    dialog: &TableDialog,
    scroll: &mut u16,
    hovered_row: Option<usize>,
) {
    let mut metrics = table_dialog_metrics(frame.area(), dialog, *scroll);
    *scroll = metrics.scroll;
    // Lift the hovered data row; the line index mirrors
    // `table_dialog_row_at_point`'s mapping so hover and click can't disagree.
    if let Some(row) = hovered_row {
        let line = if metrics.grid { 3 + 2 * row } else { row };
        if let Some(line) = metrics.lines.get_mut(line) {
            line.style = line.style.patch(theme().hover());
        }
    }

    if flat_chrome() {
        draw_dialog_frame(frame, metrics.area, dialog.title, false);
        // The footer moves from the bottom border to the bottom margin row.
        let bottom = Rect {
            y: metrics.area.y + metrics.area.height.saturating_sub(1),
            height: 1,
            ..metrics.area
        };
        frame.render_widget(
            Paragraph::new(Span::styled(metrics.footer.clone(), theme().muted()))
                .alignment(Alignment::Center),
            bottom,
        );
    } else {
        frame.render_widget(Clear, metrics.area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .title_bottom(Line::from(format!(" {} ", metrics.footer)).centered())
            .borders(Borders::ALL);
        frame.render_widget(block, metrics.area);
    }

    frame.render_widget(
        Paragraph::new(metrics.lines).scroll((metrics.scroll, 0)),
        metrics.content,
    );
    render_scrollbar_if_needed(
        frame,
        metrics.area,
        metrics.total as usize,
        metrics.content.height,
        metrics.scroll as usize,
    );
}

/// Draw the full-screen "journal chrome" frame shared by the startup modals
/// (unlock, device-access request, and the enroll/awaiting/disable notices): a
/// bordered block titled top-left with the screen name and, when `key_hint` is
/// non-empty, bottom-right with its key hints. Clears the screen first and
/// returns the inner area to lay the modal's content into.
pub(crate) fn draw_modal_frame(frame: &mut Frame<'_>, title: &str, key_hint: &str) -> Rect {
    let area = frame.area();
    frame.render_widget(Clear, area);

    if flat_chrome() {
        // No outer border: the screen name and hints float on the app
        // background, quiet in the corners like a status bar.
        frame.buffer_mut().set_style(area, base_style());
        let top = Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: 1.min(area.height),
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {title} "), theme().muted())),
            top,
        );
        if !key_hint.is_empty() && area.height > 1 {
            let bottom = Rect {
                y: area.y + area.height - 1,
                ..top
            };
            frame.render_widget(
                Paragraph::new(Span::styled(format!(" {key_hint} "), theme().muted()))
                    .alignment(Alignment::Right),
                bottom,
            );
        }
        return area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
    }

    let mut block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(format!(" {title} ")));
    if !key_hint.is_empty() {
        block = block.title_bottom(Line::from(format!(" {key_hint} ")).right_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

pub(crate) fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

pub(crate) fn panel_title(title: &str, focused: bool) -> Line<'static> {
    let label = format!(" {title} ");
    if flat_chrome() {
        // No border to thicken, so the title itself carries focus: accent+bold
        // when focused, receding to muted otherwise.
        let style = if focused {
            theme().primary().add_modifier(Modifier::BOLD)
        } else {
            theme().muted()
        };
        return Line::from(Span::styled(label, style));
    }
    if focused {
        Line::from(Span::styled(label, theme().selection()))
    } else {
        Line::from(label)
    }
}

pub(crate) fn render_vertical_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut ScrollbarState,
) {
    let mut scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    if flat_chrome() {
        scrollbar = scrollbar
            .thumb_style(theme().focus_border())
            .track_style(theme().faint_rule());
    }
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        state,
    );
}

pub(crate) fn render_scrollbar_if_needed(
    frame: &mut Frame<'_>,
    area: Rect,
    total_height: usize,
    viewport_height: u16,
    scroll: usize,
) {
    if total_height > viewport_height as usize {
        let mut state = ScrollbarState::default()
            .content_length(total_height)
            .viewport_content_length(viewport_height as usize)
            .position(crate::tui::scroll::scrollbar_position(
                scroll,
                total_height,
                viewport_height,
            ));
        render_vertical_scrollbar(frame, area, &mut state);
    }
}

pub(crate) fn centered_rect_fixed_size(width: u16, height: u16, area: Rect) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(row);
    cell
}
