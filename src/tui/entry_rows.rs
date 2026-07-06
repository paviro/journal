use journal_storage::{Entry, SearchHit, entry_group_date, parse_entry_timestamp};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    app::{App, Mode},
    scroll::clamp_scroll,
};

/// Display width of `s` in terminal cells (wide/CJK characters count as 2).
fn text_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// The longest prefix of `s` that fits within `max` display cells, and its width.
fn take_width(s: &str, max: usize) -> (String, usize) {
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cell = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cell > max {
            break;
        }
        out.push(ch);
        used += cell;
    }
    (out, used)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryRowMeta {
    pub(crate) entry_index: Option<usize>,
    pub(crate) height: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryListRow {
    pub(crate) entry_index: Option<usize>,
    lines: Vec<Line<'static>>,
}

impl EntryListRow {
    fn height(&self) -> u16 {
        self.lines.len().min(u16::MAX as usize) as u16
    }
}

/// The fully-built entry list for one `(data version, mode, journal, text_width)`
/// combination. [`App`](super::app::App) memoizes this so a frame that only
/// scrolled or moved the selection reuses it instead of rebuilding every row
/// (see `App::entry_rows`). Rows are independent of the scroll offset and the
/// selected index — both are applied downstream in [`visible_entry_items`].
pub(crate) struct EntryRowCache {
    pub(crate) rows: Vec<EntryListRow>,
    pub(crate) meta: Vec<EntryRowMeta>,
    /// Row offset → month label, for the sticky section header. Empty outside
    /// browse mode.
    pub(crate) month_sections: Vec<(usize, String)>,
    pub(crate) total_height: usize,
}

/// Build the entry list once. Runs only on a cache miss (data/journal/width
/// change), so its O(entries) cost is paid at most once per such change rather
/// than several times per frame.
pub(crate) fn build_entry_row_cache(app: &App, text_width: u16) -> EntryRowCache {
    let rows = entry_list_rows(app, text_width);
    let meta: Vec<EntryRowMeta> = rows
        .iter()
        .map(|row| EntryRowMeta {
            entry_index: row.entry_index,
            height: row.height(),
        })
        .collect();
    let total_height = meta.iter().map(|row| row.height as usize).sum();
    let month_sections = entry_month_sections(app, text_width);
    EntryRowCache {
        rows,
        meta,
        month_sections,
        total_height,
    }
}

pub(crate) fn entry_list_rows(app: &App, text_width: u16) -> Vec<EntryListRow> {
    match app.nav.mode {
        Mode::Search => {
            let mut rows = Vec::new();
            for (index, hit) in app.search.hits.iter().enumerate() {
                if index > 0 {
                    rows.push(spacer_row());
                }
                rows.push(EntryListRow {
                    entry_index: Some(index),
                    lines: search_hit_lines(hit, text_width),
                });
            }
            rows
        }
        Mode::Browse => browse_entry_rows(app, text_width),
    }
}

/// A one-line blank gap between entries.
fn spacer_row() -> EntryListRow {
    EntryListRow {
        entry_index: None,
        lines: vec![Line::from(String::new())],
    }
}

/// Search hits reuse the browse box design. Without month/day headers they carry
/// the full date (including year) on the top border and the journal on the bottom.
fn search_hit_lines(hit: &SearchHit, text_width: u16) -> Vec<Line<'static>> {
    let (date, time) = match hit.created_at.as_deref().and_then(parse_entry_timestamp) {
        Some(timestamp) => (
            Some(timestamp.format("%a %d %b %Y").to_string()),
            timestamp.format("%H:%M").to_string(),
        ),
        None => (None, String::new()),
    };
    entry_box_lines(
        date.as_deref(),
        &time,
        &hit.preview,
        Some(&hit.journal),
        text_width,
    )
}

fn browse_entry_rows(app: &App, text_width: u16) -> Vec<EntryListRow> {
    let box_width = text_width as usize + 4;
    let mut rows = Vec::new();
    let mut current_month = None;
    let mut current_day = None;
    let mut prev_was_entry = false;
    let mut is_first_month = true;

    for (index, entry) in app.selected_entries().iter().enumerate() {
        let month = entry_month_label(entry);
        if month != current_month {
            current_month = month.clone();
            current_day = None;
            if let Some(month) = month {
                // The first month rides the panel border from the start, so its
                // divider is replaced by a leading blank line (matching the
                // journals column). Later months keep their in-list divider,
                // padded by a blank line above and below, and take over the
                // border once it scrolls above the top.
                if is_first_month {
                    rows.push(spacer_row());
                } else {
                    rows.push(spacer_row());
                    rows.push(EntryListRow {
                        entry_index: None,
                        lines: vec![month_divider(box_width, &month)],
                    });
                    rows.push(spacer_row());
                }
                is_first_month = false;
                prev_was_entry = false;
            }
        }

        // One blank line between consecutive entries (not after a month divider).
        if prev_was_entry {
            rows.push(spacer_row());
        }

        // The first entry of a day carries the weekday on its border.
        let day = entry_day_label(entry);
        let day_label = if day != current_day {
            current_day = day.clone();
            day
        } else {
            None
        };

        rows.push(EntryListRow {
            entry_index: Some(index),
            lines: entry_list_lines(entry, day_label.as_deref(), text_width),
        });
        prev_was_entry = true;
    }

    rows
}

/// Month sections in the browse list's pixel space: the row offset of each
/// month's divider (or, for the first month, its leading blank line), paired
/// with its label. Mirrors the row sequencing in [`browse_entry_rows`] so the
/// sticky border label switches over in step with the scrolled list. Empty
/// outside browse mode.
pub(crate) fn entry_month_sections(app: &App, text_width: u16) -> Vec<(usize, String)> {
    if app.nav.mode != Mode::Browse {
        return Vec::new();
    }

    let mut sections = Vec::new();
    let mut current_month = None;
    let mut current_day = None;
    let mut prev_was_entry = false;
    let mut is_first_month = true;
    let mut y = 0usize;

    for entry in app.selected_entries().iter() {
        let month = entry_month_label(entry);
        if month != current_month {
            current_month = month.clone();
            current_day = None;
            if let Some(month) = month {
                if is_first_month {
                    sections.push((y, month)); // leading blank line
                    y += 1;
                } else {
                    y += 1; // blank line above the divider
                    sections.push((y, month)); // the divider row
                    y += 2; // divider + blank line below
                }
                is_first_month = false;
                prev_was_entry = false;
            }
        }

        if prev_was_entry {
            y += 1; // blank spacer between consecutive entries
        }

        let day = entry_day_label(entry);
        let day_label = if day != current_day {
            current_day = day.clone();
            day
        } else {
            None
        };

        y += entry_list_lines(entry, day_label.as_deref(), text_width).len();
        prev_was_entry = true;
    }

    sections
}

/// A month separator with the label pinned to the right edge over a heavy rule:
/// `━━━━━━━━━━━━━━━━━━━━━ July 2026`.
fn month_divider(box_width: usize, month: &str) -> Line<'static> {
    let fill = box_width.saturating_sub(text_width(month) + 1);
    Line::from(vec![
        Span::styled(format!("{} ", "━".repeat(fill)), border_style()),
        Span::styled(
            month.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])
}

#[cfg(test)]
pub(crate) fn entry_row_metadata(app: &App, text_width: u16) -> Vec<EntryRowMeta> {
    entry_list_rows(app, text_width)
        .into_iter()
        .map(|row| EntryRowMeta {
            entry_index: row.entry_index,
            height: row.height(),
        })
        .collect()
}

/// Returns visible `ListItem`s and the 0-based index of the selected entry
/// within those items (`None` if not visible or `!selection_visible`).
pub(crate) fn visible_entry_items(
    rows: &[EntryListRow],
    scroll: usize,
    viewport_height: u16,
    selected_entry_index: Option<usize>,
    selection_visible: bool,
) -> (Vec<ListItem<'static>>, Option<usize>) {
    let mut remaining_skip = scroll;
    let mut remaining_height = viewport_height;
    let mut items = Vec::new();
    let mut selected_visible_idx: Option<usize> = None;

    for row in rows {
        if remaining_height == 0 {
            break;
        }

        let height = row.height() as usize;
        if remaining_skip >= height {
            remaining_skip -= height;
            continue;
        }

        let start = remaining_skip;
        remaining_skip = 0;
        let visible_height = (height.saturating_sub(start)).min(remaining_height as usize) as u16;
        let end = start + visible_height as usize;
        let lines = row.lines[start..end].to_vec();
        remaining_height = remaining_height.saturating_sub(visible_height);

        if selection_visible
            && selected_visible_idx.is_none()
            && row.entry_index == selected_entry_index
        {
            selected_visible_idx = Some(items.len());
        }
        items.push(ListItem::new(lines));
    }

    (items, selected_visible_idx)
}

pub(crate) fn entry_month_label(entry: &Entry) -> Option<String> {
    entry_group_date(entry).map(|date| date.format("%B %Y").to_string())
}

pub(crate) fn entry_day_label(entry: &Entry) -> Option<String> {
    entry_group_date(entry).map(|date| date.format("%A %d").to_string())
}

/// Max preview lines shown inside an entry's box.
const ENTRY_BOX_PREVIEW_LINES: usize = 3;

/// Renders one browse entry as a bordered box: the day (on the first entry of a
/// day) and time sit on the top border, the word count on the bottom border, and
/// the preview flows inside.
pub(crate) fn entry_list_lines(
    entry: &Entry,
    day: Option<&str>,
    text_width: u16,
) -> Vec<Line<'static>> {
    let time = entry
        .created_time()
        .map(|timestamp| timestamp.format("%H:%M").to_string())
        .unwrap_or_default();

    entry_box_lines(
        day,
        &time,
        &entry.preview,
        Some(&word_count_label(entry.word_count)),
        text_width,
    )
}

fn word_count_label(count: usize) -> String {
    match count {
        1 => "1 word".to_string(),
        count => format!("{count} words"),
    }
}

/// The shared box shape: a `date … time` top border (date left, time right) over
/// wrapped preview lines, closed by a bottom border that may carry a footer label
/// on its left (used to show the journal for search hits).
pub(crate) fn entry_box_lines(
    date_label: Option<&str>,
    time: &str,
    preview: &str,
    footer_label: Option<&str>,
    text_width: u16,
) -> Vec<Line<'static>> {
    let inner_width = text_width as usize;
    if inner_width == 0 {
        return vec![Line::from(String::new())];
    }
    let box_width = inner_width + 4;

    let time = (!time.is_empty()).then_some(time);
    let mut lines = vec![border_line('┌', '┐', box_width, date_label, time)];
    for text in wrap_text(preview, inner_width, ENTRY_BOX_PREVIEW_LINES) {
        lines.push(box_inner_line(text, inner_width));
    }
    lines.push(border_line('└', '┘', box_width, footer_label, None));
    lines
}

fn border_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// A box border with optional bold labels on the left and right, separated by a
/// dim rule: `┌ Sunday 05 ──────── 14:30 ┐`.
pub(crate) fn border_line(
    open: char,
    close: char,
    box_width: usize,
    left: Option<&str>,
    right: Option<&str>,
) -> Line<'static> {
    let border = border_style();
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let left = left.filter(|label| !label.is_empty());
    let right = right.filter(|label| !label.is_empty());
    let left_width = left.map_or(0, |label| text_width(label) + 2);
    let right_width = right.map_or(0, |label| text_width(label) + 2);

    if box_width < left_width + right_width + 2 {
        return Line::from(Span::styled(
            format!("{open}{}{close}", "─".repeat(box_width.saturating_sub(2))),
            border,
        ));
    }

    let dashes = box_width - 2 - left_width - right_width;
    let mut spans = vec![Span::styled(open.to_string(), border)];
    if let Some(left) = left {
        spans.push(Span::styled(" ".to_string(), border));
        spans.push(Span::styled(left.to_string(), bold));
        spans.push(Span::styled(" ".to_string(), border));
    }
    spans.push(Span::styled("─".repeat(dashes), border));
    if let Some(right) = right {
        spans.push(Span::styled(" ".to_string(), border));
        spans.push(Span::styled(right.to_string(), bold));
        spans.push(Span::styled(" ".to_string(), border));
    }
    spans.push(Span::styled(close.to_string(), border));
    Line::from(spans)
}

pub(crate) fn box_inner_line(text: String, inner_width: usize) -> Line<'static> {
    let (content, used) = take_width(&text, inner_width);
    let pad = inner_width - used;
    Line::from(vec![
        Span::styled("│ ".to_string(), border_style()),
        Span::raw(content),
        Span::styled(format!("{} │", " ".repeat(pad)), border_style()),
    ])
}

/// Greedy word-wrap by display width into at most `max_lines`, ellipsizing the
/// last line when the text overflows.
pub(crate) fn wrap_text(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() || width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() && lines.len() < max_lines {
        if text_width(rest) <= width {
            lines.push(rest.to_string());
            break;
        }
        if lines.len() + 1 == max_lines {
            lines.push(truncate_ellipsis(rest, width));
            break;
        }

        let head = take_width(rest, width).0;
        let (line_end, next_start) = if head.is_empty() {
            // The first character is wider than the whole line; consume it so
            // we always make progress and never split mid-character.
            let end = rest.chars().next().map_or(rest.len(), char::len_utf8);
            (end, end)
        } else {
            match head.rfind(' ') {
                Some(space) => (space, space + 1),
                None => (head.len(), head.len()),
            }
        };
        lines.push(rest[..line_end].to_string());
        rest = &rest[next_start..];
    }
    lines
}

/// Truncate `text` to `max` display cells, ending with `…` when it overflows.
fn truncate_ellipsis(text: &str, max: usize) -> String {
    if text_width(text) <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let (mut truncated, _) = take_width(text, max - 1);
    while truncated.ends_with(' ') {
        truncated.pop();
    }
    truncated.push('…');
    truncated
}

pub(crate) fn ensure_entry_visible(
    scroll: &mut usize,
    rows: &[EntryRowMeta],
    selected_entry_index: Option<usize>,
    viewport_height: u16,
) {
    let Some((row_start, row_height)) = selected_entry_row_span(rows, selected_entry_index) else {
        *scroll = clamp_scroll(*scroll, total_entry_row_height(rows), viewport_height);
        return;
    };

    if viewport_height == 0 {
        *scroll = clamp_scroll(*scroll, total_entry_row_height(rows), viewport_height);
        return;
    }

    if row_start < *scroll {
        *scroll = row_start;
    } else {
        let row_end = row_start.saturating_add(row_height as usize);
        let viewport_end = scroll.saturating_add(viewport_height as usize);
        if row_end > viewport_end {
            *scroll = row_end.saturating_sub(viewport_height as usize);
        }
    }
    *scroll = clamp_scroll(*scroll, total_entry_row_height(rows), viewport_height);
}

pub(crate) fn selected_entry_row_span(
    rows: &[EntryRowMeta],
    selected_entry_index: Option<usize>,
) -> Option<(usize, u16)> {
    selected_entry_index?;
    let mut y = 0usize;
    for row in rows {
        if row.entry_index == selected_entry_index {
            return Some((y, row.height));
        }
        y = y.saturating_add(row.height as usize);
    }
    None
}

pub(crate) fn total_entry_row_height(rows: &[EntryRowMeta]) -> usize {
    rows.iter().map(|row| row.height as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_entry_visible_scrolls_past_u16_max_for_tall_lists() {
        // 100 boxes of 1000 rows each → 100_000 px total, far beyond u16::MAX
        // (65535). Selecting the last one must scroll to the very bottom rather
        // than clamping short — the "can't scroll to the end" regression.
        let rows: Vec<EntryRowMeta> = (0..100)
            .map(|index| EntryRowMeta {
                entry_index: Some(index),
                height: 1000,
            })
            .collect();

        let mut scroll = 0usize;
        ensure_entry_visible(&mut scroll, &rows, Some(99), 20);

        assert_eq!(scroll, 100_000 - 20);
        assert!(scroll > u16::MAX as usize);
    }

    #[test]
    fn wrapping_and_padding_use_display_width_not_char_count() {
        // Each CJK character is two cells wide. A width of 4 fits exactly two of
        // them, so a four-character run must wrap onto a second line — a
        // char-count wrap would wrongly keep all four on one 8-cell line.
        let wrapped = wrap_text("日本語訳", 4, 3);
        assert_eq!(wrapped, vec!["日本".to_string(), "語訳".to_string()]);

        // The padded inner content must span exactly `inner_width` cells: two
        // wide characters (4 cells) leave a single trailing space to reach 5.
        let line = box_inner_line("日本".to_string(), 5);
        assert_eq!(line.spans[1].content.as_ref(), "日本");
        assert_eq!(line.spans[2].content.as_ref(), "  │");
    }
}
