use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, ScrollbarState, Wrap},
};

use crate::tui::{
    render::{markdown_panel::MoodBar, render_vertical_scrollbar, scrollbar_position},
    state::{EditFeelingState, EditMoodState, EditTagFocus, EditTagState},
};

// ── Hint text constants and helpers ──────────────────────────────────────────

pub(crate) const FEELINGS_HINT: &str = " toggle (space) | save (enter) | cancel (esc)";
pub(crate) const MOOD_HINT: &str =
    " decrease (←) | increase (→) | save (enter) | clear (del) | cancel (esc)";

pub(crate) fn tags_dialog_hint(focus: EditTagFocus) -> &'static str {
    match focus {
        EditTagFocus::List => " toggle (space) | input (tab) | save (enter) | cancel (esc)",
        EditTagFocus::Input => " add (enter) | list (tab) | cancel (esc)",
    }
}

// ── Dialog area helpers (re-used by the mouse handler for hit-testing) ───────

pub(crate) fn tags_dialog_area(frame_area: Rect, filtered_len: usize) -> Rect {
    const FIXED: u16 = 6;
    let visible = (filtered_len as u16).min(10).max(1);
    let h = (FIXED + visible + 2).min(frame_area.height.saturating_sub(2));
    super::centered_rect_fixed_height(40, h, frame_area)
}

pub(crate) fn feelings_dialog_area(frame_area: Rect, all_len: usize) -> Rect {
    let visible = (all_len as u16).min(11);
    let h = (4 + visible + 2).min(frame_area.height.saturating_sub(2));
    super::centered_rect_fixed_height(40, h, frame_area)
}

pub(crate) fn mood_dialog_area(frame_area: Rect) -> Rect {
    super::centered_rect_fixed_height(44, 8, frame_area)
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
    if list_lines > max_visible {
        let mut state = ScrollbarState::default()
            .content_length(list_lines as usize)
            .viewport_content_length(max_visible as usize)
            .position(scrollbar_position(scroll, list_lines as usize, max_visible));
        render_vertical_scrollbar(frame, area, &mut state);
    }
}

// ── Dialog draw functions ─────────────────────────────────────────────────────

pub(super) fn draw_confirm_delete(frame: &mut Frame<'_>) {
    let area = super::centered_rect(50, 20, frame.area());
    frame.render_widget(Clear, area);
    let dialog = Paragraph::new("Move selected file to trash? y/n")
        .block(
            Block::default()
                .title("Confirm Delete")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(dialog, area);
}

pub(super) fn draw_new_journal_input(frame: &mut Frame<'_>, input: &str) {
    let area = super::centered_rect(60, 20, frame.area());
    frame.render_widget(Clear, area);
    let dialog = Paragraph::new(format!("Name: {input}\n\nEnter saves | Esc cancels"))
        .block(Block::default().title("New Journal").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(dialog, area);
}

pub(super) fn draw_edit_tags_dialog(frame: &mut Frame<'_>, state: &mut EditTagState) {
    let area = tags_dialog_area(frame.area(), state.filtered.len());
    let inner = super::panel_inner(area);

    let list_focused = state.focus == EditTagFocus::List;
    let input_focused = state.focus == EditTagFocus::Input;

    let list_lines = state.filtered.len() as u16;
    const TAG_DIALOG_FIXED_ROWS: u16 = 6;
    let max_visible = inner.height.saturating_sub(TAG_DIALOG_FIXED_ROWS);

    // Keep the cursor visible
    if state.cursor < state.scroll as usize {
        state.scroll = state.cursor as u16;
    }
    let last_visible = state.scroll as usize + max_visible as usize;
    if state.cursor >= last_visible.saturating_sub(1) && list_lines > max_visible {
        state.scroll = state
            .cursor
            .saturating_add(1)
            .saturating_sub(max_visible as usize)
            .min(list_lines.saturating_sub(max_visible) as usize) as u16;
    }

    let scroll = state.scroll.min(list_lines.saturating_sub(max_visible));
    let end = (scroll + max_visible).min(list_lines);

    let mut lines: Vec<Line<'_>> = Vec::new();

    lines.push(Line::from(Span::styled(
        " Tags ",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    if state.filtered.is_empty() {
        lines.push(Line::from(if state.input.is_empty() {
            " (no tags yet)"
        } else {
            " (no matches)"
        }));
    } else {
        for i in scroll..end {
            let idx = state.filtered[i as usize];
            let (tag, freq) = &state.all_tags[idx];
            let checked = state.selected.iter().any(|t| t.eq_ignore_ascii_case(tag));
            let marker = if checked { "[x]" } else { "[ ]" };
            let is_cursor = i as usize == state.cursor && list_focused;
            let prefix = if is_cursor { ">" } else { " " };
            let text = format!("{prefix}{marker} {tag} ({freq})");
            let style = if is_cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
    }

    lines.push(Line::from(""));

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
    lines.push(Line::from(Span::styled(input_text, input_style)));
    lines.push(Line::from(""));
    lines.push(Line::from(tags_dialog_hint(state.focus)));

    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().title(" Edit Tags ").borders(Borders::ALL), area);
    render_lines_in_area(frame, lines, inner);
    render_dialog_scrollbar(frame, area, list_lines, max_visible, scroll);
}

pub(super) fn draw_edit_mood_dialog(frame: &mut Frame<'_>, state: &EditMoodState) {
    let area = mood_dialog_area(frame.area());
    let inner = super::panel_inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().title(" Edit Mood ").borders(Borders::ALL), area);

    let right_label = " Blissful";

    // Empty spacer row
    let spacer_y = inner.y;
    if spacer_y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(Line::from("")),
            Rect { x: inner.x, y: spacer_y, width: inner.width, height: 1 },
        );
    }

    // Mood bar row
    let right_w = right_label.len() as u16;
    let bar_y = inner.y + 1;
    if bar_y < inner.y + inner.height {
        let bar_rect = Rect { x: inner.x, y: bar_y, width: inner.width, height: 1 };
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
    let value_y = inner.y + 3;
    if value_y < inner.y + inner.height {
        let value_rect = Rect { x: inner.x, y: value_y, width: inner.width, height: 1 };
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(10),
                Constraint::Min(3),
                Constraint::Length(right_w),
            ])
            .split(value_rect);
        frame.render_widget(
            Paragraph::new(Line::from(format!("{}", state.draft))).alignment(Alignment::Center),
            chunks[1],
        );
    }

    // Hint line
    let hint_y = inner.y + inner.height.saturating_sub(1);
    if hint_y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(Line::from(MOOD_HINT)),
            Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 },
        );
    }
}

pub(super) fn draw_edit_feelings_dialog(frame: &mut Frame<'_>, state: &mut EditFeelingState) {
    let area = feelings_dialog_area(frame.area(), state.all_feelings.len());
    let inner = super::panel_inner(area);

    let list_lines = state.all_feelings.len() as u16;
    let max_visible = inner.height.saturating_sub(4);

    if state.cursor < state.scroll as usize {
        state.scroll = state.cursor as u16;
    }
    let last_visible = state.scroll as usize + max_visible as usize;
    if state.cursor >= last_visible.saturating_sub(1) && list_lines > max_visible {
        state.scroll = state
            .cursor
            .saturating_add(1)
            .saturating_sub(max_visible as usize)
            .min(list_lines.saturating_sub(max_visible) as usize) as u16;
    }

    let scroll = state.scroll.min(list_lines.saturating_sub(max_visible));
    let end = (scroll + max_visible).min(list_lines);

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " Feelings ",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for i in scroll..end {
        let feeling = &state.all_feelings[i as usize];
        let checked = state.selected.iter().any(|value| value == feeling);
        let marker = if checked { "[x]" } else { "[ ]" };
        let is_cursor = i as usize == state.cursor;
        let prefix = if is_cursor { ">" } else { " " };
        let text = format!("{prefix}{marker} {feeling}");
        let style = if is_cursor {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(FEELINGS_HINT));

    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().title(" Edit Feelings ").borders(Borders::ALL),
        area,
    );
    render_lines_in_area(frame, lines, inner);
    render_dialog_scrollbar(frame, area, list_lines, max_visible, scroll);
}
