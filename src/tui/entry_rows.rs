use notema_domain::{Entry, SearchHit, entry_group_date};
use notema_storage::parse_entry_timestamp;
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    app::{App, Mode},
    scroll::clamp_scroll,
    theme::theme,
};

/// Display width of `s` in terminal cells (wide/CJK characters count as 2).
pub(crate) fn text_width(s: &str) -> usize {
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
pub(crate) struct RowMeta {
    pub(crate) item_index: Option<usize>,
    pub(crate) height: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct BoxRow {
    pub(crate) item_index: Option<usize>,
    lines: Vec<Line<'static>>,
}

impl BoxRow {
    /// A row carrying `index` (the entry or journal index it represents, or
    /// `None` for a non-selectable divider/spacer row) and its rendered lines.
    pub(crate) fn new(index: Option<usize>, lines: Vec<Line<'static>>) -> Self {
        Self {
            item_index: index,
            lines,
        }
    }

    fn height(&self) -> u16 {
        self.lines.len().min(u16::MAX as usize) as u16
    }
}

/// The per-row metadata (index + height) for a built row list, used by the
/// scroll/hit-test helpers.
pub(crate) fn rows_meta(rows: &[BoxRow]) -> Vec<RowMeta> {
    rows.iter()
        .map(|row| RowMeta {
            item_index: row.item_index,
            height: row.height(),
        })
        .collect()
}

/// The fully-built entry list for one `(data version, mode, journal, text_width)`
/// combination. [`App`](super::app::App) memoizes this so a frame that only
/// scrolled or moved the selection reuses it instead of rebuilding every row
/// (see `App::entry_rows`). Rows are independent of the scroll offset and the
/// selected index — both are applied downstream in [`visible_box_items`].
pub(crate) struct EntryRowCache {
    pub(crate) rows: Vec<BoxRow>,
    pub(crate) meta: Vec<RowMeta>,
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
    let meta: Vec<RowMeta> = rows
        .iter()
        .map(|row| RowMeta {
            item_index: row.item_index,
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

pub(crate) fn entry_list_rows(app: &App, text_width: u16) -> Vec<BoxRow> {
    match app.nav.mode {
        Mode::Search => {
            let mut rows = Vec::new();
            for (index, hit) in app.search.hits.iter().enumerate() {
                if index > 0 {
                    rows.push(spacer_row());
                }
                rows.push(BoxRow {
                    item_index: Some(index),
                    lines: search_hit_lines(hit, text_width),
                });
            }
            rows
        }
        Mode::Browse => browse_entry_rows(app, text_width),
    }
}

/// A one-line blank gap between entries.
fn spacer_row() -> BoxRow {
    BoxRow {
        item_index: None,
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
    // Archived journals still show up in search; flag them on the bottom-right and
    // show the plain (un-suffixed) journal name on the bottom-left.
    let archived = notema_storage::is_archived_name(&hit.journal);
    entry_box_lines(
        date.as_deref(),
        &time,
        &hit.preview,
        Some(&footer_left_label(
            notema_storage::journal_display_name(&hit.journal).to_string(),
            hit.starred,
        )),
        archived.then_some("Archived"),
        text_width,
    )
}

fn browse_entry_rows(app: &App, text_width: u16) -> Vec<BoxRow> {
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
                    rows.push(BoxRow {
                        item_index: None,
                        lines: vec![section_divider(box_width, &month, DividerAlign::Right)],
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

        rows.push(BoxRow {
            item_index: Some(index),
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

/// A section separator with the label pinned to the right edge over a heavy rule:
/// `━━━━━━━━━━━━━━━━━━━━━ July 2026`. Shared by the entry list's month headers and
/// the journal column's "Archived" divider.
/// Which edge a [`section_divider`] label is pinned to. The entry list's month
/// headers read best right-aligned; the journal column's "Archived" divider left.
pub(crate) enum DividerAlign {
    Left,
    Right,
}

pub(crate) fn section_divider(box_width: usize, label: &str, align: DividerAlign) -> Line<'static> {
    let fill = theme()
        .glyphs()
        .divider
        .to_string()
        .repeat(box_width.saturating_sub(text_width(label) + 1));
    let label = Span::styled(label.to_string(), theme().heading());
    let rule = theme().divider();
    let spans = match align {
        DividerAlign::Left => vec![label, Span::styled(format!(" {fill}"), rule)],
        DividerAlign::Right => vec![Span::styled(format!("{fill} "), rule), label],
    };
    Line::from(spans)
}

/// One journal rendered as a bordered box with its name inside, mirroring the
/// entry list. Journals carry no border labels; the name is truncated to fit.
pub(crate) fn journal_box_lines(name: &str, inner_width: usize) -> Vec<Line<'static>> {
    let box_width = inner_width + 4;
    vec![
        border_line(BoxEdge::Top, box_width, None, None),
        box_inner_line(name.to_string(), inner_width),
        border_line(BoxEdge::Bottom, box_width, None, None),
    ]
}

/// One journal in flat chrome: a background-filled card the height of the
/// bordered box (padding row, name row, padding row) followed by a blank
/// separator row so adjacent cards read as distinct blocks. The padding rows
/// paint explicit spaces — an empty line covers no cells, so it would leave
/// the card background unpainted.
pub(crate) fn journal_card_lines(
    name: &str,
    inner_width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    let box_width = inner_width + 4;
    let (content, used) = take_width(name, inner_width);
    let pad_row = || Line::from(Span::styled(" ".repeat(box_width), style));
    let name_row = Line::from(Span::styled(
        format!(
            "  {content}{}",
            " ".repeat(box_width.saturating_sub(used + 2))
        ),
        style,
    ));
    vec![pad_row(), name_row, pad_row(), Line::from(String::new())]
}

/// The journal column's rows: active journals first, then an "Archived" divider,
/// then the archived journals. Each journal row carries its index into
/// `app.library.journals` (the selection index); the divider row carries `None`
/// so it is never selectable. The divider appears only when there are both active
/// and archived journals.
pub(crate) fn journal_list_rows(app: &App, inner_width: usize) -> Vec<BoxRow> {
    let box_width = inner_width + 4;
    let active = app.active_journal_count();
    let show_divider = active > 0 && active < app.library.journals.len();
    // Flat chrome bakes selection into the chips (the List highlight would
    // also paint the blank padding rows); bordered keeps the List highlight.
    let flat = crate::tui::render::flat_chrome();
    let selected = app.nav.journal_list.selected();
    let select_all = app.nav.mode == Mode::Search
        && app.search.scope == crate::tui::app::SearchScope::AllJournals;

    let mut rows = Vec::new();
    for (index, journal) in app.library.journals.iter().enumerate() {
        if show_divider && index == active {
            let mut lines = vec![
                Line::from(String::new()),
                section_divider(box_width, "Archived", DividerAlign::Left),
                Line::from(String::new()),
            ];
            if flat {
                // Every flat row is one separator row taller than its bordered
                // counterpart, the divider included, so `journal_row_top`'s
                // uniform-height multiply holds in both chromes.
                lines.push(Line::from(String::new()));
            }
            rows.push(BoxRow::new(None, lines));
        }
        let lines = if flat {
            let hovered = matches!(
                app.hover,
                crate::tui::state::HoverTarget::Journal(i) if i == index
            );
            let style = if select_all || selected == Some(index) {
                theme().selection()
            } else if hovered {
                theme().text().patch(theme().hover())
            } else {
                theme().text().bg(theme().element_bg())
            };
            journal_card_lines(journal.display_name(), inner_width, style)
        } else {
            journal_box_lines(journal.display_name(), inner_width)
        };
        rows.push(BoxRow::new(Some(index), lines));
    }
    rows
}

#[cfg(test)]
pub(crate) fn entry_row_metadata(app: &App, text_width: u16) -> Vec<RowMeta> {
    entry_list_rows(app, text_width)
        .into_iter()
        .map(|row| RowMeta {
            item_index: row.item_index,
            height: row.height(),
        })
        .collect()
}

/// Returns the visible `ListItem`s, the 0-based index of the selected item within
/// them (`None` if not visible or `!selection_visible`), and each produced item's
/// row index (`None` for divider/spacer rows). The row-index list lets callers
/// style items by kind — e.g. the journal column highlighting every journal box
/// but not the "Archived" divider.
pub(crate) fn visible_box_items(
    rows: &[BoxRow],
    scroll: usize,
    viewport_height: u16,
    selected_index: Option<usize>,
    selection_visible: bool,
) -> (Vec<ListItem<'static>>, Option<usize>, Vec<Option<usize>>) {
    let mut remaining_skip = scroll;
    let mut remaining_height = viewport_height;
    let mut items = Vec::new();
    let mut indices = Vec::new();
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

        if selection_visible && selected_visible_idx.is_none() && row.item_index == selected_index {
            selected_visible_idx = Some(items.len());
        }
        items.push(ListItem::new(lines));
        indices.push(row.item_index);
    }

    (items, selected_visible_idx, indices)
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
        Some(&footer_left_label(
            word_count_label(entry.word_count),
            entry.starred,
        )),
        None,
        text_width,
    )
}

/// The bottom-left label with a trailing `★` when starred. Keeping the star on
/// the left leaves the bottom-right slot free for the search-result "Archived"
/// flag, and reads consistently across the browse and search box views.
fn footer_left_label(mut label: String, starred: bool) -> String {
    if starred {
        label.push_str(" ★");
    }
    label
}

fn word_count_label(count: usize) -> String {
    match count {
        1 => "1 word".to_string(),
        count => format!("{count} words"),
    }
}

/// The shared box shape: a `date … time` top border (date left, time right) over
/// wrapped preview lines, closed by a bottom border that may carry a footer label
/// on its left (the journal for search hits, or the word count) and one on its
/// right (the `archived` flag for search hits).
pub(crate) fn entry_box_lines(
    date_label: Option<&str>,
    time: &str,
    preview: &str,
    footer_left: Option<&str>,
    footer_right: Option<&str>,
    text_width: u16,
) -> Vec<Line<'static>> {
    let inner_width = text_width as usize;
    if inner_width == 0 {
        return vec![Line::from(String::new())];
    }
    let box_width = inner_width + 4;

    let time = (!time.is_empty()).then_some(time);
    if crate::tui::render::flat_chrome() {
        // Flat card: the border rows become text rows carrying the same
        // labels — day and time in the header, word count / journal flags in
        // the footer, all muted so the metadata frames the preview without
        // competing with it. A blank padding row above and below keeps the
        // labels off the card's edge; the card background comes from the
        // list-item style, which fills blank rows too.
        let mut lines = vec![
            Line::from(String::new()),
            card_edge_line(
                box_width,
                date_label.map(|label| (label, theme().muted())),
                time.map(|label| (label, theme().muted())),
            ),
        ];
        for text in wrap_text(preview, inner_width, ENTRY_BOX_PREVIEW_LINES) {
            lines.push(card_inner_line(text, inner_width));
        }
        lines.push(card_edge_line(
            box_width,
            footer_left.map(|label| (label, theme().muted())),
            footer_right.map(|label| (label, theme().muted())),
        ));
        lines.push(Line::from(String::new()));
        return lines;
    }

    let mut lines = vec![border_line(BoxEdge::Top, box_width, date_label, time)];
    for text in wrap_text(preview, inner_width, ENTRY_BOX_PREVIEW_LINES) {
        lines.push(box_inner_line(text, inner_width));
    }
    lines.push(border_line(
        BoxEdge::Bottom,
        box_width,
        footer_left,
        footer_right,
    ));
    lines
}

/// A flat card's header/footer row: `left` at the card's two-cell inset,
/// `right` pinned to the far edge, each with its own style — no border glyphs.
fn card_edge_line(
    box_width: usize,
    left: Option<(&str, Style)>,
    right: Option<(&str, Style)>,
) -> Line<'static> {
    let left = left.filter(|(label, _)| !label.is_empty());
    let right = right.filter(|(label, _)| !label.is_empty());
    let left_width = left.map_or(0, |(label, _)| text_width(label));
    let right_width = right.map_or(0, |(label, _)| text_width(label));
    if box_width < left_width + right_width + 4 {
        return Line::from(String::new());
    }

    let mut spans = vec![Span::raw("  ")];
    if let Some((label, style)) = left {
        spans.push(Span::styled(label.to_string(), style));
    }
    spans.push(Span::raw(
        " ".repeat(box_width - 4 - left_width - right_width),
    ));
    if let Some((label, style)) = right {
        spans.push(Span::styled(label.to_string(), style));
    }
    spans.push(Span::raw("  "));
    Line::from(spans)
}

/// A flat card's preview row: the bordered box's `│ text │` with the border
/// ink replaced by plain padding, so columns line up across chrome styles.
fn card_inner_line(text: String, inner_width: usize) -> Line<'static> {
    let (content, used) = take_width(&text, inner_width);
    let pad = inner_width - used;
    Line::from(vec![
        Span::raw("  "),
        Span::raw(content),
        Span::raw(format!("{}  ", " ".repeat(pad))),
    ])
}

fn border_style() -> Style {
    theme().muted()
}

/// Which edge of a hand-drawn box a [`border_line`] draws, deciding its corner
/// glyphs from the theme's line set.
#[derive(Clone, Copy)]
pub(crate) enum BoxEdge {
    Top,
    Bottom,
}

/// A box border with optional bold labels on the left and right, separated by a
/// dim rule: `┌ Sunday 05 ──────── 14:30 ┐`, in the theme's line set.
pub(crate) fn border_line(
    edge: BoxEdge,
    box_width: usize,
    left: Option<&str>,
    right: Option<&str>,
) -> Line<'static> {
    let set = theme().glyphs().borders.line_set();
    let (open, close) = match edge {
        BoxEdge::Top => (set.top_left, set.top_right),
        BoxEdge::Bottom => (set.bottom_left, set.bottom_right),
    };
    let border = border_style();
    let bold = theme().heading();
    let left = left.filter(|label| !label.is_empty());
    let right = right.filter(|label| !label.is_empty());
    let left_width = left.map_or(0, |label| text_width(label) + 2);
    let right_width = right.map_or(0, |label| text_width(label) + 2);

    if box_width < left_width + right_width + 2 {
        return Line::from(Span::styled(
            format!(
                "{open}{}{close}",
                set.horizontal.repeat(box_width.saturating_sub(2))
            ),
            border,
        ));
    }

    let dashes = box_width - 2 - left_width - right_width;
    let mut spans = vec![Span::styled(open, border)];
    if let Some(left) = left {
        spans.push(Span::styled(" ".to_string(), border));
        spans.push(Span::styled(left.to_string(), bold));
        spans.push(Span::styled(" ".to_string(), border));
    }
    spans.push(Span::styled(set.horizontal.repeat(dashes), border));
    if let Some(right) = right {
        spans.push(Span::styled(" ".to_string(), border));
        spans.push(Span::styled(right.to_string(), bold));
        spans.push(Span::styled(" ".to_string(), border));
    }
    spans.push(Span::styled(close, border));
    Line::from(spans)
}

pub(crate) fn box_inner_line(text: String, inner_width: usize) -> Line<'static> {
    let vertical = theme().glyphs().borders.line_set().vertical;
    let (content, used) = take_width(&text, inner_width);
    let pad = inner_width - used;
    Line::from(vec![
        Span::styled(format!("{vertical} "), border_style()),
        Span::raw(content),
        Span::styled(format!("{} {vertical}", " ".repeat(pad)), border_style()),
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

        let (line_end, next_start) = wrap_break(rest, width);
        lines.push(rest[..line_end].to_string());
        rest = &rest[next_start..];
    }
    lines
}

/// Choose a word-break in `text` so the first line fits `width` display cells.
/// Returns byte offsets `(line_end, next_start)`: where the line ends and where the
/// next line resumes (skipping the break space). Always makes progress — falls back
/// to a hard character split when a single word/char is wider than the line.
fn wrap_break(text: &str, width: usize) -> (usize, usize) {
    let head = take_width(text, width).0;
    if head.is_empty() {
        // The first character is wider than the whole line; consume it so we
        // always make progress and never split mid-character.
        let end = text.chars().next().map_or(text.len(), char::len_utf8);
        (end, end)
    } else {
        match head.rfind(' ') {
            Some(space) => (space, space + 1),
            None => (head.len(), head.len()),
        }
    }
}

/// Greedy word-wrap by display width where the first line fits `first_width`
/// cells and every following line fits `rest_width`. Unbounded (never
/// truncates), so long flowing labels break onto as many lines as needed.
pub(crate) fn wrap_text_hanging(text: &str, first_width: usize, rest_width: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let width = if lines.is_empty() {
            first_width
        } else {
            rest_width
        };
        if width == 0 {
            // No room on this line; if we've made no progress at all the value
            // is unrenderable, so bail rather than loop forever.
            if lines.is_empty() {
                lines.push(rest.to_string());
            }
            break;
        }
        if text_width(rest) <= width {
            lines.push(rest.to_string());
            break;
        }

        let (line_end, next_start) = wrap_break(rest, width);
        lines.push(rest[..line_end].to_string());
        rest = &rest[next_start..];
    }
    lines
}

/// Truncate `text` to `max` display cells, ending with `…` when it overflows.
pub(crate) fn truncate_ellipsis(text: &str, max: usize) -> String {
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

pub(crate) fn ensure_row_visible(
    scroll: &mut usize,
    rows: &[RowMeta],
    selected_entry_index: Option<usize>,
    viewport_height: u16,
) {
    let Some((row_start, row_height)) = selected_entry_row_span(rows, selected_entry_index) else {
        *scroll = clamp_scroll(*scroll, total_row_height(rows), viewport_height);
        return;
    };

    if viewport_height == 0 {
        *scroll = clamp_scroll(*scroll, total_row_height(rows), viewport_height);
        return;
    }

    if row_start < *scroll {
        // If nothing selectable sits above this row, it's the topmost entry —
        // scroll to 0 so the leading spacer/divider rows are revealed too,
        // rather than stopping at the row's own start and hiding the blank line
        // above it.
        let selectable_above = rows
            .iter()
            .take_while(|row| row.item_index != selected_entry_index)
            .any(|row| row.item_index.is_some());
        *scroll = if selectable_above { row_start } else { 0 };
    } else {
        let row_end = row_start.saturating_add(row_height as usize);
        let viewport_end = scroll.saturating_add(viewport_height as usize);
        if row_end > viewport_end {
            *scroll = row_end.saturating_sub(viewport_height as usize);
        }
    }
    *scroll = clamp_scroll(*scroll, total_row_height(rows), viewport_height);
}

pub(crate) fn selected_entry_row_span(
    rows: &[RowMeta],
    selected_entry_index: Option<usize>,
) -> Option<(usize, u16)> {
    selected_entry_index?;
    let mut y = 0usize;
    for row in rows {
        if row.item_index == selected_entry_index {
            return Some((y, row.height));
        }
        y = y.saturating_add(row.height as usize);
    }
    None
}

pub(crate) fn total_row_height(rows: &[RowMeta]) -> usize {
    rows.iter().map(|row| row.height as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_row_visible_scrolls_past_u16_max_for_tall_lists() {
        // 100 boxes of 1000 rows each → 100_000 px total, far beyond u16::MAX
        // (65535). Selecting the last one must scroll to the very bottom rather
        // than clamping short — the "can't scroll to the end" regression.
        let rows: Vec<RowMeta> = (0..100)
            .map(|index| RowMeta {
                item_index: Some(index),
                height: 1000,
            })
            .collect();

        let mut scroll = 0usize;
        ensure_row_visible(&mut scroll, &rows, Some(99), 20);

        assert_eq!(scroll, 100_000 - 20);
        assert!(scroll > u16::MAX as usize);
    }

    #[test]
    fn ensure_row_visible_reveals_leading_spacer_for_first_entry() {
        // A leading blank spacer (item_index: None) precedes the entries, so the
        // first entry sits at pixel 1. Scrolling up onto it must snap to 0 so the
        // spacer comes into view, rather than stopping at the row's own start.
        let mut rows = vec![RowMeta {
            item_index: None,
            height: 1,
        }];
        rows.extend((0..10).map(|index| RowMeta {
            item_index: Some(index),
            height: 3,
        }));

        let mut scroll = 5usize;
        ensure_row_visible(&mut scroll, &rows, Some(0), 6);
        assert_eq!(scroll, 0);

        // Selecting a later entry still stops at that entry's start (no over-eager
        // snap to the top).
        let mut scroll = 20usize;
        ensure_row_visible(&mut scroll, &rows, Some(1), 6);
        assert_eq!(scroll, 4);
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn starred_glyph_follows_bottom_left_label() {
        // The star rides the bottom-left label (just after the word count), so it
        // never collides with a bottom-right label like the search "Archived" flag.
        let starred = footer_left_label("2 words".to_string(), true);
        let lines = entry_box_lines(None, "14:30", "hi", Some(&starred), Some("Archived"), 30);
        let bottom = line_text(lines.last().unwrap());
        assert!(bottom.starts_with("└ 2 words ★ "));
        assert!(bottom.ends_with(" Archived ┘"));

        // Unstarred: the label is untouched and no star appears.
        let plain = footer_left_label("2 words".to_string(), false);
        assert_eq!(plain, "2 words");
        let plain_lines = entry_box_lines(None, "14:30", "hi", Some(&plain), None, 30);
        assert!(!line_text(plain_lines.last().unwrap()).contains('★'));
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
