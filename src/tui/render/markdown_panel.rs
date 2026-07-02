use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use ratatui_markdown::{
    markdown::MarkdownRenderer,
    theme::{CodeColors, ThemeConfig},
};

use crate::tui::{
    app::{App, Focus},
    render::{panel_block, panel_content_inner, scrollbar_position, viewer_scroll},
};

pub(crate) fn draw_selected_entry_view(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_entry_view() {
        let tags = app.selected_entry_tags();
        let feelings = app.selected_entry_feelings();
        app.scroll.entry_view = draw_markdown_panel(
            frame,
            area,
            &title,
            &content,
            EntryMetadata {
                tags: &tags,
                feelings: &feelings,
            },
            app.scroll.entry_view,
            app.focus == Focus::EntryView,
        );
    } else {
        let empty = Paragraph::new("No entry selected").block(panel_block(
            "Entry",
            app.focus == Focus::EntryView,
            None,
        ));
        frame.render_widget(empty, area);
    }
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    content: &str,
    metadata: EntryMetadata<'_>,
    requested_scroll: u16,
    focused: bool,
) -> u16 {
    let wc = word_count(content);
    let block = panel_block(title, focused, Some(wc));
    let inner = panel_content_inner(block.inner(area));
    let metadata_height = metadata_section_height(metadata.tags, metadata.feelings);
    let show_metadata = metadata_height > 0 && inner.height > metadata_height;

    let (content_rect, metadata_rect) = if show_metadata {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(metadata_height)])
            .split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

    let width = content_rect.width.saturating_sub(1).max(1) as usize;
    let theme = markdown_theme();
    let renderer = MarkdownRenderer::new(width);
    let blocks = renderer.parse(content);
    let lines = renderer.render(&blocks, &theme);
    let line_count = lines.len();
    let scroll = viewer_scroll(requested_scroll, line_count, content_rect.height);

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content_rect);

    if let Some(metadata_rect) = metadata_rect {
        draw_metadata_section(frame, metadata_rect, metadata);
    }

    if line_count > content_rect.height as usize {
        let mut state = ScrollbarState::default()
            .content_length(line_count)
            .viewport_content_length(content_rect.height as usize)
            .position(scrollbar_position(scroll, line_count, content_rect.height));
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .track_symbol(Some("|"))
            .thumb_symbol("#")
            .style(Style::default().add_modifier(Modifier::DIM))
            .thumb_style(Style::default().add_modifier(Modifier::BOLD));
        frame.render_stateful_widget(scrollbar, area, &mut state);
    }

    scroll
}

#[derive(Clone, Copy)]
struct EntryMetadata<'a> {
    tags: &'a [String],
    feelings: &'a [String],
}

fn metadata_section_height(tags: &[String], feelings: &[String]) -> u16 {
    let rows = (!feelings.is_empty()) as u16 + (!tags.is_empty()) as u16;
    if rows == 0 { 0 } else { 1 + rows }
}

fn draw_metadata_section(frame: &mut Frame<'_>, area: Rect, metadata: EntryMetadata<'_>) {
    let sep = "─".repeat(area.width.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(sep).style(Style::default().add_modifier(Modifier::DIM)),
        Rect { height: 1, ..area },
    );

    let mut y = area.y + 1;
    if !metadata.feelings.is_empty() {
        let feelings_line = Line::from(vec![
            Span::styled("Feelings: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(metadata.feelings.join(" | ")),
        ]);
        frame.render_widget(
            Paragraph::new(feelings_line),
            Rect {
                y,
                height: 1,
                ..area
            },
        );
        y = y.saturating_add(1);
    }

    if !metadata.tags.is_empty() {
        let tags_line = Line::from(vec![
            Span::styled("Tags: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(metadata.tags.join(" | ")),
        ]);
        frame.render_widget(
            Paragraph::new(tags_line),
            Rect {
                y,
                height: 1,
                ..area
            },
        );
    }
}

pub(crate) fn markdown_theme() -> ThemeConfig {
    let foreground = Color::Reset;
    ThemeConfig::builder()
        .with_text_color(foreground)
        .with_muted_text_color(foreground)
        .with_primary_color(foreground)
        .with_popup_selected_background(foreground)
        .with_border_color(foreground)
        .with_focused_border_color(foreground)
        .with_secondary_color(foreground)
        .with_info_color(foreground)
        .with_json_key_color(foreground)
        .with_json_string_color(foreground)
        .with_json_number_color(foreground)
        .with_json_bool_color(foreground)
        .with_json_null_color(foreground)
        .with_accent_yellow(foreground)
        .with_code_colors(reset_code_colors())
        .build()
}

fn reset_code_colors() -> CodeColors {
    CodeColors {
        comment: Color::Reset,
        keyword: Color::Reset,
        string: Color::Reset,
        string_escape: Color::Reset,
        number: Color::Reset,
        constant: Color::Reset,
        function: Color::Reset,
        r#type: Color::Reset,
        variable: Color::Reset,
        property: Color::Reset,
        operator: Color::Reset,
        punctuation: Color::Reset,
        attribute: Color::Reset,
        tag: Color::Reset,
        label: Color::Reset,
        error: Color::Reset,
    }
}
