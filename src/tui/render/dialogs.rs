use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Wrap},
};

use crate::tui::state::{EditFeelingState, EditMoodState, EditTagFocus, EditTagState};

use super::{
    chrome::{Hint, HintId, hint_height, hint_lines, render_scrollbar_if_needed},
    list_state_for_render,
    markdown_panel::MoodBar,
};

// ── Hint text constants and helpers ──────────────────────────────────────────

const FEELINGS_DIALOG_HINTS: [Hint; 3] = [
    Hint::new("toggle", "space", HintId::FeelingsToggle),
    Hint::new("save", "enter", HintId::FeelingsSave),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const MOOD_DIALOG_HINTS: [Hint; 5] = [
    Hint::new("decrease", "←", HintId::MoodDecrease),
    Hint::new("increase", "→", HintId::MoodIncrease),
    Hint::new("save", "enter", HintId::MoodSave),
    Hint::new("clear", "del", HintId::MoodClear),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const TAGS_DIALOG_LIST_HINTS: [Hint; 4] = [
    Hint::new("toggle", "space", HintId::TagsToggle),
    Hint::new("input", "tab", HintId::TagsSwitchFocus),
    Hint::new("save", "enter", HintId::TagsSave),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const TAGS_DIALOG_INPUT_HINTS: [Hint; 3] = [
    Hint::new("add", "enter", HintId::TagsAddFromInput),
    Hint::new("list", "tab", HintId::TagsSwitchFocus),
    Hint::new("cancel", "esc", HintId::CancelOverlay),
];

const LIST_DIALOG_WIDTH: u16 = 44;
const MOOD_DIALOG_WIDTH: u16 = 90;
const CONFIRM_DIALOG_WIDTH: u16 = 42;
const NEW_JOURNAL_DIALOG_WIDTH: u16 = 56;
const TAGS_DIALOG_MAX_VISIBLE_ROWS: u16 = 14;
const FEELINGS_DIALOG_MAX_VISIBLE_ROWS: u16 = 16;

pub(crate) fn feelings_dialog_hints() -> &'static [Hint] {
    &FEELINGS_DIALOG_HINTS
}

pub(crate) fn mood_dialog_hints() -> &'static [Hint] {
    &MOOD_DIALOG_HINTS
}

pub(crate) fn tags_dialog_hints(focus: EditTagFocus) -> &'static [Hint] {
    match focus {
        EditTagFocus::List => &TAGS_DIALOG_LIST_HINTS,
        EditTagFocus::Input => &TAGS_DIALOG_INPUT_HINTS,
    }
}

// ── Dialog area helpers (re-used by the mouse handler for hit-testing) ───────

pub(crate) fn tags_dialog_area(frame_area: Rect, filtered_len: usize) -> Rect {
    const FIXED: u16 = 7;
    let hint_height = tag_dialog_hint_height(frame_area);
    let visible = (filtered_len as u16).clamp(1, TAGS_DIALOG_MAX_VISIBLE_ROWS);
    let h = (FIXED + hint_height + visible).min(frame_area.height.saturating_sub(2));
    super::centered_rect_fixed_size(LIST_DIALOG_WIDTH, h, frame_area)
}

pub(crate) fn feelings_dialog_area(frame_area: Rect, all_len: usize) -> Rect {
    const FIXED: u16 = 5;
    let hint_height = feelings_dialog_hint_height(frame_area);
    let visible = (all_len as u16).min(FEELINGS_DIALOG_MAX_VISIBLE_ROWS);
    let h = (FIXED + hint_height + visible).min(frame_area.height.saturating_sub(2));
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
    hint_height(&TAGS_DIALOG_LIST_HINTS, width).max(hint_height(&TAGS_DIALOG_INPUT_HINTS, width))
}

fn feelings_dialog_hint_height(frame_area: Rect) -> u16 {
    hint_height(
        &FEELINGS_DIALOG_HINTS,
        dialog_hint_width(frame_area, LIST_DIALOG_WIDTH),
    )
}

fn mood_dialog_hint_height(frame_area: Rect) -> u16 {
    hint_height(&MOOD_DIALOG_HINTS, dialog_hint_width(frame_area, 44))
}

#[derive(Clone, Copy)]
pub(crate) struct TagsDialogLayout {
    pub(crate) area: Rect,
    pub(crate) inner: Rect,
    pub(crate) list_top_separator: Rect,
    pub(crate) list: Rect,
    pub(crate) list_bottom_separator: Rect,
    pub(crate) input: Rect,
    pub(crate) hints: Rect,
}

pub(crate) fn tags_dialog_layout(frame_area: Rect, filtered_len: usize) -> TagsDialogLayout {
    let area = tags_dialog_area(frame_area, filtered_len);
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

    TagsDialogLayout {
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
    pub(crate) hints: Rect,
}

pub(crate) fn feelings_dialog_layout(frame_area: Rect, all_len: usize) -> FeelingsDialogLayout {
    let area = feelings_dialog_area(frame_area, all_len);
    let inner = super::panel_inner(area);
    let hint_height = feelings_dialog_hint_height(frame_area);
    let list = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(3 + hint_height),
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

fn render_dialog_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    list_lines: u16,
    max_visible: u16,
    scroll: u16,
) {
    render_scrollbar_if_needed(frame, area, list_lines as usize, max_visible, scroll);
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

pub(super) fn draw_confirm_delete(frame: &mut Frame<'_>) {
    let area = super::centered_rect_fixed_size(CONFIRM_DIALOG_WIDTH, 5, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Confirm Delete")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let message_area = Rect {
        y: inner.y + inner.height.saturating_sub(1) / 2,
        height: inner.height.min(1),
        ..inner
    };
    frame.render_widget(
        Paragraph::new("Move selected file to trash? y/n").alignment(Alignment::Center),
        message_area,
    );
}

pub(super) fn draw_new_journal_input(frame: &mut Frame<'_>, input: &str) {
    let area = super::centered_rect_fixed_size(NEW_JOURNAL_DIALOG_WIDTH, 5, frame.area());
    frame.render_widget(Clear, area);
    let dialog = Paragraph::new(format!("Name: {input}\n\nEnter saves | Esc cancels"))
        .block(Block::default().title("New Journal").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(dialog, area);
}

pub(super) fn draw_edit_tags_dialog(frame: &mut Frame<'_>, state: &mut EditTagState) {
    let layout = tags_dialog_layout(frame.area(), state.filtered.len());

    let list_focused = state.focus == EditTagFocus::List;
    let input_focused = state.focus == EditTagFocus::Input;

    state.normalize_list_state();
    let list_lines = state.filtered.len();
    let max_visible = layout.list.height;
    let max_offset = list_lines.saturating_sub(max_visible as usize);
    let scroll = state.offset().min(max_offset);
    *state.list_state.offset_mut() = scroll;

    let items: Vec<ListItem<'_>> = if state.filtered.is_empty() {
        let text = if state.input.is_empty() {
            " (no tags yet)"
        } else {
            " (no matches)"
        };
        vec![ListItem::new(Line::from(text))]
    } else {
        state
            .filtered
            .iter()
            .map(|idx| {
                let (tag, freq) = &state.all_tags[*idx];
                let checked = state.selected.iter().any(|t| t.eq_ignore_ascii_case(tag));
                let marker = if checked { "[x]" } else { "[ ]" };
                ListItem::new(Line::from(format!("{marker} {tag} ({freq})")))
            })
            .collect()
    };

    let input_text = format!(
        "{}Search / new tag: {}",
        if input_focused { ">" } else { " " },
        state.input
    );
    let input_style = if input_focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    frame.render_widget(Clear, layout.area);
    frame.render_widget(
        Block::default().title(" Edit Tags ").borders(Borders::ALL),
        layout.area,
    );
    render_lines_in_area(
        frame,
        [Line::from(Span::styled(
            " Tags ",
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
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(input_text, input_style))),
        layout.input,
    );
    render_hint_line(frame, tags_dialog_hints(state.focus), layout.hints);
    render_dialog_scrollbar(
        frame,
        layout.area,
        list_lines as u16,
        max_visible,
        scroll as u16,
    );
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
    let layout = feelings_dialog_layout(frame.area(), state.all_feelings.len());

    state.normalize_list_state();
    let list_lines = state.all_feelings.len();
    let max_visible = layout.list.height;
    let max_offset = list_lines.saturating_sub(max_visible as usize);
    let scroll = state.offset().min(max_offset);
    *state.list_state.offset_mut() = scroll;

    let items: Vec<ListItem<'_>> = state
        .all_feelings
        .iter()
        .map(|feeling| {
            let checked = state.selected.iter().any(|value| value == feeling);
            let marker = if checked { "[x]" } else { "[ ]" };
            ListItem::new(Line::from(format!("{marker} {feeling}")))
        })
        .collect();

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
    let mut render_state =
        list_state_for_render(state.selected_index(), scroll, layout.list.height, true);
    frame.render_stateful_widget(list, layout.list, &mut render_state);
    render_separator(frame, layout.list_bottom_separator);
    render_hint_line(frame, feelings_dialog_hints(), layout.hints);
    render_dialog_scrollbar(
        frame,
        layout.area,
        list_lines as u16,
        max_visible,
        scroll as u16,
    );
}
