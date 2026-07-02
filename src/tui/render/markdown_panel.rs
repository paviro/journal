use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
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
        app.scroll.entry_view = draw_markdown_panel(
            frame,
            area,
            &title,
            &content,
            app.scroll.entry_view,
            app.focus == Focus::EntryView,
        );
    } else {
        let empty = Paragraph::new("No entry selected")
            .block(panel_block("Entry", app.focus == Focus::EntryView));
        frame.render_widget(empty, area);
    }
}

fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    content: &str,
    requested_scroll: u16,
    focused: bool,
) -> u16 {
    let block = panel_block(title, focused);
    let inner = panel_content_inner(block.inner(area));
    let width = inner.width.saturating_sub(1).max(1) as usize;
    let theme = markdown_theme();
    let renderer = MarkdownRenderer::new(width);
    let blocks = renderer.parse(content);
    let lines = renderer.render(&blocks, &theme);
    let line_count = lines.len();
    let scroll = viewer_scroll(requested_scroll, line_count, inner.height);

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);

    if line_count > inner.height as usize {
        let mut state = ScrollbarState::default()
            .content_length(line_count)
            .viewport_content_length(inner.height as usize)
            .position(scrollbar_position(scroll, line_count, inner.height));
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
