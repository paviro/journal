use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Wrap},
};

use unicode_width::UnicodeWidthStr;

use crate::tui::state::{
    DeleteContext, EditFeelingState, EditMetadataFocus, EditMetadataState, EditMoodState,
    FeelingRow, ListNav,
};
use crate::tui::surface::metadata_value_rows;

use super::{
    chrome::{Hint, HintId, hint_height, hint_lines, render_scrollbar_if_needed},
    list_state_for_render,
    markdown_panel::MoodBar,
};

// ── Hint text constants and helpers ──────────────────────────────────────────

const FEELINGS_DIALOG_LIST_HINTS: [Hint; 6] = [
    Hint::new("open", "→", HintId::FeelingsExpand),
    Hint::new("close", "←", HintId::FeelingsCollapse),
    Hint::new("toggle", "space", HintId::FeelingsToggle),
    Hint::new("search", "tab", HintId::FeelingsSwitchFocus),
    Hint::new("save", "enter", HintId::FeelingsSave),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const FEELINGS_DIALOG_INPUT_HINTS: [Hint; 3] = [
    Hint::new("list", "tab", HintId::FeelingsSwitchFocus),
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

const METADATA_DIALOG_INPUT_VALUE_HINTS: [Hint; 3] = [
    Hint::new("add", "enter", HintId::MetadataAddFromInput),
    Hint::new("list", "tab", HintId::MetadataSwitchFocus),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const LIST_DIALOG_WIDTH: u16 = 44;
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
    hint_height(&FEELINGS_DIALOG_LIST_HINTS, width).max(hint_height(&FEELINGS_DIALOG_INPUT_HINTS, width))
}

fn mood_dialog_hint_height(frame_area: Rect) -> u16 {
    hint_height(&MOOD_DIALOG_HINTS, dialog_hint_width(frame_area, 44))
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
/// label followed by an underlined value area that spans the rest of the row, so
/// it reads as an editable field whether or not text has been entered. The active
/// field is also reversed for a clear focus cue.
fn render_search_field(frame: &mut Frame<'_>, rect: Rect, label: &str, value: &str, focused: bool) {
    let prefix = format!("{}{label}", if focused { ">" } else { " " });
    let prefix_w = UnicodeWidthStr::width(prefix.as_str());
    // Leave one blank column before the dialog border so the underlined field
    // doesn't run flush against it.
    let field_w = (rect.width as usize).saturating_sub(prefix_w).saturating_sub(1);

    // Pad (or clip) the value so the underline always fills the field width.
    let value_w = UnicodeWidthStr::width(value);
    let field = if value_w < field_w {
        format!("{value}{}", " ".repeat(field_w - value_w))
    } else {
        value.chars().rev().take(field_w).collect::<Vec<_>>().into_iter().rev().collect()
    };

    let mut field_style = Style::default().add_modifier(Modifier::UNDERLINED);
    if focused {
        field_style = field_style.add_modifier(Modifier::REVERSED);
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(prefix),
            Span::styled(field, field_style),
        ])),
        rect,
    );
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

pub(super) fn draw_confirm_delete(frame: &mut Frame<'_>, ctx: &DeleteContext) {
    let (height, message) = match ctx {
        DeleteContext::Entry { has_body: true } => (5, "Move entry to trash?  y/n".to_string()),
        DeleteContext::Entry { has_body: false } => {
            (5, "Permanently delete entry?  y/n".to_string())
        }
        DeleteContext::Journal {
            name,
            trash_count,
            delete_count,
        } => {
            let line2 = match (*trash_count, *delete_count) {
                (0, d) => format!("{d} entries deleted permanently  y/n"),
                (t, 0) => format!("{t} entries moved to trash  y/n"),
                (t, d) => format!("{t} entries → trash, {d} deleted  y/n"),
            };
            let display = journal_storage::journal_display_name(name);
            (6, format!("Delete journal '{display}'?\n{line2}"))
        }
    };

    let dialog_width = (CONFIRM_DIALOG_WIDTH).max(
        message
            .lines()
            .map(|l| l.len() as u16 + 4)
            .max()
            .unwrap_or(0),
    );
    let area = super::centered_rect_fixed_size(dialog_width, height, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Confirm Delete")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<&str> = message.lines().collect();
    let start_y = inner.y + inner.height.saturating_sub(lines.len() as u16) / 2;
    for (i, line) in lines.iter().enumerate() {
        let line_area = Rect {
            y: start_y + i as u16,
            height: 1,
            ..inner
        };
        frame.render_widget(
            Paragraph::new(*line).alignment(Alignment::Center),
            line_area,
        );
    }
}

pub(super) fn draw_new_journal_input(frame: &mut Frame<'_>, input: &str) {
    let area = super::centered_rect_fixed_size(NEW_JOURNAL_DIALOG_WIDTH, 5, frame.area());
    frame.render_widget(Clear, area);
    let dialog = Paragraph::new(format!("Name: {input}\n\nEnter saves | Esc cancels"))
        .block(Block::default().title("New Journal").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(dialog, area);
}

pub(super) fn draw_edit_metadata_dialog(frame: &mut Frame<'_>, state: &mut EditMetadataState) {
    let layout = metadata_dialog_layout(frame.area(), state.filtered.len());
    let title = state.kind.title();
    let value_name = state.kind.value_name();

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
        &format!("Search / new {value_name}: "),
        &state.input,
        input_focused,
    );
    render_hint_line(
        frame,
        metadata_dialog_hints(state.focus, state.input.trim().is_empty()),
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
    render_search_field(frame, layout.input, "Search: ", &state.input, input_focused);

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
