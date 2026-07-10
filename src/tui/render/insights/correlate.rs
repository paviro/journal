//! Shared rendering for the People / Activities / Tags tabs: a ranked list of
//! correlated values with their mood association. On a wide panel this is a
//! bordered ASCII table — a header rule plus a diverging "lifts / drains" bar that
//! fills the free width; on a narrow side column it degrades to compact one-line
//! rows (also reused by the Feelings tab's "Mood by feeling" section). Both paths
//! scroll: `scroll` is the first visible row, clamped here to the list length.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use journal_analytics::Correlate;

use super::signed;
use crate::tui::entry_rows::truncate_ellipsis;
use crate::tui::render::render_centered_notice;
use crate::tui::render::table::{self, border, pad, push_cell};
use crate::tui::theme::theme;

/// Below this panel width the table's borders crowd the columns, so the compact
/// one-line rows are used instead.
const TABLE_MIN_WIDTH: u16 = 70;

/// Fixed inner content widths for the numeric columns (headroom for `Count`,
/// `+3.0`, and a `-10.0`-style delta).
const COUNT_W: usize = 5;
const AVG_W: usize = 5;
const DELTA_W: usize = 5;

/// What the panel needs to draw a scrollbar and map a drag: the list length and
/// how many rows are visible at once.
pub(super) struct InsightsListMetrics {
    pub(super) total: usize,
    pub(super) viewport: usize,
}

/// Draw a ranked list of correlates (frequency order) starting at row `*scroll`,
/// or an empty notice. Clamps `*scroll` to the list length and returns the scroll
/// metrics for the panel's scrollbar.
pub(super) fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    items: &[Correlate],
    empty_msg: &str,
    feeling_header: &str,
    scroll: &mut u16,
) -> InsightsListMetrics {
    if items.is_empty() {
        *scroll = 0;
        render_centered_notice(frame, area, empty_msg);
        return InsightsListMetrics {
            total: 0,
            viewport: 0,
        };
    }
    let columns = (area.width >= TABLE_MIN_WIDTH)
        .then(|| Columns::fit(area.width as usize))
        .flatten();
    match columns {
        Some(columns) if area.height >= 5 => {
            draw_table(frame, area, items, scroll, &columns, feeling_header)
        }
        _ => draw_compact(frame, area, items, scroll),
    }
}

/// Clamp `*scroll` so the last row can still reach the bottom of a `viewport`-row
/// window, and return the visible slice's row range.
fn visible_range(total: usize, viewport: usize, scroll: &mut u16) -> std::ops::Range<usize> {
    let max_scroll = total.saturating_sub(viewport);
    let start = (*scroll as usize).min(max_scroll);
    *scroll = start as u16;
    start..(start + viewport).min(total)
}

/// The narrow side-column layout: one line per correlate, no header or borders.
fn draw_compact(
    frame: &mut Frame<'_>,
    area: Rect,
    items: &[Correlate],
    scroll: &mut u16,
) -> InsightsListMetrics {
    let viewport = area.height as usize;
    let range = visible_range(items.len(), viewport, scroll);
    let name_w = (area.width as usize / 4).clamp(8, 20);
    let lines: Vec<Line> = items[range]
        .iter()
        .map(|correlate| correlate_line(correlate, name_w, true))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    InsightsListMetrics {
        total: items.len(),
        viewport,
    }
}

/// The wide layout: a bordered ASCII table with a full grid — a rule between every
/// row — plus a faint zebra stripe on alternate rows. The header and top border
/// take two rows and each data row carries a trailing rule, so a `height`-row
/// panel shows `(height - 3) / 2` scrollable rows.
fn draw_table(
    frame: &mut Frame<'_>,
    area: Rect,
    items: &[Correlate],
    scroll: &mut u16,
    columns: &Columns,
    feeling_header: &str,
) -> InsightsListMetrics {
    let viewport = (area.height.saturating_sub(3) / 2) as usize;
    let range = visible_range(items.len(), viewport, scroll);

    let max_abs = items
        .iter()
        .filter_map(|correlate| correlate.mood_delta)
        .map(f32::abs)
        .fold(0.0_f32, f32::max);

    let border = theme().muted();
    let mut lines: Vec<Line> = Vec::with_capacity(2 * viewport + 3);
    lines.push(columns.rule('┌', '┬', '┐', border, border));
    lines.push(columns.header_row(feeling_header));
    // The header/body divider keeps the border weight; the grid lines *between*
    // data rows fade their dashes and run the column borders straight through as
    // plain `│` (no `┼` junctions), so the vertical lines stay continuous and
    // uniform even though the faint dashes no longer connect to them.
    lines.push(columns.rule('├', '┼', '┤', border, border));
    for (row, correlate) in items[range.clone()].iter().enumerate() {
        if row > 0 {
            lines.push(columns.rule('│', '│', '│', border, theme().faint_rule()));
        }
        lines.push(columns.data_row(correlate, max_abs));
    }
    lines.push(columns.rule('└', '┴', '┘', border, border));

    frame.render_widget(Paragraph::new(lines), area);
    InsightsListMetrics {
        total: items.len(),
        viewport,
    }
}

/// Resolved inner content widths for each table column. The name, bar, and feeling
/// columns absorb the panel's free width; the bar is dropped when there is no room
/// for a legible one.
struct Columns {
    name: usize,
    bar: Option<usize>,
    feeling: usize,
}

impl Columns {
    /// The mins each flexible column needs before it earns its place.
    const NAME_MIN: usize = 10;
    const FEELING_MIN: usize = 8;
    const BAR_MIN: usize = 13;

    /// Fit the columns to `panel_width`, preferring a bar column and falling back
    /// to a bar-less table; `None` when even that is too cramped to border.
    fn fit(panel_width: usize) -> Option<Self> {
        for with_bar in [true, false] {
            let cols = if with_bar { 6 } else { 5 };
            // A row is `│ c0 │ c1 … │`: `cols + 1` verticals plus two pad cells per
            // column, so the content widths share `width - (3 * cols + 1)`.
            let inner = panel_width.checked_sub(3 * cols + 1)?;
            let fixed = COUNT_W + AVG_W + DELTA_W;
            let Some(flex) = inner.checked_sub(fixed) else {
                continue;
            };
            let need =
                Self::NAME_MIN + Self::FEELING_MIN + if with_bar { Self::BAR_MIN } else { 0 };
            if flex < need {
                continue;
            }
            let extra = flex - need;
            return Some(if with_bar {
                // Name and feeling grow at weight 3, the bar at weight 4; the bar
                // takes the rounding remainder.
                let name = Self::NAME_MIN + extra * 3 / 10;
                let feeling = Self::FEELING_MIN + extra * 3 / 10;
                let bar = flex - name - feeling;
                Self {
                    name,
                    bar: Some(bar),
                    feeling,
                }
            } else {
                let name = Self::NAME_MIN + extra / 2;
                Self {
                    name,
                    bar: None,
                    feeling: flex - name,
                }
            });
        }
        None
    }

    /// The content widths in column order.
    fn widths(&self) -> Vec<usize> {
        let mut widths = vec![self.name, COUNT_W, AVG_W, DELTA_W];
        if let Some(bar) = self.bar {
            widths.push(bar);
        }
        widths.push(self.feeling);
        widths
    }

    /// A horizontal border rule, e.g. `┌────┬────┐`, matching the column widths.
    fn rule(
        &self,
        left: char,
        mid: char,
        right: char,
        junction: Style,
        dash: Style,
    ) -> Line<'static> {
        table::rule(&self.widths(), left, mid, right, junction, dash)
    }

    /// The dim header row, its labels padded to the column widths. `feeling_header`
    /// names the trailing column, which differs by tab (associated vs. co-occurring
    /// feelings).
    fn header_row(&self, feeling_header: &str) -> Line<'static> {
        let mut spans = vec![border()];
        push_cell(
            &mut spans,
            Span::styled(pad("Name", self.name, false), theme().muted()),
        );
        push_cell(
            &mut spans,
            Span::styled(pad("Count", COUNT_W, true), theme().muted()),
        );
        push_cell(
            &mut spans,
            Span::styled(pad("Avg", AVG_W, true), theme().muted()),
        );
        push_cell(
            &mut spans,
            Span::styled(pad("Δ", DELTA_W, true), theme().muted()),
        );
        if let Some(bar) = self.bar {
            // Reads left-to-right to match the diverging bar: drains fill left of
            // the centre marker, lifts fill right.
            push_cell(
                &mut spans,
                Span::styled(pad("Drains / lifts", bar, false), theme().muted()),
            );
        }
        push_cell(
            &mut spans,
            Span::styled(pad(feeling_header, self.feeling, false), theme().muted()),
        );
        Line::from(spans)
    }

    /// One data row: name, count, avg mood, delta, an optional diverging bar, and
    /// the top feeling — each boxed by the `│` column borders.
    fn data_row(&self, correlate: &Correlate, max_abs: f32) -> Line<'static> {
        let mut spans = vec![border()];
        push_cell(
            &mut spans,
            Span::raw(pad(
                &truncate_ellipsis(&correlate.name, self.name),
                self.name,
                false,
            )),
        );
        push_cell(
            &mut spans,
            Span::raw(format!("{:>COUNT_W$}", correlate.count)),
        );
        push_cell(
            &mut spans,
            match correlate.avg_mood {
                Some(avg) => Span::styled(format!("{:>AVG_W$}", signed(avg)), theme().signed(avg)),
                None => Span::styled(format!("{:>AVG_W$}", "—"), theme().muted()),
            },
        );
        push_cell(
            &mut spans,
            match correlate.mood_delta {
                Some(delta) => Span::styled(
                    format!("{:>DELTA_W$}", signed(delta)),
                    theme().signed(delta),
                ),
                None => Span::styled(format!("{:>DELTA_W$}", "—"), theme().muted()),
            },
        );
        if let Some(bar) = self.bar {
            // The bar is many spans, so splice them in between the pad + borders
            // rather than through the single-span `push_cell`.
            spans.push(Span::raw(" "));
            spans.extend(delta_bar(correlate.mood_delta, max_abs, bar).spans);
            spans.push(Span::raw(" "));
            spans.push(border());
        }
        // The most-common feelings with their counts, joined to fill the column and
        // ellipsized when they overflow (a wide panel shows several, a snug one the top).
        push_cell(
            &mut spans,
            Span::styled(
                pad(
                    &truncate_ellipsis(&feelings_label(correlate), self.feeling),
                    self.feeling,
                    false,
                ),
                theme().muted(),
            ),
        );
        Line::from(spans)
    }
}

/// The associated feelings as `calm (5), grateful (3)`, or `—` when there are none.
fn feelings_label(correlate: &Correlate) -> String {
    if correlate.top_feelings.is_empty() {
        return "—".to_string();
    }
    correlate
        .top_feelings
        .iter()
        .map(|(name, count)| format!("{name} ({count})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A diverging bar centred on zero: the fill extends right of the centre marker
/// when this value rides a happier-than-average mood (`positive`) and left when
/// sadder (`negative`), its length proportional to `delta / max_abs`. The dim `·`
/// track and `▓` fill share the panel's bar vocabulary; a `None`/zero delta or a
/// flat journal (`max_abs == 0`) leaves the bare track.
fn delta_bar(delta: Option<f32>, max_abs: f32, width: usize) -> Line<'static> {
    // The centre marker plus two halves span exactly `width`; the right half keeps
    // any odd cell so the box border stays aligned.
    let track = width.saturating_sub(1);
    let left_len = track / 2;
    let right_len = track - left_len;
    let mut left = vec![false; left_len];
    let mut right = vec![false; right_len];

    if let Some(delta) = delta
        && max_abs > 0.0
        && delta != 0.0
    {
        let frac = (delta.abs() / max_abs).clamp(0.0, 1.0);
        if delta > 0.0 {
            let filled = ((frac * right_len as f32).round() as usize).min(right_len);
            right.iter_mut().take(filled).for_each(|cell| *cell = true);
        } else {
            // Fill the negative side from the centre outward (the right end of
            // the left half is nearest the marker).
            let filled = ((frac * left_len as f32).round() as usize).min(left_len);
            left.iter_mut()
                .rev()
                .take(filled)
                .for_each(|cell| *cell = true);
        }
    }

    // Filled cells carry the sign colour; empty ones read as the muted `·`
    // groove, so length alone conveys magnitude on monochrome.
    let mut spans = Vec::with_capacity(width);
    for &on in &left {
        spans.push(cell_span(on, theme().negative()));
    }
    spans.push(Span::styled("│", theme().muted()));
    for &on in &right {
        spans.push(cell_span(on, theme().positive()));
    }
    Line::from(spans)
}

/// One bar cell: a filled `▓` in `style`, or the muted `·` groove when empty.
fn cell_span(filled: bool, style: ratatui::style::Style) -> Span<'static> {
    if filled {
        Span::styled("▓", style)
    } else {
        Span::styled("·", theme().muted())
    }
}

/// One correlate row: `name   count   avg±   Δ±   [feeling]`. The `avg`/`Δ`
/// columns are tinted by sign. Set `show_feeling` off where the top feeling would
/// just be the row itself (the Feelings tab's mood ranking).
pub(super) fn correlate_line(
    correlate: &Correlate,
    name_w: usize,
    show_feeling: bool,
) -> Line<'static> {
    let mut spans = vec![Span::raw(format!(
        "{:<name_w$} {:>3}  ",
        truncate_ellipsis(&correlate.name, name_w),
        correlate.count,
    ))];
    match correlate.avg_mood {
        Some(avg) => spans.push(Span::styled(
            format!("{:>5}", signed(avg)),
            theme().signed(avg),
        )),
        None => spans.push(Span::styled("    —", theme().muted())),
    }
    match correlate.mood_delta {
        Some(delta) => spans.push(Span::styled(
            format!("  Δ{:>5}", signed(delta)),
            theme().signed(delta),
        )),
        None => spans.push(Span::styled("       ", theme().muted())),
    }
    if show_feeling && let Some((feeling, count)) = correlate.top_feelings.first() {
        spans.push(Span::styled(
            format!("  {feeling} ({count})"),
            theme().muted(),
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number of cells the bar occupies, and how many are filled on each side
    /// of the centre marker.
    fn bar_shape(line: &Line) -> (usize, usize, usize) {
        let cells: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let width = cells.chars().count();
        let marker = cells.find('│').map(|byte| cells[..byte].chars().count());
        let (left, right) = match marker {
            Some(mid) => (
                cells.chars().take(mid).filter(|c| *c == '▓').count(),
                cells.chars().skip(mid + 1).filter(|c| *c == '▓').count(),
            ),
            None => (0, 0),
        };
        (width, left, right)
    }

    #[test]
    fn delta_bar_has_a_centre_marker_and_fixed_width() {
        let (width, left, right) = bar_shape(&delta_bar(None, 2.0, 11));
        // 11 → 5 groove + marker + 5 groove; a None delta leaves the bare track.
        assert_eq!(width, 11);
        assert_eq!((left, right), (0, 0));
    }

    #[test]
    fn positive_delta_fills_right_and_negative_fills_left() {
        // A full-magnitude delta fills its whole half; the other side stays empty.
        let (_, left, right) = bar_shape(&delta_bar(Some(2.0), 2.0, 11));
        assert_eq!((left, right), (0, 5));

        let (_, left, right) = bar_shape(&delta_bar(Some(-2.0), 2.0, 11));
        assert_eq!((left, right), (5, 0));

        // Half-magnitude fills half the side, proportional to `delta / max_abs`.
        let (_, _, right) = bar_shape(&delta_bar(Some(1.0), 2.0, 11));
        assert_eq!(right, 3); // round(0.5 * 5)
    }

    #[test]
    fn a_table_row_spans_the_full_panel_width() {
        // Every rendered line — rule, header, data — is exactly the panel width, so
        // the borders line up into a solid box. Checked at an even and an odd width
        // because the diverging bar splits its track around a centre marker.
        for panel in [120, 121] {
            let columns = Columns::fit(panel).expect("panel should fit a table");
            let width = |line: Line| -> usize {
                line.spans
                    .iter()
                    .flat_map(|span| span.content.chars())
                    .count()
            };
            let inner: usize = columns.widths().iter().map(|w| w + 3).sum::<usize>() + 1;
            assert_eq!(inner, panel, "columns should tile the panel width");
            assert_eq!(
                width(columns.rule('┌', '┬', '┐', theme().muted(), theme().muted())),
                panel
            );
            assert_eq!(width(columns.header_row("Feelings")), panel);

            let sample = Correlate {
                name: "alex".to_string(),
                count: 3,
                avg_mood: Some(2.0),
                top_feelings: vec![("calm".to_string(), 2)],
                mood_delta: Some(1.5),
            };
            assert_eq!(width(columns.data_row(&sample, 3.0)), panel);
        }
    }

    #[test]
    fn scroll_clamps_to_the_last_page() {
        // 100 rows, a 10-row window: a huge requested offset lands on the last page.
        let mut scroll = u16::MAX;
        let range = visible_range(100, 10, &mut scroll);
        assert_eq!(range, 90..100);
        assert_eq!(scroll, 90);
    }
}
