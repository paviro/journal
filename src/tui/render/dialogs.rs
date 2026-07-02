use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, ScrollbarState, Wrap},
};

use crate::tui::{
    render::{markdown_panel::MoodBar, render_vertical_scrollbar, scrollbar_position},
    state::{EditFeelingState, EditMoodState, EditTagFocus, EditTagState},
};

fn centered_rect_with_height(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

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
    let area_height = frame.area().height;

    // Content: header + blank + N tags + input + gap + help
    let max_tags_visible = 10u16;
    let visible_tags = (state.filtered.len() as u16).min(max_tags_visible);
    let inner_height = 5u16 + visible_tags; // header(1) + blank(1) + tags + input(1) + gap(1) + help(1)
    let dialog_height = (inner_height + 2).min(area_height.saturating_sub(2)); // + borders, cap at terminal

    let area = centered_rect_with_height(40, dialog_height, frame.area());
    frame.render_widget(Clear, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let list_focused = state.focus == EditTagFocus::List;
    let input_focused = state.focus == EditTagFocus::Input;

    let list_lines = state.filtered.len() as u16;
    let max_visible = inner.height.saturating_sub(3); // reserve 2 for input + 1 gap

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

    // Tag list header
    lines.push(Line::from(Span::styled(
        " Tags ",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Tag rows
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

    // Input row
    let input_text = format!(
        "{}Search / new tag: {}",
        if input_focused { "> " } else { "  " },
        state.input
    );
    let input_style = if input_focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    lines.push(Line::from(Span::styled(input_text, input_style)));
    lines.push(Line::from(""));

    // Instructions
    let help = if list_focused {
        " toggle (space) | input (tab) | save (enter) | cancel (esc)"
    } else {
        " add (enter) | list (tab) | cancel (esc)"
    };
    lines.push(Line::from(help));

    let block = Block::default().title(" Edit Tags ").borders(Borders::ALL);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    // Render lines inside the inner area
    for (y_offset, line) in lines.into_iter().enumerate() {
        let y = inner.y + y_offset as u16;
        if y >= inner.y + inner.height {
            break;
        }
        frame.render_widget(
            Paragraph::new(line).style(Style::default()),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
    }

    if list_lines > max_visible {
        let mut state = ScrollbarState::default()
            .content_length(list_lines as usize)
            .viewport_content_length(max_visible as usize)
            .position(scrollbar_position(scroll, list_lines as usize, max_visible));
        render_vertical_scrollbar(frame, area, &mut state);
    }
}

pub(super) fn draw_edit_mood_dialog(frame: &mut Frame<'_>, state: &EditMoodState) {
    let area = centered_rect_with_height(44, 7, frame.area());
    frame.render_widget(Clear, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let block = Block::default().title(" Edit Mood ").borders(Borders::ALL);
    frame.render_widget(block, area);

    let right_label = " Blissful";

    // Render non-bar lines
    for (y_offset, text) in [
        (0u16, ""),
        (2u16, ""),
        (
            3u16,
            "decrease (←) | increase (→) | save (enter) | clear (del) | cancel (esc)",
        ),
    ] {
        let y = inner.y + y_offset;
        if y < inner.y + inner.height {
            frame.render_widget(
                Paragraph::new(Line::from(text)),
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }

    // Render bar line with MoodBar widget
    let bar_y = inner.y + 1;
    if bar_y < inner.y + inner.height {
        let bar_rect = Rect {
            x: inner.x,
            y: bar_y,
            width: inner.width,
            height: 1,
        };
        let right_w = right_label.len() as u16;
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(10), // "Miserable "
                Constraint::Min(3),
                Constraint::Length(right_w),
            ])
            .split(bar_rect);
        frame.render_widget(Paragraph::new("Miserable "), chunks[0]);
        frame.render_widget(MoodBar::new(state.draft), chunks[1]);
        frame.render_widget(Paragraph::new(right_label), chunks[2]);
    }
}

pub(super) fn draw_edit_feelings_dialog(frame: &mut Frame<'_>, state: &mut EditFeelingState) {
    let area_height = frame.area().height;
    let max_feelings_visible = 12u16;
    let visible_feelings = (state.all_feelings.len() as u16).min(max_feelings_visible);
    let inner_height = 3u16 + visible_feelings;
    let dialog_height = (inner_height + 2).min(area_height.saturating_sub(2));

    let area = centered_rect_with_height(40, dialog_height, frame.area());
    frame.render_widget(Clear, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let list_lines = state.all_feelings.len() as u16;
    let max_visible = inner.height.saturating_sub(3);

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

    lines.push(Line::from(" toggle (space) | save (enter) | cancel (esc)"));

    let block = Block::default()
        .title(" Edit Feelings ")
        .borders(Borders::ALL);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    for (y_offset, line) in lines.into_iter().enumerate() {
        let y = inner.y + y_offset as u16;
        if y >= inner.y + inner.height {
            break;
        }
        frame.render_widget(
            Paragraph::new(line).style(Style::default()),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
    }

    if list_lines > max_visible {
        let mut state = ScrollbarState::default()
            .content_length(list_lines as usize)
            .viewport_content_length(max_visible as usize)
            .position(scrollbar_position(scroll, list_lines as usize, max_visible));
        render_vertical_scrollbar(frame, area, &mut state);
    }
}
