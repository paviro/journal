use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::{App, Focus, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HintId {
    NewJournal,
    ToggleArchiveJournal,
    NewEntry,
    BeginSearch,
    Quit,
    EditSelected,
    ViewSelected,
    BeginDelete,
    BeginEditTags,
    BeginEditPeople,
    BeginEditActivities,
    BeginEditFeelings,
    BeginEditMood,
    ToggleStarred,
    ExitSearch,
    CancelOverlay,
    CloseEntryView,
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
    BeginEditLocation,
    LocationSwitchFocus,
    LocationResolve,
    LocationGrabDevice,
    LocationSelectRow,
    LocationSave,
    LocationClear,
    OpenImageViewer,
    HintsToggle,
    ToggleJournals,
    InsightsTab,
    InsightsScope,
    InsightsTimeframe,
    ExpandInsights,
    CloseInsights,
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

/// The reversed + bold style for a hint's key chip.
fn key_chip_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

#[derive(Debug, Clone)]
struct RenderedHintLine {
    /// The row's full visual text (prefix + justified hints), identical to what is
    /// drawn — so `find`-based column lookups line up with hit-testing.
    text: String,
    /// A left-hand prefix (status label), drawn plain before the hints. Usually empty.
    prefix: String,
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

/// Lay out one already-packed row: `prefix` at the far left, then the hints spaced
/// evenly across the rest of `width` (space-around — roughly equal padding at both
/// ends and between hints). Returns the drawn text and each hint's start column.
fn layout_hint_row(prefix: &str, hints: &[Hint], width: u16) -> RenderedHintLine {
    let prefix_width = UnicodeWidthStr::width(prefix);
    let mut text = String::from(prefix);
    if hints.is_empty() {
        return RenderedHintLine {
            text,
            prefix: prefix.to_string(),
            placements: Vec::new(),
        };
    }

    let widths: Vec<usize> = hints.iter().map(hint_width).collect();
    let hint_total: usize = widths.iter().sum();
    let area = (width as usize).saturating_sub(prefix_width);
    // n hints → n+1 gaps (both ends + between); only the leading n are drawn, the
    // trailing one is implicit right padding. Spread the slack beyond the minimum
    // gaps evenly across all n+1.
    let gap_count = hints.len() + 1;
    let (base, remainder) = spread_gaps(area, hint_total, gap_count);

    let mut col = prefix_width;
    let mut placements = Vec::with_capacity(hints.len());
    for (index, hint) in hints.iter().enumerate() {
        let gap = HINT_MIN_GAP + base + usize::from(index < remainder);
        for _ in 0..gap {
            text.push(' ');
        }
        col += gap;
        placements.push((clamp_u16(col), *hint));
        text.push_str(&hint.text());
        col += widths[index];
    }
    RenderedHintLine {
        text,
        prefix: prefix.to_string(),
        placements,
    }
}

/// Render a laid-out hint row as styled spans: the prefix and gaps stay plain and
/// each key chip is drawn reversed + bold. Columns match [`RenderedHintLine::text`]
/// exactly, so the visual output lines up with hit-testing.
fn styled_hint_line(rendered: &RenderedHintLine) -> Line<'static> {
    if rendered.placements.is_empty() {
        return Line::from(rendered.text.clone());
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut col = 0u16;
    if !rendered.prefix.is_empty() {
        col = clamp_u16(UnicodeWidthStr::width(rendered.prefix.as_str()));
        spans.push(Span::raw(rendered.prefix.clone()));
    }
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
        spans.push(Span::raw(label));
    }
    Line::from(spans)
}

pub(crate) fn hint_lines(hints: &[Hint], width: u16) -> Vec<Line<'static>> {
    rendered_hint_lines(hints, width)
        .iter()
        .map(styled_hint_line)
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
    RenderedHintLine {
        text,
        prefix: String::new(),
        placements,
    }
}

fn wrapped_hint_rows(hints: &[Hint], width: u16) -> Vec<Vec<Hint>> {
    let available = width as usize;
    let mut rows: Vec<Vec<Hint>> = Vec::new();
    let mut row: Vec<Hint> = Vec::new();
    // Reserve a trailing edge gap; each hint reserves a leading gap plus its width
    // (matching the space-around layout), so a packed row is always justifiable.
    let mut used = HINT_MIN_GAP;

    for hint in hints.iter().copied() {
        let need = HINT_MIN_GAP + hint_width(&hint);
        if !row.is_empty() && used + need > available {
            rows.push(std::mem::take(&mut row));
            used = HINT_MIN_GAP;
        }
        used += need;
        row.push(hint);
    }

    if !row.is_empty() {
        rows.push(row);
    }

    rows
}

/// The footer's justified rows joined by newlines, for tests to inspect.
#[cfg(test)]
pub(crate) fn footer_text(app: &App, width: u16) -> String {
    if !app.status().is_empty() {
        return app.status().to_string();
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

pub(crate) fn footer_lines(app: &App, width: u16) -> Text<'static> {
    if !app.status().is_empty() {
        return Text::from(app.status().to_string());
    }
    if !app.state.ui.show_hints {
        return Text::default();
    }

    let lines = match app.nav.mode {
        Mode::Search => search_footer_line(app).lines(width),
        Mode::Browse => browse_footer_line(app).lines(width),
    };
    Text::from(lines)
}

pub(crate) fn footer_height(app: &App, width: u16) -> u16 {
    if !app.status().is_empty() {
        return 1;
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
    if !app.status().is_empty() {
        return None;
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
    if !app.status().is_empty() || !app.state.ui.show_hints {
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
    rendered_hint_lines(&expanded_footer_hints(app), width.saturating_sub(1))
        .iter()
        .map(|row| row.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn expanded_footer_lines(app: &App, width: u16) -> Text<'static> {
    if !app.state.ui.show_hints {
        return Text::default();
    }
    Text::from(hint_lines(
        &expanded_footer_hints(app),
        width.saturating_sub(1),
    ))
}

pub(crate) fn expanded_footer_height(app: &App, width: u16) -> u16 {
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
    prefix: Option<String>,
    hints: Vec<Hint>,
}

impl HintLine {
    fn rendered_lines(&self, width: u16) -> Vec<RenderedHintLine> {
        let prefix = self.prefix.as_deref().unwrap_or("");
        if self.hints.is_empty() {
            return if prefix.is_empty() {
                Vec::new()
            } else {
                vec![layout_hint_row(prefix, &[], width)]
            };
        }
        if prefix.is_empty() {
            return rendered_hint_lines(&self.hints, width);
        }

        let prefix_width = clamp_u16(UnicodeWidthStr::width(prefix));
        let first_hint_width = self.hints.first().map(hint_width).unwrap_or(0) as u16;
        let mut lines = Vec::new();
        let mut remaining = self.hints.as_slice();
        if prefix_width
            .saturating_add(HINT_MIN_GAP as u16)
            .saturating_add(first_hint_width)
            <= width
        {
            let first_area = width.saturating_sub(prefix_width);
            if let Some(first_row) = wrapped_hint_rows(remaining, first_area).first() {
                lines.push(layout_hint_row(prefix, first_row, width));
                remaining = &remaining[first_row.len()..];
            }
        } else {
            lines.push(layout_hint_row(prefix, &[], width));
        }
        lines.extend(rendered_hint_lines(remaining, width));
        lines
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        self.rendered_lines(width)
            .iter()
            .map(styled_hint_line)
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
            let mut hints = selected_entry_action_hints(true);
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

    HintLine {
        prefix: None,
        hints,
    }
}

fn browse_footer_line(app: &App) -> HintLine {
    let hints = match app.nav.focus {
        Focus::Journals => {
            let mut hints = vec![Hint::new("new journal", "n", HintId::NewJournal)];
            hints.extend(archive_hint(app));
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
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
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::Entries => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.extend(selected_entry_action_hints(true));
            }
            // The image viewer opens only from a focused entry view, so no
            // `images` hint here.
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::EntryView => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.extend(selected_entry_action_hints(true));
            }
            hints.extend(image_hint(app));
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
    };

    HintLine {
        prefix: None,
        hints,
    }
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

fn journals_hint(app: &App) -> Hint {
    let label = if app.state.ui.show_journals {
        "hide journals"
    } else {
        "journals"
    };
    Hint::new(label, "j", HintId::ToggleJournals)
}

fn selected_entry_action_hints(include_view: bool) -> Vec<Hint> {
    let mut hints = Vec::new();
    hints.push(Hint::new("edit", "e", HintId::EditSelected));
    if include_view {
        hints.push(Hint::new("view", "enter", HintId::ViewSelected));
    }
    hints.push(Hint::new("del", "d", HintId::BeginDelete));
    hints.push(Hint::new("tags", "t", HintId::BeginEditTags));
    hints.push(Hint::new("feel", "f", HintId::BeginEditFeelings));
    hints.push(Hint::new("mood", "m", HintId::BeginEditMood));
    hints.push(Hint::new("location", "l", HintId::BeginEditLocation));
    hints
}

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
        hints.push(Hint::new("tags", "t", HintId::BeginEditTags));
        hints.push(Hint::new("ppl", "p", HintId::BeginEditPeople));
        hints.push(Hint::new("act", "a", HintId::BeginEditActivities));
        hints.push(Hint::new("feel", "f", HintId::BeginEditFeelings));
        hints.push(Hint::new("mood", "m", HintId::BeginEditMood));
        hints.push(Hint::new("location", "l", HintId::BeginEditLocation));
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

pub(crate) fn panel_block(
    title: &str,
    focused: bool,
    footer_label: Option<String>,
) -> Block<'static> {
    let mut block = Block::default()
        .title(panel_title(title, focused))
        .borders(Borders::ALL);

    if focused {
        block = block
            .border_type(BorderType::Thick)
            .border_style(Style::default().add_modifier(Modifier::BOLD));
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
            .style(Style::default().add_modifier(Modifier::DIM)),
        line,
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
    if focused {
        Line::from(Span::styled(
            label,
            Style::default().add_modifier(Modifier::REVERSED),
        ))
    } else {
        Line::from(label)
    }
}

pub(crate) fn render_vertical_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut ScrollbarState,
) {
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
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
