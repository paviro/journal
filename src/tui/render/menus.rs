//! Table-shaped overlay menus (metadata, settings, editor shortcuts) and the
//! `TableDialog` machinery that lays them out, draws them, and hit-tests rows.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::table;
use crate::tui::surface::surface_outer_width;
use crate::tui::theme::Theme;

use super::chrome::{
    centered_rect_fixed_size, clear_surface, flat_chrome, render_scrollbar_if_needed,
};
use super::footer::{key_chip_style, key_chip_text};
use super::frames::{dialog_frame_rows, dialog_inner, draw_dialog_frame};

mod overlays;
pub(crate) use overlays::{
    MetadataMenuMode, draw_metadata_menu, draw_settings_menu, metadata_menu_interactions,
    settings_menu_interactions,
};

const EDITOR_SHORTCUT_SECTIONS: [(&str, &[(&str, &str)]); 4] = [
    (
        "File",
        &[
            ("ctrl/⌘+s", "Save"),
            ("ctrl+o", "Fullscreen"),
            ("ctrl+g", "Metadata"),
            ("esc", "Discard"),
        ],
    ),
    (
        "Edit",
        &[
            ("ctrl+a", "Select all"),
            ("ctrl/⌘+z", "Undo"),
            ("ctrl+y · ⌘⇧z", "Redo"),
            ("ctrl/⌘+x", "Cut → clipboard"),
            ("ctrl/⌘+c", "Copy → clipboard"),
            ("ctrl/⌘+v", "Paste"),
            ("ctrl+k", "Cut to line end"),
            ("ctrl+w", "Delete word"),
        ],
    ),
    (
        "Move",
        &[
            ("arrows", "Move"),
            ("shift+move", "Select"),
            ("ctrl/⌥+←/→", "Word"),
            ("home/end", "Line start/end"),
            ("ctrl+↑/↓", "Paragraph"),
            ("pgup/pgdn", "Page"),
        ],
    ),
    // The textarea also honors these emacs bindings; the app leaves them alone
    // (unlike ctrl+a, which it takes for select-all).
    (
        "Emacs",
        &[
            ("ctrl+b/f", "Char left / right"),
            ("ctrl+p/n", "Line up / down"),
            ("ctrl+e", "Line end"),
            ("ctrl+h/d", "Delete back / forward"),
            ("ctrl+j", "Delete to line start"),
        ],
    ),
];

/// Draw the internal editor's shortcut reference: the same centered, multi-column
/// table as the global help overlay. Opened with Ctrl+T, scrolled with the
/// arrows/page keys, dismissed by any other key or a click.
pub(crate) fn draw_editor_shortcuts(theme: &Theme, frame: &mut Frame<'_>, scroll: &mut u16) {
    draw_section_table(
        theme,
        frame,
        &EDITOR_SHORTCUT_SECTIONS,
        "Editor Shortcuts",
        "press any key to close",
        scroll,
    );
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
fn dialog_cell(
    theme: &Theme,
    text: &str,
    col: usize,
    key_col: usize,
    width: usize,
) -> Vec<Span<'static>> {
    if col == key_col && !text.is_empty() {
        let chip = key_chip_text(text);
        let padding = width.saturating_sub(UnicodeWidthStr::width(chip.as_str()));
        return vec![
            Span::styled(chip, key_chip_style(theme)),
            Span::raw(" ".repeat(padding)),
        ];
    }
    let style = if col == 0 && col != key_col {
        theme.heading()
    } else {
        theme.text()
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
fn row_separator(theme: &Theme, widths: &[usize], row: &[String], muted: Style) -> Line<'static> {
    let faint = table::themed_faint_rule_style(theme);
    let set = theme.glyphs().borders.line_set();
    let mut spans = vec![Span::styled(set.vertical.to_string(), muted)];
    for (c, w) in widths.iter().enumerate() {
        if c > 0 {
            spans.push(Span::styled(set.vertical.to_string(), muted));
        }
        if row[c].is_empty() {
            spans.push(Span::raw(" ".repeat(w + 2)));
        } else {
            spans.push(Span::styled(set.horizontal.repeat(w + 2), faint));
        }
    }
    spans.push(Span::styled(set.vertical.to_string(), muted));
    Line::from(spans)
}

/// The full bordered grid (insights style): outer border, muted header, and a faint
/// rule between each row. Returns the lines and the table's total column width.
fn grid_table(
    theme: &Theme,
    headers: &[&str],
    rows: &[Vec<String>],
    key_col: usize,
) -> (Vec<Line<'static>>, u16) {
    let widths = dialog_widths(headers, rows, key_col);
    let muted = table::themed_border_style(theme);

    let mut lines = Vec::with_capacity(2 * rows.len() + 4);
    lines.push(table::themed_rule(
        theme,
        &widths,
        table::RulePos::Top,
        muted,
        muted,
    ));
    let mut header = vec![table::themed_border(theme)];
    for (c, label) in headers.iter().enumerate() {
        table::themed_push_cell_spans(
            theme,
            &mut header,
            vec![Span::styled(table::pad(label, widths[c], false), muted)],
        );
    }
    lines.push(Line::from(header));
    lines.push(table::themed_rule(
        theme,
        &widths,
        table::RulePos::Mid,
        muted,
        muted,
    ));
    for (r, row) in rows.iter().enumerate() {
        // A faint rule between rows, its column borders running straight through as
        // plain `│` so the verticals stay continuous — matching the insights table.
        // A spanning group cell (empty on continuation rows) keeps its rule blank so
        // it reads as one merged cell.
        if r > 0 {
            lines.push(row_separator(theme, &widths, row, muted));
        }
        let mut spans = vec![table::themed_border(theme)];
        for (c, text) in row.iter().enumerate() {
            table::themed_push_cell_spans(
                theme,
                &mut spans,
                dialog_cell(theme, text, c, key_col, widths[c]),
            );
        }
        lines.push(Line::from(spans));
    }
    lines.push(table::themed_rule(
        theme,
        &widths,
        table::RulePos::Bottom,
        muted,
        muted,
    ));

    // Each column renders as `│ <content> `; the last cell adds the closing `│`.
    let width = widths.iter().map(|w| w + 3).sum::<usize>() + 1;
    (lines, width as u16)
}

/// The chrome-less fallback: one data row per line (no borders or rules), columns
/// aligned and separated by two spaces — the same collapse the insights tabs use
/// when there isn't room for the full grid.
fn compact_table(
    theme: &Theme,
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
                spans.extend(dialog_cell(theme, text, c, key_col, widths[c]));
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
    theme: &'a Theme,
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

pub(crate) struct MenuInteractions {
    pub(crate) rows: Vec<(Rect, usize)>,
    pub(crate) footer: Rect,
}

fn table_dialog_metrics(frame_area: Rect, dialog: &TableDialog, scroll: u16) -> TableDialogMetrics {
    let theme = dialog.theme;
    // Rows the frame takes around the table: the two border rows (which carry
    // the title and footer) when bordered; flat pads the title and gives the
    // footer its own row above the bottom padding.
    let frame_rows = if flat_chrome(theme) {
        dialog_frame_rows(theme) + 1
    } else {
        dialog_frame_rows(theme)
    };
    let (grid_lines, grid_w) = grid_table(theme, dialog.headers, dialog.rows, dialog.key_col);
    let avail_h = frame_area.height.saturating_sub(2).max(3);
    let (lines, content_w, grid) = if grid_lines.len() as u16 + frame_rows <= avail_h {
        (grid_lines, grid_w, true)
    } else {
        let (compact_lines, compact_w) =
            compact_table(theme, dialog.headers, dialog.rows, dialog.key_col);
        (compact_lines, compact_w, false)
    };
    let total = lines.len() as u16;
    let outer_h = (total + frame_rows).min(avail_h);
    let footer = if total > outer_h.saturating_sub(frame_rows) {
        format!("↑↓ scroll · {}", dialog.footer)
    } else {
        dialog.footer.to_string()
    };
    let border_label = |text: &str| surface_outer_width(theme, UnicodeWidthStr::width(text) as u16);
    let outer_w = surface_outer_width(theme, content_w)
        .max(border_label(dialog.title))
        .max(border_label(&footer))
        .min(frame_area.width);
    let area = centered_rect_fixed_size(outer_w, outer_h, frame_area);
    let mut content = dialog_inner(theme, area);
    if flat_chrome(theme) {
        // The last inner row belongs to the footer.
        content.height = content.height.saturating_sub(1);
    }
    let content_w = content_w.min(content.width);
    let content = Rect {
        x: content.x + (content.width - content_w) / 2,
        width: content_w,
        ..content
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

fn table_dialog_interactions(
    frame_area: Rect,
    dialog: &TableDialog,
    scroll: u16,
) -> MenuInteractions {
    let metrics = table_dialog_metrics(frame_area, dialog, scroll);
    let rows = (0..metrics.content.height)
        .filter_map(|visible_line| {
            let content_line = visible_line.saturating_add(metrics.scroll);
            let index = if metrics.grid {
                if content_line < 3 || !(content_line - 3).is_multiple_of(2) {
                    return None;
                }
                (content_line - 3) / 2
            } else {
                content_line
            };
            ((index as usize) < dialog.rows.len()).then_some((
                Rect {
                    y: metrics.content.y + visible_line,
                    height: 1,
                    ..metrics.content
                },
                index as usize,
            ))
        })
        .collect();
    MenuInteractions {
        rows,
        footer: Rect {
            x: metrics.area.x,
            y: metrics.area.y + metrics.area.height.saturating_sub(1),
            width: metrics.area.width,
            height: 1,
        },
    }
}

/// The global keyboard-shortcut cheatsheet, grouped by the panel/context each key
/// applies to. Opened with `?` from browse or a search result.
const HELP_SECTIONS: [(&str, &[(&str, &str)]); 6] = [
    (
        "Move",
        &[
            ("↑ ↓", "Move / scroll"),
            ("← →", "Panels"),
            ("enter", "View / expand"),
            ("esc", "Back"),
        ],
    ),
    (
        "Journals",
        &[("n", "New journal"), ("a", "Archive"), ("d", "Delete")],
    ),
    (
        "Entry",
        &[
            ("e", "Edit"),
            ("n", "New entry"),
            ("d", "Delete"),
            ("s", "Star"),
            ("i", "Images"),
        ],
    ),
    (
        "Metadata",
        &[
            ("t", "Tags"),
            ("p", "People"),
            ("a", "Activities"),
            ("f", "Feelings"),
            ("m", "Mood"),
            ("l", "Location"),
            ("ctrl+g", "Metadata menu"),
        ],
    ),
    ("Insights", &[("g", "Scope"), ("w", "Timeframe")]),
    (
        "General",
        &[
            ("/", "Search"),
            ("j", "Journals"),
            (",", "Settings"),
            ("h", "Toggle hints"),
            ("r", "Refresh"),
            ("?", "This help"),
            ("q", "Quit"),
        ],
    ),
];

/// Cap on the cheatsheet's columns: three reads as a balanced grid without the
/// key/action pairs drifting too far apart.
const HELP_MAX_COLS: usize = 3;

/// The `│`-with-a-space-each-side rule drawn between two columns.
const HELP_RULE_PAD: u16 = 3;

/// One section rendered as a block of lines: a bold group heading, a full-`width`
/// faint rule under it, then its bindings — each an aligned key chip followed by
/// the action. The rows themselves are left ragged; `section_table_lines` pads
/// every cell to the column width when it splices the columns together.
fn section_block(
    theme: &Theme,
    group: &str,
    items: &[(&str, &str)],
    width: usize,
) -> Vec<Line<'static>> {
    let chip_w = max_chip_width(items);
    let set = theme.glyphs().borders.line_set();
    let mut lines = Vec::with_capacity(items.len() + 2);
    lines.push(Line::from(Span::styled(group.to_string(), theme.heading())));
    lines.push(Line::from(Span::styled(
        set.horizontal.repeat(width),
        table::themed_faint_rule_style(theme),
    )));
    for (keys, action) in items {
        let chip = key_chip_text(keys);
        let gap = chip_w.saturating_sub(UnicodeWidthStr::width(chip.as_str())) + 2;
        lines.push(Line::from(vec![
            Span::styled(chip, key_chip_style(theme)),
            Span::raw(" ".repeat(gap)),
            Span::styled((*action).to_string(), theme.text()),
        ]));
    }
    lines
}

/// The widest key chip in a section — every binding's action starts two columns
/// past it, so the chips and actions line up. Shared by `section_block` (layout)
/// and `section_width` (sizing) so the two cannot drift.
fn max_chip_width(items: &[(&str, &str)]) -> usize {
    items
        .iter()
        .map(|(keys, _)| UnicodeWidthStr::width(key_chip_text(keys).as_str()))
        .max()
        .unwrap_or(0)
}

/// The widest line a section block would render at, used to size every column
/// to a common width before the blocks are built.
fn section_width(items: &[(&str, &str)]) -> usize {
    let chip_w = max_chip_width(items);
    items
        .iter()
        .map(|(_, action)| chip_w + 2 + UnicodeWidthStr::width(*action))
        .max()
        .unwrap_or(0)
}

/// Split the sections into `ncols` contiguous, non-empty columns, choosing the
/// cut that keeps the tallest column as short as possible so the grid reads as
/// balanced (and no column is ever left empty). A blank row separates sections
/// stacked in the same column.
fn section_columns(blocks: &[Vec<Line<'static>>], ncols: usize) -> Vec<Vec<Line<'static>>> {
    let ncols = ncols.clamp(1, blocks.len().max(1));
    let sizes: Vec<usize> = blocks.iter().map(Vec::len).collect();
    let mut start = 0;
    balanced_splits(&sizes, ncols)
        .into_iter()
        .map(|end| {
            let mut column: Vec<Line<'static>> = Vec::new();
            for block in &blocks[start..end] {
                if !column.is_empty() {
                    column.push(Line::default());
                }
                column.extend(block.iter().cloned());
            }
            start = end;
            column
        })
        .collect()
}

/// The tallest column produced by a set of end-boundaries, counting a blank row
/// between sections stacked in the same column.
pub(super) fn column_span(sizes: &[usize], bounds: &[usize]) -> usize {
    let mut start = 0;
    let mut tallest = 0;
    for &end in bounds {
        let height = sizes[start..end].iter().sum::<usize>() + (end - start).saturating_sub(1);
        tallest = tallest.max(height);
        start = end;
    }
    tallest
}

/// Recurse over every way to place the remaining column boundaries after
/// `start`, keeping the split whose tallest column is smallest. Each column
/// must take at least one section, so a boundary always leaves room for the
/// columns still to come.
fn search_splits(
    sizes: &[usize],
    start: usize,
    cols: usize,
    current: &mut Vec<usize>,
    best: &mut Option<(usize, Vec<usize>)>,
) {
    let n = sizes.len();
    if cols == 1 {
        current.push(n);
        let span = column_span(sizes, current);
        if best.as_ref().is_none_or(|(best_span, _)| span < *best_span) {
            *best = Some((span, current.clone()));
        }
        current.pop();
        return;
    }
    for end in (start + 1)..=(n - (cols - 1)) {
        current.push(end);
        search_splits(sizes, end, cols - 1, current, best);
        current.pop();
    }
}

/// End-boundaries of the `ncols` columns (the last is always `sizes.len()`) that
/// minimize the tallest column when the sections are cut into contiguous groups.
pub(super) fn balanced_splits(sizes: &[usize], ncols: usize) -> Vec<usize> {
    let n = sizes.len();
    if n == 0 {
        return vec![0];
    }
    // Clamp so a request for more columns than sections can't underflow the
    // `n - (cols - 1)` range in `search_splits`; production callers already keep
    // `ncols <= n`, so this only guards direct/edge use.
    let ncols = ncols.min(n);
    if ncols <= 1 {
        return vec![n];
    }
    let mut best = None;
    let mut current = Vec::with_capacity(ncols);
    search_splits(sizes, 0, ncols, &mut current, &mut best);
    best.map_or_else(|| vec![n], |(_, bounds)| bounds)
}

/// Splice the columns into one table body, row by row, with a themed vertical
/// rule between each pair. Short columns are padded with blank rows so the rules
/// run straight to the bottom.
fn section_table_lines(
    theme: &Theme,
    columns: &[Vec<Line<'static>>],
    col_w: usize,
) -> Vec<Line<'static>> {
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    let rule = theme.glyphs().borders.line_set().vertical.to_string();
    (0..rows)
        .map(|r| {
            let mut spans = Vec::new();
            for (c, column) in columns.iter().enumerate() {
                if c > 0 {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        rule.to_string(),
                        table::themed_border_style(theme),
                    ));
                    spans.push(Span::raw(" "));
                }
                // Pad every cell — real, short, or missing — to the column
                // width so the rules run dead straight down the table.
                let cell = column
                    .get(r)
                    .map(|line| line.spans.as_slice())
                    .unwrap_or(&[]);
                let used: usize = cell
                    .iter()
                    .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                    .sum();
                spans.extend(cell.iter().cloned());
                spans.push(Span::raw(" ".repeat(col_w.saturating_sub(used))));
            }
            Line::from(spans)
        })
        .collect()
}

/// Draw the global keyboard cheatsheet: the centered, multi-column reference
/// table opened with `?` from browse or a search result.
pub(crate) fn draw_help(theme: &Theme, frame: &mut Frame<'_>, scroll: &mut u16) {
    draw_section_table(
        theme,
        frame,
        &HELP_SECTIONS,
        "Keyboard Shortcuts",
        "press any key to close",
        scroll,
    );
}

/// Draw a keyboard cheatsheet as a centered table: the `sections` arranged in
/// balanced columns split by vertical rules, sized to their content and
/// centered on screen. `scroll` only engages when a short terminal can't show
/// every row. Shared by the global help overlay and the editor's reference.
fn draw_section_table(
    theme: &Theme,
    frame: &mut Frame<'_>,
    sections: &[(&str, &[(&str, &str)])],
    title: &str,
    footer: &str,
    scroll: &mut u16,
) {
    let frame_area = frame.area();
    let col_w = sections
        .iter()
        .map(|(_, items)| section_width(items))
        .max()
        .unwrap_or(0) as u16;

    // Widen the grid until it either runs out of horizontal room or hits the
    // column cap; more columns means a shorter, squarer table.
    let avail_w = frame_area
        .width
        .saturating_sub(surface_outer_width(theme, 0));
    let fit = ((avail_w + HELP_RULE_PAD) / (col_w + HELP_RULE_PAD)).max(1) as usize;
    let ncols = fit.min(HELP_MAX_COLS).min(sections.len());

    let blocks: Vec<Vec<Line<'static>>> = sections
        .iter()
        .map(|(group, items)| section_block(theme, group, items, col_w as usize))
        .collect();
    let columns = section_columns(&blocks, ncols);
    let ncols = columns.len() as u16;
    let lines = section_table_lines(theme, &columns, col_w as usize);

    let content_w = col_w * ncols + HELP_RULE_PAD * ncols.saturating_sub(1);
    let frame_rows = if flat_chrome(theme) {
        dialog_frame_rows(theme) + 1
    } else {
        dialog_frame_rows(theme)
    };
    let avail_h = frame_area.height.saturating_sub(2).max(3);
    let total = lines.len() as u16;
    // A blank row sits between the table and the footer, so the box is a row
    // taller than its content and the footer breathes off the last line.
    let outer_h = (total + frame_rows + 1).min(avail_h);
    let content_h = outer_h.saturating_sub(frame_rows + 1);
    let border_label = |text: &str| surface_outer_width(theme, UnicodeWidthStr::width(text) as u16);
    let outer_w = surface_outer_width(theme, content_w)
        .max(border_label(title))
        .max(border_label(footer))
        .min(frame_area.width);
    let area = centered_rect_fixed_size(outer_w, outer_h, frame_area);

    let mut content = dialog_inner(theme, area);
    if flat_chrome(theme) {
        content.height = content.height.saturating_sub(1);
    }
    // Center the table within the (possibly wider) content box.
    let content = Rect {
        x: content.x + content.width.saturating_sub(content_w) / 2,
        width: content_w.min(content.width),
        height: content_h,
        ..content
    };
    *scroll = (*scroll).min(total.saturating_sub(content.height));

    if flat_chrome(theme) {
        draw_dialog_frame(theme, frame, area, title, false);
        let bottom = Rect {
            y: area.y + area.height.saturating_sub(2),
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(Span::styled(footer.to_string(), theme.muted()))
                .alignment(Alignment::Center),
            bottom,
        );
    } else {
        clear_surface(theme, frame, area, theme.dialog_bg());
        let block = Block::default()
            .title(format!(" {title} "))
            .title_bottom(Line::from(format!(" {footer} ")).centered())
            .borders(Borders::ALL)
            .border_set(theme.glyphs().borders.border_set())
            .border_style(theme.dialog_border());
        frame.render_widget(block, area);
    }

    frame.render_widget(Paragraph::new(lines).scroll((*scroll, 0)), content);
    render_scrollbar_if_needed(
        theme,
        frame,
        area,
        total as usize,
        content.height,
        *scroll as usize,
        true,
    );
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
    let theme = dialog.theme;
    let mut metrics = table_dialog_metrics(frame.area(), dialog, *scroll);
    *scroll = metrics.scroll;
    // Lift the hovered data row; the line index mirrors
    // `table_dialog_interactions`'s mapping so hover and click can't disagree.
    if let Some(row) = hovered_row {
        let line = if metrics.grid { 3 + 2 * row } else { row };
        if let Some(line) = metrics.lines.get_mut(line) {
            line.style = line.style.patch(theme.hover());
        }
    }

    if flat_chrome(theme) {
        draw_dialog_frame(theme, frame, metrics.area, dialog.title, false);
        // The footer moves from the bottom border to its own row above the
        // bottom padding.
        let bottom = Rect {
            y: metrics.area.y + metrics.area.height.saturating_sub(2),
            height: 1,
            ..metrics.area
        };
        frame.render_widget(
            Paragraph::new(Span::styled(metrics.footer.clone(), theme.muted()))
                .alignment(Alignment::Center),
            bottom,
        );
    } else {
        clear_surface(theme, frame, metrics.area, theme.dialog_bg());
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .title_bottom(Line::from(format!(" {} ", metrics.footer)).centered())
            .borders(Borders::ALL)
            .border_set(theme.glyphs().borders.border_set())
            .border_style(theme.dialog_border());
        frame.render_widget(block, metrics.area);
    }

    frame.render_widget(
        Paragraph::new(metrics.lines).scroll((metrics.scroll, 0)),
        metrics.content,
    );
    render_scrollbar_if_needed(
        theme,
        frame,
        metrics.area,
        metrics.total as usize,
        metrics.content.height,
        metrics.scroll as usize,
        true,
    );
}
