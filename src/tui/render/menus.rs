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
use crate::tui::state::MetadataKind;
use crate::tui::surface::{point_in_rect, surface_outer_width};
use crate::tui::theme::theme;

use super::chrome::{
    centered_rect_fixed_size, clear_surface, flat_chrome, render_scrollbar_if_needed,
};
use super::footer::{HintId, key_chip_style, key_chip_text};
use super::frames::{dialog_frame_rows, dialog_inner, draw_dialog_frame};

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
    let set = theme().glyphs().borders.line_set();
    let mut spans = vec![Span::styled(set.vertical, muted)];
    for (c, w) in widths.iter().enumerate() {
        if c > 0 {
            spans.push(Span::styled(set.vertical, muted));
        }
        if row[c].is_empty() {
            spans.push(Span::raw(" ".repeat(w + 2)));
        } else {
            spans.push(Span::styled(set.horizontal.repeat(w + 2), faint));
        }
    }
    spans.push(Span::styled(set.vertical, muted));
    Line::from(spans)
}

/// The full bordered grid (insights style): outer border, muted header, and a faint
/// rule between each row. Returns the lines and the table's total column width.
fn grid_table(headers: &[&str], rows: &[Vec<String>], key_col: usize) -> (Vec<Line<'static>>, u16) {
    let widths = dialog_widths(headers, rows, key_col);
    let muted = table::border_style();

    let mut lines = Vec::with_capacity(2 * rows.len() + 4);
    lines.push(table::rule(&widths, table::RulePos::Top, muted, muted));
    let mut header = vec![table::border()];
    for (c, label) in headers.iter().enumerate() {
        table::push_cell_spans(
            &mut header,
            vec![Span::styled(table::pad(label, widths[c], false), muted)],
        );
    }
    lines.push(Line::from(header));
    lines.push(table::rule(&widths, table::RulePos::Mid, muted, muted));
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
    lines.push(table::rule(&widths, table::RulePos::Bottom, muted, muted));

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
    // Rows the frame takes around the table: the two border rows (which carry
    // the title and footer) when bordered; flat pads the title and gives the
    // footer its own row above the bottom padding.
    let frame_rows = if flat_chrome() {
        dialog_frame_rows() + 1
    } else {
        dialog_frame_rows()
    };
    let (grid_lines, grid_w) = grid_table(dialog.headers, dialog.rows, dialog.key_col);
    let avail_h = frame_area.height.saturating_sub(2).max(3);
    let (lines, content_w, grid) = if grid_lines.len() as u16 + frame_rows <= avail_h {
        (grid_lines, grid_w, true)
    } else {
        let (compact_lines, compact_w) = compact_table(dialog.headers, dialog.rows, dialog.key_col);
        (compact_lines, compact_w, false)
    };
    let total = lines.len() as u16;
    let outer_h = (total + frame_rows).min(avail_h);
    let footer = if total > outer_h.saturating_sub(frame_rows) {
        format!("↑↓ scroll · {}", dialog.footer)
    } else {
        dialog.footer.to_string()
    };
    let border_label = |text: &str| surface_outer_width(UnicodeWidthStr::width(text) as u16);
    let outer_w = surface_outer_width(content_w)
        .max(border_label(dialog.title))
        .max(border_label(&footer))
        .min(frame_area.width);
    let area = centered_rect_fixed_size(outer_w, outer_h, frame_area);
    let mut content = dialog_inner(area);
    if flat_chrome() {
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
        // The footer moves from the bottom border to its own row above the
        // bottom padding.
        let bottom = Rect {
            y: metrics.area.y + metrics.area.height.saturating_sub(2),
            height: 1,
            ..metrics.area
        };
        frame.render_widget(
            Paragraph::new(Span::styled(metrics.footer.clone(), theme().muted()))
                .alignment(Alignment::Center),
            bottom,
        );
    } else {
        clear_surface(frame, metrics.area, theme().dialog_bg());
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .title_bottom(Line::from(format!(" {} ", metrics.footer)).centered())
            .borders(Borders::ALL)
            .border_set(theme().glyphs().borders.border_set())
            .border_style(theme().dialog_border());
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
        true,
    );
}
