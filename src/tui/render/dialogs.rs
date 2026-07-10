use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph},
};

use unicode_width::UnicodeWidthStr;

use crate::tui::app::{
    EditFeelingState, EditLocationFocus, EditLocationState, EditMetadataFocus, EditMetadataState,
    FeelingRow, LocationResolveStatus,
};
use crate::tui::entry_rows::wrap_text;
use crate::tui::state::{DeleteContext, EditMoodState, ListNav};
use crate::tui::surface::metadata_value_rows;
use crate::tui::text_input::TextInput;

use super::{
    chrome::{
        Hint, HintId, centered_rect_fixed_size, hint_height, hint_lines, render_confirm_buttons,
        render_scrollbar_if_needed,
    },
    list_state_for_render,
    markdown_panel::MoodBar,
};
use std::time::Instant;

// ── Hint text constants and helpers ──────────────────────────────────────────

const FEELINGS_DIALOG_LIST_HINTS: [Hint; 6] = [
    Hint::new("open", "→", HintId::FeelingsExpand),
    Hint::new("close", "←", HintId::FeelingsCollapse),
    Hint::new("toggle", "space", HintId::FeelingsToggle),
    Hint::new("search", "tab", HintId::FeelingsSwitchFocus),
    Hint::new("save", "enter", HintId::FeelingsSave),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const FEELINGS_DIALOG_INPUT_HINTS: [Hint; 4] = [
    Hint::new("list", "tab", HintId::FeelingsSwitchFocus),
    Hint::new("select all", "^a", HintId::InputSelectAll),
    Hint::new("save", "enter", HintId::FeelingsSave),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const SELECTED_LABEL: &str = "Selected: ";

/// Wrap the picked feelings into display rows, reusing the entry view's
/// metadata-row layout: the "Selected: " label reserves the first row's leading
/// width, values are separated by " | ", and each row is a list of indices into
/// `selected`. Empty when nothing is picked (rendered as "Selected: none").
fn feelings_selected_rows(selected: &[String]) -> Vec<Vec<usize>> {
    let width = LIST_DIALOG_WIDTH.saturating_sub(2).max(1);
    metadata_value_rows(SELECTED_LABEL.len() as u16, width, selected)
}

/// Number of lines the "Selected: …" footer occupies once wrapped — at least one
/// (the "Selected: none" line when nothing is picked). Used both to size the dialog
/// and to render it, so the reserved height always matches the drawn lines.
pub(crate) fn feelings_selected_line_count(selected: &[String]) -> usize {
    feelings_selected_rows(selected).len().max(1)
}

const MOOD_DIALOG_HINTS: [Hint; 5] = [
    Hint::new("decrease", "←", HintId::MoodDecrease),
    Hint::new("increase", "→", HintId::MoodIncrease),
    Hint::new("save", "enter", HintId::MoodSave),
    Hint::new("clear", "del", HintId::MoodClear),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const METADATA_DIALOG_LIST_HINTS: [Hint; 4] = [
    Hint::new("toggle", "space", HintId::MetadataToggle),
    Hint::new("input", "tab", HintId::MetadataSwitchFocus),
    Hint::new("save", "enter", HintId::MetadataSave),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const METADATA_DIALOG_INPUT_EMPTY_HINTS: [Hint; 3] = [
    Hint::new("save", "enter", HintId::MetadataSave),
    Hint::new("list", "tab", HintId::MetadataSwitchFocus),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const METADATA_DIALOG_INPUT_VALUE_HINTS: [Hint; 4] = [
    Hint::new("add", "enter", HintId::MetadataAddFromInput),
    Hint::new("list", "tab", HintId::MetadataSwitchFocus),
    Hint::new("select all", "^a", HintId::InputSelectAll),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const LOCATION_DIALOG_LIST_HINTS: [Hint; 4] = [
    Hint::new("pick", "enter", HintId::LocationSelectRow),
    Hint::new("edit", "tab", HintId::LocationSwitchFocus),
    Hint::new("clear", "del", HintId::LocationClear),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const LOCATION_DIALOG_QUERY_HINTS: [Hint; 5] = [
    Hint::new("look up", "enter", HintId::LocationResolve),
    Hint::new("locate", "^l", HintId::LocationGrabDevice),
    Hint::new("next", "tab", HintId::LocationSwitchFocus),
    Hint::new("select all", "^a", HintId::InputSelectAll),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

/// Query-field hints once the query is resolved: Enter now saves.
const LOCATION_DIALOG_QUERY_RESOLVED_HINTS: [Hint; 5] = [
    Hint::new("save", "enter", HintId::LocationSave),
    Hint::new("locate", "^l", HintId::LocationGrabDevice),
    Hint::new("next", "tab", HintId::LocationSwitchFocus),
    Hint::new("select all", "^a", HintId::InputSelectAll),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const LOCATION_DIALOG_NAME_HINTS: [Hint; 5] = [
    Hint::new("save", "enter", HintId::LocationSave),
    Hint::new("locate", "^l", HintId::LocationGrabDevice),
    Hint::new("next", "tab", HintId::LocationSwitchFocus),
    Hint::new("select all", "^a", HintId::InputSelectAll),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const LIST_DIALOG_WIDTH: u16 = 44;
const LOCATION_DIALOG_WIDTH: u16 = 66;
const LOCATION_DIALOG_MAX_VISIBLE_ROWS: u16 = 8;
/// Text width available to a wrapped list row: the inner width less the border,
/// the selection-cursor column, and one leading pad column.
const LOCATION_LIST_WRAP_WIDTH: usize = (LOCATION_DIALOG_WIDTH - 4) as usize;
/// Cap the lines a single (wrapped) list row may occupy.
const LOCATION_LIST_MAX_ITEM_LINES: usize = 3;

/// Wrap a list label into its display lines (at least one).
fn location_row_lines(label: &str) -> Vec<String> {
    let lines = wrap_text(
        label,
        LOCATION_LIST_WRAP_WIDTH,
        LOCATION_LIST_MAX_ITEM_LINES,
    );
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

/// Total rows the list occupies once every label is wrapped — what the dialog is
/// sized to, so multi-line rows aren't clipped.
pub(crate) fn location_list_rows(labels: &[String]) -> usize {
    labels
        .iter()
        .map(|label| location_row_lines(label).len())
        .sum::<usize>()
        .max(1)
}

/// Map a click at `row` within the list `Rect` to a label index, accounting for
/// rows that wrap onto continuation lines. `offset` is the index of the first
/// visible label. `None` when the click lands past the last rendered row.
pub(crate) fn location_list_row_at(
    list: Rect,
    labels: &[String],
    offset: usize,
    row: u16,
) -> Option<usize> {
    let relative = row.checked_sub(list.y)? as usize;
    if relative >= list.height as usize {
        return None;
    }
    let mut line = 0usize;
    for (index, label) in labels.iter().enumerate().skip(offset) {
        line += location_row_lines(label).len();
        if relative < line {
            return Some(index);
        }
    }
    None
}
const MOOD_DIALOG_WIDTH: u16 = 90;
const CONFIRM_DIALOG_WIDTH: u16 = 42;
const NEW_JOURNAL_DIALOG_WIDTH: u16 = 56;
const METADATA_DIALOG_MAX_VISIBLE_ROWS: u16 = 14;
const FEELINGS_DIALOG_MAX_VISIBLE_ROWS: u16 = 16;

pub(crate) fn feelings_dialog_hints(focus: EditMetadataFocus) -> &'static [Hint] {
    match focus {
        EditMetadataFocus::List => &FEELINGS_DIALOG_LIST_HINTS,
        EditMetadataFocus::Input => &FEELINGS_DIALOG_INPUT_HINTS,
    }
}

pub(crate) fn mood_dialog_hints() -> &'static [Hint] {
    &MOOD_DIALOG_HINTS
}

pub(crate) fn location_dialog_hints(
    focus: EditLocationFocus,
    query_looked_up: bool,
) -> &'static [Hint] {
    match focus {
        EditLocationFocus::Query if query_looked_up => &LOCATION_DIALOG_QUERY_RESOLVED_HINTS,
        EditLocationFocus::Query => &LOCATION_DIALOG_QUERY_HINTS,
        EditLocationFocus::Name => &LOCATION_DIALOG_NAME_HINTS,
        EditLocationFocus::List => &LOCATION_DIALOG_LIST_HINTS,
    }
}

pub(crate) fn metadata_dialog_hints(
    focus: EditMetadataFocus,
    input_is_empty: bool,
) -> &'static [Hint] {
    match (focus, input_is_empty) {
        (EditMetadataFocus::List, _) => &METADATA_DIALOG_LIST_HINTS,
        (EditMetadataFocus::Input, true) => &METADATA_DIALOG_INPUT_EMPTY_HINTS,
        (EditMetadataFocus::Input, false) => &METADATA_DIALOG_INPUT_VALUE_HINTS,
    }
}

// ── Dialog area helpers (re-used by the mouse handler for hit-testing) ───────

pub(crate) fn metadata_dialog_area(frame_area: Rect, filtered_len: usize) -> Rect {
    const FIXED: u16 = 7;
    let hint_height = tag_dialog_hint_height(frame_area);
    let visible = (filtered_len as u16).clamp(1, METADATA_DIALOG_MAX_VISIBLE_ROWS);
    let h = (FIXED + hint_height + visible).min(frame_area.height.saturating_sub(2));
    super::centered_rect_fixed_size(LIST_DIALOG_WIDTH, h, frame_area)
}

/// Height of every row inside the dialog border that is *not* the list: the
/// title, both list separators, the search input, the selected summary and the
/// two blank spacers around it, and the hint block. Sizing the dialog and placing
/// the list both derive from this one value so they can't drift apart.
fn feelings_dialog_chrome_height(frame_area: Rect, selected_lines: usize) -> u16 {
    // title + two separators + search input + spacer + summary + spacer + hints
    1 + 2 + 1 + 1 + selected_lines as u16 + 1 + feelings_dialog_hint_height(frame_area)
}

pub(crate) fn feelings_dialog_area(
    frame_area: Rect,
    all_len: usize,
    selected_lines: usize,
) -> Rect {
    // Clamp to at least one row so the "(no matches)" line has somewhere to render
    // when a filter matches nothing, matching the metadata dialog.
    let visible = (all_len as u16).clamp(1, FEELINGS_DIALOG_MAX_VISIBLE_ROWS);
    const BORDERS: u16 = 2;
    let h = (BORDERS + feelings_dialog_chrome_height(frame_area, selected_lines) + visible)
        .min(frame_area.height.saturating_sub(2));
    super::centered_rect_fixed_size(LIST_DIALOG_WIDTH, h, frame_area)
}

pub(crate) fn mood_dialog_area(frame_area: Rect) -> Rect {
    let h = 7 + mood_dialog_hint_height(frame_area);
    super::centered_rect_fixed_size(
        MOOD_DIALOG_WIDTH,
        h.min(frame_area.height.saturating_sub(2)),
        frame_area,
    )
}

fn dialog_hint_width(frame_area: Rect, width: u16) -> u16 {
    let area = super::centered_rect_fixed_size(width, 1, frame_area);
    let inner = super::panel_inner(area);
    inner.width.saturating_sub(1)
}

fn tag_dialog_hint_height(frame_area: Rect) -> u16 {
    let width = dialog_hint_width(frame_area, LIST_DIALOG_WIDTH);
    hint_height(&METADATA_DIALOG_LIST_HINTS, width)
        .max(hint_height(&METADATA_DIALOG_INPUT_EMPTY_HINTS, width))
        .max(hint_height(&METADATA_DIALOG_INPUT_VALUE_HINTS, width))
}

fn feelings_dialog_hint_height(frame_area: Rect) -> u16 {
    // Reserve the taller of the two focus states so the layout stays put as the
    // user tabs between the list and the search input.
    let width = dialog_hint_width(frame_area, LIST_DIALOG_WIDTH);
    hint_height(&FEELINGS_DIALOG_LIST_HINTS, width)
        .max(hint_height(&FEELINGS_DIALOG_INPUT_HINTS, width))
}

fn mood_dialog_hint_height(frame_area: Rect) -> u16 {
    hint_height(&MOOD_DIALOG_HINTS, dialog_hint_width(frame_area, 44))
}

fn location_dialog_hint_height(frame_area: Rect) -> u16 {
    // Reserve the tallest focus state so the layout doesn't shift as focus moves.
    let width = dialog_hint_width(frame_area, LOCATION_DIALOG_WIDTH);
    hint_height(&LOCATION_DIALOG_QUERY_HINTS, width)
        .max(hint_height(&LOCATION_DIALOG_QUERY_RESOLVED_HINTS, width))
        .max(hint_height(&LOCATION_DIALOG_NAME_HINTS, width))
        .max(hint_height(&LOCATION_DIALOG_LIST_HINTS, width))
}

/// Fixed rows above the list, mirroring the feelings dialog's framing: a title,
/// a separator, the two inputs, a blank spacer, the status line, a separator, and
/// the list heading.
const LOCATION_DIALOG_CHROME: u16 = 8;
/// A blank row between the list and the hint block, matching the feelings dialog.
const LOCATION_DIALOG_HINTS_SPACER: u16 = 1;

pub(crate) fn location_dialog_area(frame_area: Rect, list_rows: usize) -> Rect {
    const BORDERS: u16 = 2;
    let hint_height = location_dialog_hint_height(frame_area);
    let visible = (list_rows as u16).clamp(1, LOCATION_DIALOG_MAX_VISIBLE_ROWS);
    let h =
        (BORDERS + LOCATION_DIALOG_CHROME + LOCATION_DIALOG_HINTS_SPACER + hint_height + visible)
            .min(frame_area.height.saturating_sub(2));
    super::centered_rect_fixed_size(LOCATION_DIALOG_WIDTH, h, frame_area)
}

#[derive(Clone, Copy)]
pub(crate) struct LocationDialogLayout {
    pub(crate) area: Rect,
    pub(crate) title: Rect,
    pub(crate) title_separator: Rect,
    pub(crate) name: Rect,
    pub(crate) query: Rect,
    pub(crate) status: Rect,
    pub(crate) list_separator: Rect,
    pub(crate) heading: Rect,
    pub(crate) list: Rect,
    pub(crate) hints: Rect,
}

pub(crate) fn location_dialog_layout(frame_area: Rect, list_rows: usize) -> LocationDialogLayout {
    let area = location_dialog_area(frame_area, list_rows);
    let inner = super::panel_inner(area);
    let hint_height = location_dialog_hint_height(frame_area);
    let row = |offset: u16| Rect {
        x: inner.x,
        y: inner.y + offset,
        width: inner.width,
        height: 1,
    };
    // Rows: title(0) sep(1) address(2) name(3) spacer(4) status(5) sep(6) heading(7),
    // then the list, a blank spacer, and the hints.
    let list_height = inner
        .height
        .saturating_sub(LOCATION_DIALOG_CHROME + LOCATION_DIALOG_HINTS_SPACER + hint_height);
    let list = Rect {
        x: inner.x,
        y: inner.y + LOCATION_DIALOG_CHROME,
        width: inner.width,
        height: list_height,
    };
    let hints = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(hint_height),
        width: inner.width,
        height: hint_height,
    };

    LocationDialogLayout {
        area,
        title: row(0),
        title_separator: row(1),
        query: row(2),
        name: row(3),
        // row(4) is a blank spacer between the inputs and the status line.
        status: row(5),
        list_separator: row(6),
        heading: row(7),
        list,
        hints,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MetadataDialogLayout {
    pub(crate) area: Rect,
    pub(crate) inner: Rect,
    pub(crate) list_top_separator: Rect,
    pub(crate) list: Rect,
    pub(crate) list_bottom_separator: Rect,
    pub(crate) input: Rect,
    pub(crate) hints: Rect,
}

pub(crate) fn metadata_dialog_layout(
    frame_area: Rect,
    filtered_len: usize,
) -> MetadataDialogLayout {
    let area = metadata_dialog_area(frame_area, filtered_len);
    let inner = super::panel_inner(area);
    let hint_height = tag_dialog_hint_height(frame_area);
    let list_height = inner.height.saturating_sub(5 + hint_height);
    let list = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: list_height,
    };
    let list_top_separator = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: 1,
    };
    let list_bottom_separator = Rect {
        x: inner.x,
        y: list.y + list.height,
        width: inner.width,
        height: 1,
    };
    let input = Rect {
        x: inner.x,
        y: list_bottom_separator.y + 1,
        width: inner.width,
        height: 1,
    };
    let hints = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(hint_height),
        width: inner.width,
        height: hint_height,
    };

    MetadataDialogLayout {
        area,
        inner,
        list_top_separator,
        list,
        list_bottom_separator,
        input,
        hints,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FeelingsDialogLayout {
    pub(crate) area: Rect,
    pub(crate) inner: Rect,
    pub(crate) list_top_separator: Rect,
    pub(crate) list: Rect,
    pub(crate) list_bottom_separator: Rect,
    pub(crate) input: Rect,
    pub(crate) selected: Rect,
    pub(crate) hints: Rect,
}

pub(crate) fn feelings_dialog_layout(
    frame_area: Rect,
    all_len: usize,
    selected_lines: usize,
) -> FeelingsDialogLayout {
    let area = feelings_dialog_area(frame_area, all_len, selected_lines);
    let inner = super::panel_inner(area);
    let hint_height = feelings_dialog_hint_height(frame_area);
    let selected_h = selected_lines as u16;
    let chrome = feelings_dialog_chrome_height(frame_area, selected_lines);
    let list = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(chrome),
    };
    let list_top_separator = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: 1,
    };
    let list_bottom_separator = Rect {
        x: inner.x,
        y: list.y + list.height,
        width: inner.width,
        height: 1,
    };
    let input = Rect {
        x: inner.x,
        y: list_bottom_separator.y + 1,
        width: inner.width,
        height: 1,
    };
    // A blank spacer line sits between the search input and the summary.
    let selected = Rect {
        x: inner.x,
        y: input.y + 2,
        width: inner.width,
        height: selected_h,
    };
    // A blank spacer line sits between `selected` and `hints`.
    let hints = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(hint_height),
        width: inner.width,
        height: hint_height,
    };

    FeelingsDialogLayout {
        area,
        inner,
        list_top_separator,
        list,
        list_bottom_separator,
        input,
        selected,
        hints,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MoodDialogLayout {
    pub(crate) area: Rect,
    pub(crate) inner: Rect,
    pub(crate) bar: Rect,
    pub(crate) value: Rect,
    pub(crate) hints: Rect,
}

pub(crate) fn mood_dialog_layout(frame_area: Rect) -> MoodDialogLayout {
    let area = mood_dialog_area(frame_area);
    let inner = super::panel_inner(area);
    let hint_height = mood_dialog_hint_height(frame_area);
    let right_w = " Blissful".len() as u16;
    let bar_row = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: 1,
    };
    let bar_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(3),
            Constraint::Length(right_w),
        ])
        .split(bar_row);
    let value_row = Rect {
        x: inner.x,
        y: inner.y + 3,
        width: inner.width,
        height: 1,
    };
    let value_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(3),
            Constraint::Length(right_w),
        ])
        .split(value_row);
    let hints = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(hint_height),
        width: inner.width,
        height: hint_height,
    };

    MoodDialogLayout {
        area,
        inner,
        bar: bar_chunks[1],
        value: value_chunks[1],
        hints,
    }
}

// ── Shared render helpers ─────────────────────────────────────────────────────

/// Render a single-line search/filter input styled as a form field: a normal
/// label followed by an underlined textarea spanning the rest of the row, so it
/// reads as an editable field whether or not text has been entered. The active
/// field is marked by the `>` prefix and the native bar cursor at the caret.
/// (No whole-field reversal: a reversed text selection would vanish inside it.)
fn render_search_field(
    frame: &mut Frame<'_>,
    rect: Rect,
    label: &str,
    value: &mut TextInput,
    focused: bool,
) {
    let prefix = format!("{}{label}", if focused { ">" } else { " " });
    let prefix_w = UnicodeWidthStr::width(prefix.as_str()) as u16;
    frame.render_widget(Paragraph::new(prefix), rect);

    // Leave one blank column before the dialog border so the underlined field
    // doesn't run flush against it.
    let field = Rect {
        x: rect.x + prefix_w,
        width: rect.width.saturating_sub(prefix_w).saturating_sub(1),
        ..rect
    };
    value.render_in(frame, field, focused);
}

fn render_lines_in_area<'a>(
    frame: &mut Frame<'_>,
    lines: impl IntoIterator<Item = Line<'a>>,
    inner: Rect,
) {
    for (y_offset, line) in lines.into_iter().enumerate() {
        let y = inner.y + y_offset as u16;
        if y >= inner.y + inner.height {
            break;
        }
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn render_separator(frame: &mut Frame<'_>, area: Rect) {
    let area = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    if area.width == 0 {
        return;
    }

    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize))
            .style(Style::default().add_modifier(Modifier::DIM)),
        Rect { height: 1, ..area },
    );
}

fn hint_content_area(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(1),
        ..area
    }
}

fn render_hint_line(frame: &mut Frame<'_>, hints: &[Hint], area: Rect) {
    let content = hint_content_area(area);
    frame.render_widget(Paragraph::new(hint_lines(hints, content.width)), content);
}

// ── Dialog draw functions ─────────────────────────────────────────────────────

/// The "Fetching weather and air quality…" modal shown while a save waits on its
/// background context fetch. The ellipsis cycles `.`→`..`→`...` every ~400ms;
/// dropped dots become spaces so the fixed-width box doesn't jitter.
pub(super) fn draw_fetching_environment(frame: &mut Frame<'_>, started: Instant) {
    let dots = (started.elapsed().as_millis() / 400 % 3) as usize + 1;
    let message = format!(
        "Fetching weather and air quality{}{}",
        ".".repeat(dots),
        " ".repeat(3 - dots)
    );
    // Border (2) + a space of padding each side (2) around the fixed-width text.
    let width = message.width() as u16 + 4;
    let area = centered_rect_fixed_size(width, 3, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(message).alignment(Alignment::Center), inner);
}

/// The `(height, message)` a confirm-delete dialog needs for `ctx`. The message is
/// centered at the top; the Delete/Cancel buttons occupy the last inner row.
fn confirm_delete_content(ctx: &DeleteContext) -> (u16, String) {
    match ctx {
        DeleteContext::Entry { has_body: true } => (5, "Move entry to trash?".to_string()),
        DeleteContext::Entry { has_body: false } => (5, "Permanently delete entry?".to_string()),
        DeleteContext::Journal {
            name,
            trash_count,
            delete_count,
        } => {
            let line2 = match (*trash_count, *delete_count) {
                (0, d) => format!("{d} entries deleted permanently"),
                (t, 0) => format!("{t} entries moved to trash"),
                (t, d) => format!("{t} entries → trash, {d} deleted"),
            };
            let display = journal_storage::journal_display_name(name);
            (6, format!("Delete journal '{display}'?\n{line2}"))
        }
    }
}

fn confirm_delete_area(frame_area: Rect, ctx: &DeleteContext) -> Rect {
    let (height, message) = confirm_delete_content(ctx);
    let width = CONFIRM_DIALOG_WIDTH.max(
        message
            .lines()
            .map(|l| l.len() as u16 + 4)
            .max()
            .unwrap_or(0),
    );
    super::centered_rect_fixed_size(width, height, frame_area)
}

/// The bordered inner rect of the confirm-delete dialog, so the mouse handler can
/// hit-test the buttons against the same geometry the draw uses.
pub(crate) fn confirm_delete_inner(frame_area: Rect, ctx: &DeleteContext) -> Rect {
    Block::default()
        .borders(Borders::ALL)
        .inner(confirm_delete_area(frame_area, ctx))
}

pub(super) fn draw_confirm_delete(frame: &mut Frame<'_>, ctx: &DeleteContext) {
    let (_, message) = confirm_delete_content(ctx);
    let area = confirm_delete_area(frame.area(), ctx);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Confirm Delete")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Message at the top, the Delete/Cancel buttons on the last inner row.
    for (i, line) in message.lines().enumerate() {
        let line_area = Rect {
            y: inner.y + i as u16,
            height: 1,
            ..inner
        };
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), line_area);
    }
    render_confirm_buttons(frame, inner, "Delete (y)", "Cancel (n)");
}

pub(super) fn draw_new_journal_input(frame: &mut Frame<'_>, input: &mut TextInput) {
    let area = super::centered_rect_fixed_size(NEW_JOURNAL_DIALOG_WIDTH, 5, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default().title("New Journal").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label = "Name: ";
    frame.render_widget(Paragraph::new(label), inner);
    let field = Rect {
        x: inner.x + label.len() as u16,
        y: inner.y,
        width: inner.width.saturating_sub(label.len() as u16),
        height: 1,
    };
    input.render_in(frame, field, true);

    let hint = Rect {
        y: inner.y + 2,
        height: 1,
        ..inner
    };
    frame.render_widget(Paragraph::new("Enter saves | Esc cancels"), hint);
}

pub(super) fn draw_edit_metadata_dialog(frame: &mut Frame<'_>, state: &mut EditMetadataState) {
    let layout = metadata_dialog_layout(frame.area(), state.filtered.len());
    let title = state.kind.title();

    let list_focused = state.focus == EditMetadataFocus::List;
    let input_focused = state.focus == EditMetadataFocus::Input;

    state.normalize_list_state();
    let list_lines = state.filtered.len();
    let max_visible = layout.list.height;
    let max_offset = list_lines.saturating_sub(max_visible as usize);
    let scroll = state.offset().min(max_offset);
    state.list.set_offset(scroll);

    let items: Vec<ListItem<'_>> = if state.filtered.is_empty() {
        let text = if state.input.is_empty() {
            format!(" (no {title} yet)").to_lowercase()
        } else {
            " (no matches)".to_string()
        };
        vec![ListItem::new(Line::from(text))]
    } else {
        state
            .filtered
            .iter()
            .map(|idx| {
                let (tag, freq) = &state.all_values[*idx];
                let checked = state.selected.iter().any(|t| t.eq_ignore_ascii_case(tag));
                let marker = if checked { "[x]" } else { "[ ]" };
                ListItem::new(Line::from(format!("{marker} {tag} ({freq})")))
            })
            .collect()
    };

    frame.render_widget(Clear, layout.area);
    frame.render_widget(
        Block::default()
            .title(format!(" Edit {title} "))
            .borders(Borders::ALL),
        layout.area,
    );
    render_lines_in_area(
        frame,
        [Line::from(Span::styled(
            format!(" {title} "),
            Style::default().add_modifier(Modifier::BOLD),
        ))],
        layout.inner,
    );
    render_separator(frame, layout.list_top_separator);
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);
    let mut render_state = list_state_for_render(
        state.selected_index(),
        scroll,
        layout.list.height,
        list_focused && !state.filtered.is_empty(),
    );
    frame.render_stateful_widget(list, layout.list, &mut render_state);
    render_separator(frame, layout.list_bottom_separator);
    render_search_field(
        frame,
        layout.input,
        "Search / new: ",
        &mut state.input,
        input_focused,
    );
    render_hint_line(
        frame,
        metadata_dialog_hints(state.focus, state.input.as_str().trim().is_empty()),
        layout.hints,
    );
    render_scrollbar_if_needed(frame, layout.area, list_lines, max_visible, scroll);
}

pub(super) fn draw_edit_mood_dialog(frame: &mut Frame<'_>, state: &EditMoodState) {
    let layout = mood_dialog_layout(frame.area());

    frame.render_widget(Clear, layout.area);
    frame.render_widget(
        Block::default().title(" Edit Mood ").borders(Borders::ALL),
        layout.area,
    );

    let right_label = " Blissful";

    // Empty spacer row
    let spacer_y = layout.inner.y;
    if spacer_y < layout.inner.y + layout.inner.height {
        frame.render_widget(
            Paragraph::new(Line::from("")),
            Rect {
                x: layout.inner.x,
                y: spacer_y,
                width: layout.inner.width,
                height: 1,
            },
        );
    }

    // Mood bar row
    let right_w = right_label.len() as u16;
    let bar_y = layout.inner.y + 1;
    if bar_y < layout.inner.y + layout.inner.height {
        let bar_rect = Rect {
            x: layout.inner.x,
            y: bar_y,
            width: layout.inner.width,
            height: 1,
        };
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(10),
                Constraint::Min(3),
                Constraint::Length(right_w),
            ])
            .split(bar_rect);
        frame.render_widget(Paragraph::new("Miserable "), chunks[0]);
        frame.render_widget(MoodBar::new(state.draft), chunks[1]);
        frame.render_widget(Paragraph::new(right_label), chunks[2]);
    }

    // Value number centred below the bar
    if layout.value.y < layout.inner.y + layout.inner.height {
        frame.render_widget(
            Paragraph::new(Line::from(format!("{}", state.draft))).alignment(Alignment::Center),
            layout.value,
        );
    }

    // Hint line
    if layout.hints.y < layout.inner.y + layout.inner.height {
        render_hint_line(frame, mood_dialog_hints(), layout.hints);
    }
}

pub(super) fn draw_edit_location_dialog(frame: &mut Frame<'_>, state: &mut EditLocationState) {
    let showing_candidates = state.showing_candidates();
    let labels = state.list_labels();
    let item_count = labels.len();
    // Size the dialog to the wrapped row span so multi-line rows aren't clipped.
    let layout = location_dialog_layout(frame.area(), location_list_rows(&labels));

    state.normalize_list_state();
    let max_visible = layout.list.height;
    let max_offset = item_count.saturating_sub(max_visible as usize);
    let scroll = state.offset().min(max_offset);
    state.list.set_offset(scroll);

    let list_focused = state.focus == EditLocationFocus::List;
    let dim = Style::default().add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    frame.render_widget(Clear, layout.area);
    frame.render_widget(
        Block::default()
            .title(" Edit Location ")
            .borders(Borders::ALL),
        layout.area,
    );

    render_lines_in_area(
        frame,
        [Line::from(Span::styled(" Location ", bold))],
        layout.title,
    );
    render_separator(frame, layout.title_separator);

    let query_focused = state.focus == EditLocationFocus::Query;
    let name_focused = state.focus == EditLocationFocus::Name;
    render_search_field(
        frame,
        layout.query,
        "Place / address / coords: ",
        &mut state.query,
        query_focused,
    );
    render_search_field(frame, layout.name, "Name: ", &mut state.name, name_focused);

    // Status line: reflects the in-flight/last lookup, or the resolved value.
    let status_line = match &state.status {
        LocationResolveStatus::Idle => {
            match state.resolved.as_ref().and_then(|l| l.display_label()) {
                Some(label) => Line::from(vec![Span::raw("  "), Span::styled(label, dim)]),
                None => Line::from(Span::styled(
                    "  Enter a place, address, or \"lat, lon\", then press enter",
                    dim,
                )),
            }
        }
        LocationResolveStatus::Resolving => Line::from(Span::styled("  Resolving…", dim)),
        LocationResolveStatus::NoMatch => Line::from(Span::styled("  No matches found", dim)),
        LocationResolveStatus::Error(error) => Line::from(Span::styled(format!("  {error}"), dim)),
        LocationResolveStatus::Resolved => {
            match state.resolved.as_ref().and_then(|l| l.display_label()) {
                Some(label) => Line::from(vec![Span::styled("  ✓ ", bold), Span::raw(label)]),
                None => Line::from(Span::styled("  Resolved", dim)),
            }
        }
    };
    render_lines_in_area(frame, [status_line], layout.status);

    render_separator(frame, layout.list_separator);

    let heading = if showing_candidates {
        " Matches "
    } else {
        " Recent places "
    };
    render_lines_in_area(
        frame,
        [Line::from(Span::styled(heading, bold))],
        layout.heading,
    );

    // Wrap long rows onto continuation lines (aligned under the first) instead of
    // clipping them.
    let items: Vec<ListItem<'_>> = if labels.is_empty() {
        let text = if showing_candidates {
            " (no matches)"
        } else {
            " (no saved places yet)"
        };
        vec![ListItem::new(Line::from(text))]
    } else {
        labels
            .iter()
            .map(|label| {
                let lines: Vec<Line<'static>> = location_row_lines(label)
                    .into_iter()
                    .map(|line| Line::from(format!(" {line}")))
                    .collect();
                ListItem::new(lines)
            })
            .collect()
    };

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);
    let mut render_state = list_state_for_render(
        state.selected_index(),
        scroll,
        layout.list.height,
        list_focused && item_count > 0,
    );
    frame.render_stateful_widget(list, layout.list, &mut render_state);

    render_hint_line(
        frame,
        location_dialog_hints(state.focus, state.query_looked_up),
        layout.hints,
    );
    render_scrollbar_if_needed(frame, layout.area, item_count, max_visible, scroll);
}

pub(super) fn draw_edit_feelings_dialog(frame: &mut Frame<'_>, state: &mut EditFeelingState) {
    let rows = state.visible_rows();
    let selected_line_count = feelings_selected_line_count(&state.selected);
    let layout = feelings_dialog_layout(frame.area(), rows.len(), selected_line_count);
    let filtering = state.is_filtering();
    let list_focused = state.focus == EditMetadataFocus::List;
    let input_focused = state.focus == EditMetadataFocus::Input;

    state.normalize_list_state();
    let list_lines = rows.len();
    let max_visible = layout.list.height;
    let max_offset = list_lines.saturating_sub(max_visible as usize);
    let scroll = state.offset().min(max_offset);
    state.list.set_offset(scroll);

    let items: Vec<ListItem<'_>> = if rows.is_empty() {
        vec![ListItem::new(Line::from(" (no matches)"))]
    } else {
        rows.iter()
            .map(|row| match *row {
                FeelingRow::Header { group } => {
                    let g = &state.groups[group];
                    let bold = Style::default().add_modifier(Modifier::BOLD);
                    // Disclosure marker trails the name so it never collides with the
                    // list's leading `>` selection cursor. ▾ open, ▸ collapsed.
                    let disclosure = if state.expanded[group] { '▾' } else { '▸' };
                    let mut spans = vec![Span::styled(g.name, bold)];
                    // The selected-count badge is lighter than the category name.
                    let selected = state.group_selected_count(group);
                    if selected > 0 {
                        spans.push(Span::raw(format!(" ({selected})")));
                    }
                    spans.push(Span::styled(format!(" {disclosure}"), bold));
                    ListItem::new(Line::from(spans))
                }
                FeelingRow::Feeling { group, feeling } => {
                    let g = &state.groups[group];
                    let name = g.feelings[feeling].name;
                    let checked = state.selected.iter().any(|value| value == name);
                    let marker = if checked { "[x]" } else { "[ ]" };
                    // While filtering the headers are hidden, so tag each match with
                    // its group for context.
                    let text = if filtering {
                        format!("{marker} {name}  ({})", g.name)
                    } else {
                        format!("   {marker} {name}")
                    };
                    ListItem::new(Line::from(text))
                }
            })
            .collect()
    };

    frame.render_widget(Clear, layout.area);
    frame.render_widget(
        Block::default()
            .title(" Edit Feelings ")
            .borders(Borders::ALL),
        layout.area,
    );
    render_lines_in_area(
        frame,
        [Line::from(Span::styled(
            " Feelings ",
            Style::default().add_modifier(Modifier::BOLD),
        ))],
        layout.inner,
    );
    render_separator(frame, layout.list_top_separator);
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);
    let mut render_state = list_state_for_render(
        state.selected_index(),
        scroll,
        layout.list.height,
        list_focused && !rows.is_empty(),
    );
    frame.render_stateful_widget(list, layout.list, &mut render_state);
    render_separator(frame, layout.list_bottom_separator);
    render_search_field(
        frame,
        layout.input,
        "Search: ",
        &mut state.input,
        input_focused,
    );

    // The summary lines get a leading pad space; the "Selected:" label is bold and
    // its continuation lines align under the first.
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let selected_rows = feelings_selected_rows(&state.selected);
    let summary: Vec<Line<'_>> = if selected_rows.is_empty() {
        vec![Line::from(vec![
            Span::raw(" "),
            Span::styled("Selected:", bold),
            Span::raw(" none"),
        ])]
    } else {
        selected_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let joined = row
                    .iter()
                    .map(|&i| state.selected[i].as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                if index == 0 {
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled("Selected:", bold),
                        Span::raw(format!(" {joined}")),
                    ])
                } else {
                    Line::from(format!(" {joined}"))
                }
            })
            .collect()
    };
    render_lines_in_area(frame, summary, layout.selected);
    render_hint_line(frame, feelings_dialog_hints(state.focus), layout.hints);
    render_scrollbar_if_needed(frame, layout.area, list_lines, max_visible, scroll);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_list_row_at_maps_wrapped_rows() {
        let long = "Some very long place name that keeps going ".repeat(3);
        let labels = vec!["First".to_string(), long.clone(), "Third".to_string()];
        let l0 = location_row_lines(&labels[0]).len();
        let l1 = location_row_lines(&labels[1]).len();
        assert!(l1 > 1, "long label should wrap onto multiple lines");

        let list = Rect {
            x: 0,
            y: 10,
            width: LOCATION_DIALOG_WIDTH,
            height: 40,
        };

        // First label's opening line.
        assert_eq!(location_list_row_at(list, &labels, 0, 10), Some(0));
        // Any continuation line of the wrapped label still maps to it.
        let last_of_second = 10 + (l0 + l1 - 1) as u16;
        assert_eq!(
            location_list_row_at(list, &labels, 0, last_of_second),
            Some(1)
        );
        // The third label starts right after the wrapped one.
        let third_start = 10 + (l0 + l1) as u16;
        assert_eq!(location_list_row_at(list, &labels, 0, third_start), Some(2));
        // A click past the last rendered row misses.
        let past = third_start + location_row_lines(&labels[2]).len() as u16;
        assert_eq!(location_list_row_at(list, &labels, 0, past), None);
        // Scrolled: the first visible row is the label at `offset`.
        assert_eq!(location_list_row_at(list, &labels, 1, 10), Some(1));
    }
}
