//! Responsive building blocks shared by the insight tabs. Every widget measures
//! the `Rect` it is handed and adapts — none knows whether it is drawing into a
//! side column or an expanded full-screen panel. All colour comes from
//! [`theme`], so the blocks stay legible with the palette stripped.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::entry_rows::{DividerAlign, section_divider, text_width, truncate_ellipsis};
use crate::tui::render::flat_chrome;
use crate::tui::theme::theme;

/// Intra-panel composition breakpoints, measured from the content `Rect` (not
/// the terminal). Named so the responsiveness tests can pin them.
pub(crate) const TWO_COL_MIN_WIDTH: u16 = 56;
pub(crate) const THREE_COL_MIN_WIDTH: u16 = 92;
/// Below this height a tab drops boxed cards for compact one-line rows.
pub(crate) const SHORT_HEIGHT: u16 = 14;

/// One entry in a vertical [`stack`]: a minimum height to be included at all,
/// and a fill weight for sharing the leftover rows.
pub(crate) struct Section {
    pub(crate) min: u16,
    pub(crate) fill: u16,
}

impl Section {
    pub(crate) fn new(min: u16, fill: u16) -> Self {
        Self { min, fill }
    }
}

/// Stack `sections` top-to-bottom in `area`. Each is included only if its `min`
/// still fits (later ones drop out first — put highest-signal sections first);
/// leftover height is shared by `fill` weight, with any rounding remainder given
/// to the last filling section so the area is fully used. Returns one rect per
/// section (`None` when it didn't fit).
pub(crate) fn stack(area: Rect, sections: &[Section]) -> Vec<Option<Rect>> {
    let mut included = Vec::with_capacity(sections.len());
    let mut used = 0u16;
    for section in sections {
        let fits = used + section.min <= area.height;
        if fits {
            used += section.min;
        }
        included.push(fits);
    }

    let total_fill: u16 = sections
        .iter()
        .zip(&included)
        .filter(|(_, keep)| **keep)
        .map(|(section, _)| section.fill)
        .sum();
    let extra = area.height - used;

    // Pre-compute each section's fill share; hand the remainder to the last
    // filling section so no rows are left blank at the bottom.
    let last_fill = sections
        .iter()
        .enumerate()
        .rev()
        .find(|(idx, section)| included[*idx] && section.fill > 0)
        .map(|(idx, _)| idx);
    let mut assigned = 0u16;

    let mut result = vec![None; sections.len()];
    let mut y = area.y;
    for (idx, section) in sections.iter().enumerate() {
        if !included[idx] {
            continue;
        }
        let mut height = section.min;
        if total_fill > 0 && section.fill > 0 {
            let mut share = extra * section.fill / total_fill;
            if Some(idx) == last_fill {
                share = extra - assigned;
            }
            assigned += share;
            height += share;
        }
        result[idx] = Some(Rect {
            x: area.x,
            y,
            width: area.width,
            height,
        });
        y += height;
    }
    result
}

/// Draw a section heading as the app's shared divider rule — a bold left label
/// trailed by a `━` line (`Balance ━━━━━━`), matching the entry list's month
/// headers and the journals column's "Archived" divider. One blank row precedes
/// it to set it off from the section above; the title takes the lone row when the
/// area is a single line high. Returns the area below the title.
pub(crate) fn heading(frame: &mut Frame<'_>, area: Rect, text: &str) -> Rect {
    if area.height == 0 {
        return area;
    }
    let title_y = if area.height >= 2 { area.y + 1 } else { area.y };
    frame.render_widget(
        Paragraph::new(section_divider(
            area.width as usize,
            text,
            DividerAlign::Left,
        )),
        Rect {
            y: title_y,
            height: 1,
            ..area
        },
    );
    let used = title_y + 1 - area.y;
    Rect {
        y: title_y + 1,
        height: area.height - used,
        ..area
    }
}

/// Draw a dim caption line (e.g. a histogram axis) in `area`'s first row.
pub(crate) fn caption(frame: &mut Frame<'_>, area: Rect, text: &str) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text.to_string(), theme().muted()))),
        Rect { height: 1, ..area },
    );
}

/// How many card columns the area affords.
pub(crate) fn columns_for(area: Rect) -> usize {
    if area.width >= THREE_COL_MIN_WIDTH {
        3
    } else if area.width >= TWO_COL_MIN_WIDTH {
        2
    } else {
        1
    }
}

/// Whether the area is too short for boxed cards / multi-widget stacks.
pub(crate) fn is_short(area: Rect) -> bool {
    area.height < SHORT_HEIGHT
}

/// Split `area` into a row-major grid of `cols × rows` even cells.
pub(crate) fn grid(area: Rect, cols: usize, rows: usize) -> Vec<Rect> {
    if cols == 0 || rows == 0 {
        return Vec::new();
    }
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Fill(1); rows])
        .split(area);
    let mut cells = Vec::with_capacity(cols * rows);
    for row in row_rects.iter() {
        let col_rects = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Fill(1); cols])
            .split(*row);
        cells.extend(col_rects.iter().copied());
    }
    cells
}

/// A single headline metric: a dim label, a bold value, and an optional dim
/// sub-line (a secondary figure, trend, or unit).
pub(crate) struct Stat {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) value_style: ratatui::style::Style,
    pub(crate) sub: Option<Span<'static>>,
}

impl Stat {
    pub(crate) fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            value_style: theme().heading(),
            sub: None,
        }
    }

    pub(crate) fn sub(mut self, sub: Span<'static>) -> Self {
        self.sub = Some(sub);
        self
    }
}

/// Lay a row of headline metrics out as boxed cards, collapsing to compact
/// one-line rows when the area is short.
pub(crate) fn draw_stats(frame: &mut Frame<'_>, area: Rect, stats: &[Stat]) {
    if stats.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    if is_short(area) {
        let lines: Vec<Line> = stats.iter().map(stat_row_line).collect();
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let cols = columns_for(area).min(stats.len());
    let rows = stats.len().div_ceil(cols);
    // Flat cards have no borders, so the contiguous grid cells would abut. Trim a
    // gutter off the trailing edge of every interior cell — 2 columns between
    // columns, 1 row between rows — matching the Overview grid's flat spacing.
    let (hgap, vgap) = if flat_chrome() { (2, 1) } else { (0, 0) };
    for (index, (mut cell, stat)) in grid(area, cols, rows).into_iter().zip(stats).enumerate() {
        if index % cols < cols - 1 {
            cell.width = cell.width.saturating_sub(hgap);
        }
        if index / cols < rows - 1 {
            cell.height = cell.height.saturating_sub(vgap);
        }
        draw_stat_card(frame, cell, stat);
    }
}

/// A metric as one compact line: `Label  Value  sub`.
fn stat_row_line(stat: &Stat) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("{}  ", stat.label), theme().muted()),
        Span::styled(stat.value.clone(), stat.value_style),
    ];
    if let Some(sub) = &stat.sub {
        spans.push(Span::raw(" "));
        spans.push(sub.clone());
    }
    Line::from(spans)
}

/// One metric as a bordered card: value centered and bold, label dim above. Falls
/// back to a single centered line if the cell is too short to box. The caller sizes
/// the card — keep it compact so the tile hugs its content rather than boxing empty
/// space.
pub(crate) fn draw_stat_card(frame: &mut Frame<'_>, area: Rect, stat: &Stat) {
    if area.height < 3 || area.width < 4 {
        frame.render_widget(
            Paragraph::new(stat_row_line(stat)).alignment(Alignment::Center),
            area,
        );
        return;
    }
    let lines = vec![
        Line::from(Span::styled(stat.label.clone(), theme().muted())),
        Line::from(Span::styled(stat.value.clone(), stat.value_style)),
    ];
    // Flat mode drops the border and fills the tile with the card surface colour;
    // bordered mode keeps the drawn box. The inner height loses the two border rows
    // only when a border is drawn.
    let flat = flat_chrome();
    let block = if flat {
        Block::new().style(Style::default().bg(theme().raised_bg()))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme().card_border())
    };
    // Vertically centre the two lines (label / value) in the card; round the pad up
    // so the block never hugs the top edge on an even inner height.
    let inner_height = if flat {
        area.height
    } else {
        area.height.saturating_sub(2)
    } as usize;
    let pad_top = inner_height.saturating_sub(2).div_ceil(2);
    let lines = std::iter::repeat_n(Line::default(), pad_top)
        .chain(lines)
        .collect::<Vec<_>>();
    let card = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(block);
    frame.render_widget(card, area);
}

/// One horizontal bar: a label, a 0..1 fill, a value caption, and the fill style.
pub(crate) struct Bar {
    pub(crate) label: String,
    pub(crate) fill: f32,
    pub(crate) value: String,
    pub(crate) style: ratatui::style::Style,
}

/// Render `bars` as `label ████····  value`, showing the top rows that fit plus
/// a dim `+k more` footer when the list overflows the area.
pub(crate) fn draw_bars(frame: &mut Frame<'_>, area: Rect, bars: &[Bar]) {
    if area.width == 0 || area.height == 0 || bars.is_empty() {
        return;
    }
    let (rows_area, shown, more) = list_regions(area, bars.len());
    let label_w = bars
        .iter()
        .map(|bar| text_width(&bar.label))
        .max()
        .unwrap_or(0)
        .min(14);
    let value_w = bars.iter().map(|bar| bar.value.len()).max().unwrap_or(0);
    // label + ' ' + bar + ' ' + value
    let bar_w = (rows_area.width as usize)
        .saturating_sub(label_w + value_w + 2)
        .max(1);

    let lines: Vec<Line> = bars
        .iter()
        .take(shown)
        .map(|bar| {
            let filled = ((bar.fill.clamp(0.0, 1.0) * bar_w as f32).round() as usize).min(bar_w);
            Line::from(vec![
                Span::raw(format!(
                    "{:<label_w$}",
                    truncate_ellipsis(&bar.label, label_w)
                )),
                Span::raw(" "),
                // The fill glyph (default `▓`, dark shade) shares the airy
                // texture of the empty track rather than reading as a slab;
                // themes may swap either glyph.
                Span::styled(
                    theme().chart_bar().glyph.to_string().repeat(filled),
                    bar.style,
                ),
                Span::styled(
                    theme()
                        .chart_track()
                        .glyph
                        .to_string()
                        .repeat(bar_w - filled),
                    theme().chart_track().style,
                ),
                Span::raw(" "),
                Span::raw(format!("{:>value_w$}", bar.value)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), rows_area);
    draw_more_note(frame, more);
}

/// Split `area` into a rows region and, when `total` overflows, a one-line
/// `+k more` footer. Returns the rows rect, how many to draw, and the footer.
pub(crate) fn list_regions(area: Rect, total: usize) -> (Rect, usize, Option<(Rect, String)>) {
    let capacity = area.height as usize;
    if total <= capacity {
        return (area, total, None);
    }
    let shown = capacity.saturating_sub(1);
    let rows = Rect {
        height: shown as u16,
        ..area
    };
    let footer = Rect {
        y: area.y + shown as u16,
        height: 1,
        ..area
    };
    (
        rows,
        shown,
        Some((footer, format!("+{} more", total - shown))),
    )
}

pub(crate) fn draw_more_note(frame: &mut Frame<'_>, more: Option<(Rect, String)>) {
    if let Some((area, text)) = more {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, theme().muted()))),
            area,
        );
    }
}

/// A cell of the themed up ramp (`charts.glyphs.ramp`): index 0 is blank, 8 full.
fn ramp_cell(ramp: &[char; 9], eighths: usize) -> char {
    ramp[eighths.min(8)]
}

/// The downward counterpart of [`ramp_cell`] (`charts.glyphs.ramp_down`), for
/// bars hanging below a baseline. Only the "Block Elements" upper glyphs are
/// universally rendered by terminal fonts (the finer eighth-height *upper* blocks
/// live in "Symbols for Legacy Computing", which most fonts lack), so the tip
/// quantises to empty / one-eighth / half / full — four cells.
fn ramp_cell_down(ramp: &[char; 4], eighths: usize) -> char {
    let level = match eighths.min(8) {
        0 => 0,
        1..=2 => 1,
        3..=6 => 2,
        _ => 3,
    };
    ramp[level]
}

/// A vertical bar chart of signed averages around a zero baseline: each column
/// grows *up* (positive, green) or *down* (negative, red) from a dim mid-line. The
/// chart **auto-scales to its own largest-magnitude bar**, so the extreme value
/// fills its half and the rest are drawn in proportion — clustered values (e.g.
/// every weekday slightly negative) stay legible, and the shortest bar still marks
/// the best/least-bad bucket. Scale is therefore relative, not an absolute value.
/// The height is fixed by `area`, never by the data; the bottom row carries dim
/// `labels` centred under their columns (truncated to the column width, so wide
/// columns show a full year and narrow ones an initial). `None` values leave an
/// empty column with just its baseline tick, so gaps still read positionally.
pub(crate) fn draw_signed_columns(
    frame: &mut Frame<'_>,
    area: Rect,
    values: &[Option<f32>],
    labels: &[&str],
) {
    let n = values.len();
    if area.width == 0 || area.height < 2 || n == 0 {
        return;
    }
    // Consistent spacing that doesn't depend on the column count: a full gap cell
    // between bars, and half that (`edge`) as the leading/trailing padding, so the
    // strip hugs the chart edges more tightly than the bars sit from each other.
    // The bars absorb the leftover width (the first `extra` are one cell wider)
    // rather than centring the strip. `col_w(i)` is column `i`'s bar width.
    let width = area.width as usize;
    let gap = usize::from(width > 2 * n);
    let edge = gap / 2;
    let inner = width.saturating_sub(2 * edge + (n - 1) * gap);
    let base_w = (inner / n).max(1);
    let extra = inner % n;
    let col_w = |i: usize| base_w + usize::from(i < extra);

    // Rows: a label row at the bottom, a baseline row, and the plot split evenly
    // into a positive half above and a negative half below the baseline.
    let plot_h = area.height as usize - 1;
    let up_h = plot_h.saturating_sub(1) / 2;
    let down_h = plot_h - 1 - up_h;
    let baseline_row = area.y + up_h as u16;
    let label_row = area.y + area.height - 1;
    let up_eighths = up_h * 8;
    let down_eighths = down_h * 8;
    // Scale to the largest magnitude present, so the extreme bar fills its half and
    // differences between clustered values stay visible. Guard the all-zero case.
    let range = values
        .iter()
        .flatten()
        .fold(0.0_f32, |max, avg| max.max(avg.abs()))
        .max(f32::EPSILON);
    let norm = |avg: f32| (avg / range).clamp(-1.0, 1.0);

    // The character each column shows on plot row `y`, with whether it is a
    // positive (green) or negative (red) cell.
    let ramps = theme().glyphs().ramps;
    let column_cell = |value: &Option<f32>, y: u16| -> (char, bool) {
        match value {
            Some(avg) if *avg > 0.0 && y < baseline_row => {
                let level = (baseline_row - y) as usize; // 1 = nearest the baseline
                let filled = (norm(*avg) * up_eighths as f32).round() as usize;
                (
                    ramp_cell(&ramps.up, filled.saturating_sub((level - 1) * 8)),
                    true,
                )
            }
            Some(avg) if *avg < 0.0 && y > baseline_row => {
                let level = (y - baseline_row) as usize;
                let filled = (-norm(*avg) * down_eighths as f32).round() as usize;
                (
                    ramp_cell_down(&ramps.down, filled.saturating_sub((level - 1) * 8)),
                    false,
                )
            }
            _ => (' ', true),
        }
    };

    for r in 0..plot_h {
        let y = area.y + r as u16;
        if y == baseline_row {
            continue; // drawn as one dim rule below.
        }
        let mut spans = vec![Span::raw(" ".repeat(edge))];
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" ".repeat(gap)));
            }
            let (ch, positive) = column_cell(value, y);
            let style = if positive {
                theme().positive()
            } else {
                theme().negative()
            };
            spans.push(Span::styled(ch.to_string().repeat(col_w(i)), style));
        }
        spans.push(Span::raw(" ".repeat(edge)));
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                y,
                height: 1,
                ..area
            },
        );
    }

    // The dim zero baseline: a solid rule under each bar, the theme's lighter
    // tick across the gaps. The solid `─` stays fixed — the tick-vs-solid
    // weight difference is what marks bar-versus-gap without color.
    let baseline = theme().chart_baseline();
    let tick = theme().glyphs().chart_baseline.to_string();
    let rule = theme().glyphs().chart_rule.to_string();
    let mut base = vec![Span::styled(tick.repeat(edge), baseline)];
    for i in 0..n {
        if i > 0 {
            base.push(Span::styled(tick.repeat(gap), baseline));
        }
        base.push(Span::styled(rule.repeat(col_w(i)), baseline));
    }
    base.push(Span::styled(tick.repeat(edge), baseline));
    frame.render_widget(
        Paragraph::new(Line::from(base)),
        Rect {
            y: baseline_row,
            height: 1,
            ..area
        },
    );

    // Dim labels centred in each column, truncated to what the column holds.
    let mut label_spans = vec![Span::raw(" ".repeat(edge))];
    for i in 0..n {
        if i > 0 {
            label_spans.push(Span::raw(" ".repeat(gap)));
        }
        let label = labels.get(i).copied().unwrap_or("");
        label_spans.push(Span::styled(
            center_truncate(label, col_w(i)),
            theme().chart_label(),
        ));
    }
    label_spans.push(Span::raw(" ".repeat(edge)));
    frame.render_widget(
        Paragraph::new(Line::from(label_spans)),
        Rect {
            y: label_row,
            height: 1,
            ..area
        },
    );
}

/// Fit `text` into `width` cells, centred with space padding; when it is too long,
/// keep the leading `width` chars with no ellipsis (an initial for a single cell,
/// `Mon`/`2024` when the column is wider).
fn center_truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let trimmed: String = text.chars().take(width).collect();
    let pad = width - text_width(&trimmed);
    let left = pad / 2;
    format!("{}{}{}", " ".repeat(left), trimmed, " ".repeat(pad - left))
}

/// A vertical bar chart of `values` drawn with the block ramp, scaled to the
/// tallest bucket. One cell per column plus a space when it fits; degrades to a
/// one-row sparkline when the area is a single line high.
pub(crate) fn draw_histogram(frame: &mut Frame<'_>, area: Rect, values: &[usize]) {
    if area.width == 0 || area.height == 0 || values.is_empty() {
        return;
    }
    let n = values.len();
    let gap = usize::from((2 * n).saturating_sub(1) <= area.width as usize);
    let max = (*values.iter().max().unwrap_or(&0)).max(1);
    let bar_h = area.height as usize;
    let eighths_per = bar_h * 8;

    let ramp = theme().glyphs().ramps.up;
    let mut lines: Vec<Line> = Vec::with_capacity(bar_h);
    for row in 0..bar_h {
        // Row 0 is the top; `level` counts cells up from the baseline.
        let level = bar_h - row;
        let lower = (level - 1) * 8;
        let mut spans: Vec<Span> = Vec::with_capacity(n * 2);
        for (i, &value) in values.iter().enumerate() {
            if i > 0 && gap == 1 {
                spans.push(Span::raw(" "));
            }
            let filled = (value as f32 / max as f32 * eighths_per as f32).round() as usize;
            let cell = filled.saturating_sub(lower).min(8);
            spans.push(Span::styled(
                ramp_cell(&ramp, cell).to_string(),
                theme().chart_bar().style,
            ));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// A proportion bar split into positive / neutral / negative segments by their
/// share, over `width` cells. Segment length carries the proportion (so it reads
/// on monochrome); colour and weight distinguish the three.
pub(crate) fn sentiment_segments(
    positive: usize,
    neutral: usize,
    negative: usize,
    width: usize,
) -> Line<'static> {
    let total = positive + neutral + negative;
    if total == 0 || width == 0 {
        let track = theme().chart_track();
        return Line::from(Span::styled(
            track.glyph.to_string().repeat(width),
            track.style,
        ));
    }
    let cells = |count: usize| ((count as f32 / total as f32) * width as f32).round() as usize;
    let mut pos = cells(positive);
    let mut neg = cells(negative);
    // Give any rounding remainder to the neutral middle so the bar fills exactly.
    let neu = width.saturating_sub(pos + neg);
    if pos + neu + neg > width {
        // Trim the larger of the coloured ends if rounding overshot.
        if pos >= neg {
            pos = pos.saturating_sub(pos + neu + neg - width);
        } else {
            neg = neg.saturating_sub(pos + neu + neg - width);
        }
    }
    // Each sentiment renders with its own fill: color themes vary the hue,
    // eclipse varies the glyph (█▒░) so the series stay apart without color.
    let segment = |fill: crate::tui::theme::Fill, cells: usize| {
        Span::styled(fill.glyph.to_string().repeat(cells), fill.style)
    };
    Line::from(vec![
        segment(theme().chart_positive(), pos),
        segment(theme().chart_neutral(), neu),
        segment(theme().chart_negative(), neg),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn rect(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    /// Render `draw_signed_columns` and return its rows as strings.
    fn signed_rows(vals: &[Option<f32>], labels: &[&str], width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_signed_columns(frame, rect(width, height), vals, labels))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    #[test]
    fn signed_columns_grow_up_for_positive_down_for_negative() {
        // Six 8-wide columns; the baseline sits at row 3, labels on the last row.
        let vals = [
            Some(-4.0),
            Some(4.0),
            Some(0.0),
            Some(-1.0),
            None,
            Some(2.5),
        ];
        // Width 41 → six 6-wide bars flush to the left with a 1-cell gap between
        // each (leading/trailing padding is half the gap, i.e. 0), so column `col`'s
        // first bar cell sits at `7*col`.
        let rows: Vec<Vec<char>> = signed_rows(&vals, &["A", "B", "C", "D", "E", "F"], 41, 9)
            .iter()
            .map(|row| row.chars().collect())
            .collect();
        let cell = |row: usize, col: usize| rows[row][7 * col];

        // The positive column B fills upward above the baseline (rows 0..3)...
        assert_eq!(cell(2, 1), '█', "B should fill up nearest the baseline");
        // ...and the strongest negative A fills downward below it (rows 4..8).
        assert_eq!(cell(4, 0), '█', "A should fill down from the baseline");
        // A zero value (C) and a `None` (E) leave their columns blank on both halves.
        for row in [0, 1, 2, 4, 5, 6, 7] {
            assert_eq!(cell(row, 2), ' ', "C stays blank");
            assert_eq!(cell(row, 4), ' ', "E stays blank");
        }
        // The baseline rule and centred labels land on their own rows.
        assert_eq!(cell(3, 0), '─', "baseline rule");
        let labels: String = rows[8].iter().collect();
        assert!(
            labels.contains('A') && labels.contains('F'),
            "column labels: {labels}"
        );
    }

    #[test]
    fn signed_columns_auto_scale_to_the_largest_magnitude() {
        // All three below zero; auto-scaling to the deepest (-4) keeps the milder
        // days visibly shorter, so "which is least bad" still reads.
        let vals = [Some(-1.0), Some(-2.0), Some(-4.0)];
        // Width 23 → three 7-wide bars flush left with 1-cell gaps; column `col` at `8*col`.
        let rows: Vec<Vec<char>> = signed_rows(&vals, &["A", "B", "C"], 23, 9)
            .iter()
            .map(|row| row.chars().collect())
            .collect();
        let cell = |row: usize, col: usize| rows[row][8 * col];

        // Row 4 is the cell just below the baseline: every column reaches it.
        assert_eq!((cell(4, 0), cell(4, 1), cell(4, 2)), ('█', '█', '█'));
        // Only the extreme (-4) reaches the deepest row; the milder days stop short.
        assert_eq!(cell(7, 2), '█', "-4 fills its whole half");
        assert_eq!(cell(7, 0), ' ', "-1 stays shallow");
        assert_eq!(cell(7, 1), ' ', "-2 stays mid-depth");
        assert_eq!(cell(5, 0), ' ', "-1 is a single cell deep");
        assert_eq!(cell(5, 1), '█', "-2 is deeper than -1");
    }

    #[test]
    fn columns_scale_with_width() {
        assert_eq!(columns_for(rect(40, 20)), 1);
        assert_eq!(columns_for(rect(60, 20)), 2);
        assert_eq!(columns_for(rect(100, 20)), 3);
    }

    #[test]
    fn short_area_is_flagged() {
        assert!(is_short(rect(80, 6)));
        assert!(!is_short(rect(80, 20)));
    }

    #[test]
    fn stack_drops_trailing_sections_that_do_not_fit_and_fills_height() {
        // Only the first two of three min-4 sections fit in 9 rows; the filling
        // section absorbs the leftover so the area is fully used.
        let slots = stack(
            rect(20, 9),
            &[Section::new(4, 1), Section::new(4, 1), Section::new(4, 0)],
        );
        assert!(slots[0].is_some());
        assert!(slots[1].is_some());
        assert!(slots[2].is_none());
        let total: u16 = slots.iter().flatten().map(|rect| rect.height).sum();
        assert_eq!(total, 9);
    }

    #[test]
    fn sentiment_segments_fill_exactly_and_split_by_share() {
        let line = sentiment_segments(3, 0, 1, 8);
        let width: usize = line
            .spans
            .iter()
            .map(|span| span.content.chars().count())
            .sum();
        assert_eq!(width, 8);
        // 3:0:1 over 8 cells → 6 positive, 0 neutral, 2 negative.
        assert_eq!(line.spans[0].content.chars().count(), 6);
        assert_eq!(line.spans[2].content.chars().count(), 2);
    }

    #[test]
    fn sentiment_segments_stay_distinguishable_without_color_on_eclipse() {
        // The eclipse theme separates the three series by glyph, not hue: each
        // rendered segment must use a different fill character.
        crate::tui::theme::set_test_theme(crate::tui::theme::test_eclipse_theme());
        let line = sentiment_segments(2, 2, 2, 9);
        let glyphs: Vec<char> = line
            .spans
            .iter()
            .filter_map(|span| span.content.chars().next())
            .collect();
        assert_eq!(glyphs.len(), 3);
        assert!(
            glyphs[0] != glyphs[1] && glyphs[1] != glyphs[2] && glyphs[0] != glyphs[2],
            "eclipse sentiment glyphs not pairwise distinct: {glyphs:?}"
        );
    }
}
