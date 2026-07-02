use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, ScrollbarState, Widget},
};
use ratatui_029::{
    layout::Alignment as MarkdownAlignment,
    style::{Color as MarkdownColor, Modifier as MarkdownModifier, Style as MarkdownStyle},
    text::{Line as MarkdownLine, Span as MarkdownSpan},
};
use ratatui_markdown::{
    markdown::MarkdownRenderer,
    theme::{CodeColors, ThemeConfig},
};

use crate::tui::{
    app::{App, Focus},
    render::{
        panel_block, panel_content_inner, render_vertical_scrollbar, scrollbar_position,
        viewer_scroll,
    },
};

pub(crate) fn draw_selected_entry_view(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_entry_view() {
        let tags = app.selected_entry_tags();
        let feelings = app.selected_entry_feelings();
        let mood = app.selected_entry_mood();
        app.scroll.entry_view = draw_markdown_panel(
            frame,
            area,
            &title,
            &content,
            EntryMetadata {
                tags: &tags,
                feelings: &feelings,
                mood,
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
    let metadata_height = metadata_section_height(metadata.tags, metadata.feelings, metadata.mood);
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
    let lines = adapt_markdown_lines(renderer.render(&blocks, &theme));
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
        render_vertical_scrollbar(frame, area, &mut state);
    }

    scroll
}

#[derive(Clone, Copy)]
struct EntryMetadata<'a> {
    tags: &'a [String],
    feelings: &'a [String],
    mood: Option<i8>,
}

fn metadata_section_height(tags: &[String], feelings: &[String], mood: Option<i8>) -> u16 {
    let rows = mood.is_some() as u16 + (!feelings.is_empty()) as u16 + (!tags.is_empty()) as u16;
    if rows == 0 { 0 } else { 1 + rows }
}

fn draw_metadata_section(frame: &mut Frame<'_>, area: Rect, metadata: EntryMetadata<'_>) {
    let sep = "─".repeat(area.width.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(sep).style(Style::default().add_modifier(Modifier::DIM)),
        Rect { height: 1, ..area },
    );

    let mut y = area.y + 1;
    if let Some(score) = metadata.mood {
        let mood_rect = Rect {
            y,
            height: 1,
            ..area
        };
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(10), // "Miserable "
                Constraint::Min(4),
                Constraint::Length(9), // " Blissful"
            ])
            .split(mood_rect);
        frame.render_widget(Paragraph::new("Miserable "), chunks[0]);
        frame.render_widget(MoodBar::new(score), chunks[1]);
        frame.render_widget(Paragraph::new(" Blissful"), chunks[2]);
        y = y.saturating_add(1);
    }
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

pub(crate) struct MoodBar {
    score: i8,
}

impl MoodBar {
    pub(crate) fn new(score: i8) -> Self {
        Self { score }
    }
}

impl Widget for MoodBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = area.width as usize;
        if width < 3 {
            return;
        }

        let center = width / 2;
        let lw = center;
        let rw = width - center - 1;

        let neg = self.score.min(0).unsigned_abs() as usize;
        let pos = self.score.max(0) as usize;

        let filled_left = if lw > 0 && neg > 0 {
            (neg * lw / 5).max(1).min(lw)
        } else {
            0
        };
        let filled_right = if rw > 0 && pos > 0 {
            (pos * rw / 5).max(1).min(rw)
        } else {
            0
        };

        let bold = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().add_modifier(Modifier::DIM);

        for i in 0..width {
            let x = area.x + i as u16;
            let Some(cell) = buf.cell_mut((x, area.y)) else {
                continue;
            };
            if i == center {
                cell.set_symbol(if self.score == 0 { "┃" } else { "│" });
                cell.set_style(Style::default());
            } else if i < center {
                let dist = center - i;
                if dist <= filled_left {
                    cell.set_symbol("━");
                    cell.set_style(bold);
                } else {
                    cell.set_symbol("─");
                    cell.set_style(dim);
                }
            } else {
                let dist = i - center;
                if dist <= filled_right {
                    cell.set_symbol("━");
                    cell.set_style(bold);
                } else {
                    cell.set_symbol("─");
                    cell.set_style(dim);
                }
            }
        }
    }
}

pub(crate) fn markdown_theme() -> ThemeConfig {
    let foreground = MarkdownColor::Reset;
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
        comment: MarkdownColor::Reset,
        keyword: MarkdownColor::Reset,
        string: MarkdownColor::Reset,
        string_escape: MarkdownColor::Reset,
        number: MarkdownColor::Reset,
        constant: MarkdownColor::Reset,
        function: MarkdownColor::Reset,
        r#type: MarkdownColor::Reset,
        variable: MarkdownColor::Reset,
        property: MarkdownColor::Reset,
        operator: MarkdownColor::Reset,
        punctuation: MarkdownColor::Reset,
        attribute: MarkdownColor::Reset,
        tag: MarkdownColor::Reset,
        label: MarkdownColor::Reset,
        error: MarkdownColor::Reset,
    }
}

fn adapt_markdown_lines(lines: Vec<MarkdownLine<'_>>) -> Vec<Line<'_>> {
    lines.into_iter().map(adapt_markdown_line).collect()
}

fn adapt_markdown_line(line: MarkdownLine<'_>) -> Line<'_> {
    Line {
        style: adapt_markdown_style(line.style),
        alignment: line.alignment.map(adapt_markdown_alignment),
        spans: line.spans.into_iter().map(adapt_markdown_span).collect(),
    }
}

fn adapt_markdown_span(span: MarkdownSpan<'_>) -> Span<'_> {
    Span {
        style: adapt_markdown_style(span.style),
        content: span.content,
    }
}

fn adapt_markdown_style(markdown_style: MarkdownStyle) -> Style {
    let mut style = Style::default()
        .add_modifier(adapt_markdown_modifier(markdown_style.add_modifier))
        .remove_modifier(adapt_markdown_modifier(markdown_style.sub_modifier));

    if let Some(fg) = markdown_style.fg {
        style = style.fg(adapt_markdown_color(fg));
    }
    if let Some(bg) = markdown_style.bg {
        style = style.bg(adapt_markdown_color(bg));
    }
    if let Some(underline_color) = markdown_style.underline_color {
        style = style.underline_color(adapt_markdown_color(underline_color));
    }

    style
}

fn adapt_markdown_modifier(modifier: MarkdownModifier) -> Modifier {
    Modifier::from_bits_truncate(modifier.bits())
}

fn adapt_markdown_alignment(alignment: MarkdownAlignment) -> Alignment {
    match alignment {
        MarkdownAlignment::Left => Alignment::Left,
        MarkdownAlignment::Center => Alignment::Center,
        MarkdownAlignment::Right => Alignment::Right,
    }
}

fn adapt_markdown_color(color: MarkdownColor) -> Color {
    match color {
        MarkdownColor::Reset => Color::Reset,
        MarkdownColor::Black => Color::Black,
        MarkdownColor::Red => Color::Red,
        MarkdownColor::Green => Color::Green,
        MarkdownColor::Yellow => Color::Yellow,
        MarkdownColor::Blue => Color::Blue,
        MarkdownColor::Magenta => Color::Magenta,
        MarkdownColor::Cyan => Color::Cyan,
        MarkdownColor::Gray => Color::Gray,
        MarkdownColor::DarkGray => Color::DarkGray,
        MarkdownColor::LightRed => Color::LightRed,
        MarkdownColor::LightGreen => Color::LightGreen,
        MarkdownColor::LightYellow => Color::LightYellow,
        MarkdownColor::LightBlue => Color::LightBlue,
        MarkdownColor::LightMagenta => Color::LightMagenta,
        MarkdownColor::LightCyan => Color::LightCyan,
        MarkdownColor::White => Color::White,
        MarkdownColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        MarkdownColor::Indexed(index) => Color::Indexed(index),
    }
}
