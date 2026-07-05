use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};
use ratatui_029::{
    layout::Alignment as MarkdownAlignment,
    style::{Color as MarkdownColor, Modifier as MarkdownModifier, Style as MarkdownStyle},
    text::{Line as MarkdownLine, Span as MarkdownSpan},
};
use ratatui_markdown::{
    markdown::{MarkdownBlock, MarkdownRenderer},
    theme::{CodeColors, ThemeConfig},
};

use std::path::Path;

use crate::tui::{
    app::{App, EntryViewImageHits, Focus},
    image::{digit_for_image, sole_image_ref},
    render::{
        count_label, entry_metadata_layout, panel_block, render_scrollbar_if_needed, viewer_scroll,
    },
    surface::{
        EntryMetadataLayout, EntryMetadataValues, MetadataRowLayout, PanelGeometry,
        metadata_value_rows,
    },
};

const SCROLLING_METADATA_ENTRY_VIEW_HEIGHT_CUTOFF: u16 = 20;

pub(crate) fn draw_selected_entry_view(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_entry_view() {
        let tags = app.selected_entry_tags();
        let people = app.selected_entry_people();
        let activities = app.selected_entry_activities();
        let feelings = app.selected_entry_feelings();
        let mood = app.selected_entry_mood();
        let entry_path = app.selected_entry_target().map(|target| target.path);

        let (scroll, labels, content_rect) = draw_markdown_panel(
            frame,
            area,
            PanelEntry {
                title: &title,
                content: &content,
                metadata: EntryMetadata {
                    tags: &tags,
                    people: &people,
                    activities: &activities,
                    feelings: &feelings,
                    mood,
                },
            },
            app.scroll.entry_view,
            app.focus == Focus::EntryView,
            entry_path.as_deref(),
        );
        app.scroll.entry_view = scroll;
        app.entry_view_image_hits = EntryViewImageHits {
            content_rect,
            scroll,
            labels,
        };
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

/// The entry content rendered by the markdown panel.
struct PanelEntry<'a> {
    title: &'a str,
    content: &'a str,
    metadata: EntryMetadata<'a>,
}

/// Draw the entry body and metadata, returning the applied scroll, the clickable
/// image-label positions (`(body line index, image index)`), and the body rect
/// (for mapping clicks back to labels).
fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    entry: PanelEntry<'_>,
    requested_scroll: u16,
    focused: bool,
    entry_path: Option<&Path>,
) -> (u16, Vec<(usize, usize)>, Rect) {
    let PanelEntry {
        title,
        content,
        metadata,
    } = entry;
    let wc = word_count(content);
    let block = panel_block(title, focused, Some(count_label(wc, "word", "words")));
    let layout = entry_metadata_layout(area, metadata.values());
    let metadata_scrolls = metadata_scrolls_with_body(area);
    let content_rect = if metadata_scrolls {
        PanelGeometry::new(area).content
    } else {
        layout.content
    };

    let width = content_rect.width.saturating_sub(1).max(1) as usize;
    let theme = markdown_theme();
    let renderer = MarkdownRenderer::new(width);
    let (mut lines, labels) = build_body_lines(content, &renderer, &theme, entry_path);
    if metadata_scrolls {
        lines.extend(metadata_section_lines(content_rect.width, metadata));
    }
    let line_count = lines.len();
    let scroll = viewer_scroll(requested_scroll, line_count, content_rect.height);

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content_rect);

    if !metadata_scrolls && layout.metadata.is_some() {
        draw_metadata_section(frame, layout, metadata);
    }

    render_scrollbar_if_needed(frame, area, line_count, content_rect.height, scroll);

    (scroll, labels, content_rect)
}

fn metadata_scrolls_with_body(area: Rect) -> bool {
    area.height < SCROLLING_METADATA_ENTRY_VIEW_HEIGHT_CUTOFF
}

/// Build the entry-body lines, replacing each lone in-folder image with a
/// clickable `[Image N …]` label and recording `(body line index, image index)`
/// so clicks and the viewer agree on numbering. Without an entry path the body
/// is rendered as-is.
fn build_body_lines(
    content: &str,
    renderer: &MarkdownRenderer,
    theme: &ThemeConfig,
    entry_path: Option<&Path>,
) -> (Vec<Line<'static>>, Vec<(usize, usize)>) {
    let Some(entry_path) = entry_path else {
        let mut lines = vec![Line::from("")];
        lines.extend(render_text_chunk(content, renderer, theme));
        return (lines, Vec::new());
    };

    // A leading blank row so the body starts one line below the border, matching
    // the blank that leads the journal and entry columns.
    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    let mut labels = Vec::new();
    let mut buffer = String::new();
    let mut image_index = 0usize;
    // True while the last emitted row was an image label with nothing buffered
    // since. Lets a blank source line right after an image emit an explicit blank
    // row instead of being swallowed by the empty buffer, preserving the gap.
    let mut after_image = false;

    for line in content.split('\n') {
        let Some((alt, _asset)) = sole_image_ref(line, entry_path) else {
            if after_image && buffer.is_empty() && line.trim().is_empty() {
                lines.push(Line::from(""));
                continue;
            }
            if !buffer.is_empty() {
                buffer.push('\n');
            }
            buffer.push_str(line);
            after_image = false;
            continue;
        };

        if !buffer.is_empty() {
            // A trailing blank source line leaves the buffer ending in a single
            // `\n`, which the renderer collapses; add a second so the gap before
            // the label survives.
            if buffer.ends_with('\n') {
                buffer.push('\n');
            }
            lines.extend(render_text_chunk(&buffer, renderer, theme));
            buffer.clear();
        }
        after_image = true;

        let start_row = lines.len();
        lines.push(image_label_line(image_index, &alt));
        labels.push((start_row, image_index));
        image_index += 1;
    }

    if !buffer.is_empty() {
        lines.extend(render_text_chunk(&buffer, renderer, theme));
    }

    (lines, labels)
}

/// A clickable `[Image N: alt - click here or press K]` label. The number is
/// 1-based; images 1-9 bind to their digit, the tenth to `0`, and later images
/// drop the `press K` hint (no digit left to bind).
fn image_label_line(index: usize, alt: &str) -> Line<'static> {
    let alt = alt.trim();
    let number = index + 1;
    let head = if alt.is_empty() {
        format!("Image {number}")
    } else {
        format!("Image {number}: {alt}")
    };
    let text = match digit_for_image(index) {
        Some(key) => format!("[{head} - click here or press {key}]"),
        None => format!("[{head} - click here]"),
    };
    Line::from(Span::styled(
        text,
        Style::default().add_modifier(Modifier::UNDERLINED),
    ))
}

/// Render a chunk of markdown text into owned (`'static`) lines.
fn render_text_chunk(
    text: &str,
    renderer: &MarkdownRenderer,
    theme: &ThemeConfig,
) -> Vec<Line<'static>> {
    let blocks = prepare_markdown_blocks(renderer.parse(text), renderer, theme);
    adapt_markdown_lines(renderer.render(&blocks, theme))
        .into_iter()
        .map(into_owned_line)
        .collect()
}

fn into_owned_line(line: Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .into_iter()
            .map(|span| Span {
                style: span.style,
                content: std::borrow::Cow::Owned(span.content.into_owned()),
            })
            .collect(),
    }
}

#[derive(Clone, Copy)]
struct EntryMetadata<'a> {
    tags: &'a [String],
    people: &'a [String],
    activities: &'a [String],
    feelings: &'a [String],
    mood: Option<i8>,
}

impl<'a> EntryMetadata<'a> {
    fn values(self) -> EntryMetadataValues<'a> {
        EntryMetadataValues {
            tags: self.tags,
            people: self.people,
            activities: self.activities,
            feelings: self.feelings,
            mood: self.mood,
        }
    }
}

fn draw_metadata_section(
    frame: &mut Frame<'_>,
    layout: EntryMetadataLayout,
    metadata: EntryMetadata<'_>,
) {
    let Some(area) = layout.metadata else {
        return;
    };
    let sep = "─".repeat(area.width.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(sep).style(Style::default().add_modifier(Modifier::DIM)),
        Rect { height: 1, ..area },
    );

    if let Some(score) = metadata.mood
        && let Some(mood_rect) = layout.mood
    {
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
    }
    if !metadata.feelings.is_empty()
        && let Some(row) = layout.feelings
    {
        frame.render_widget(
            Paragraph::new(metadata_value_lines_for_row(
                "Feelings: ",
                row,
                metadata.feelings,
            )),
            row.rect,
        );
    }

    if !metadata.people.is_empty()
        && let Some(row) = layout.people
    {
        frame.render_widget(
            Paragraph::new(metadata_value_lines_for_row(
                "People: ",
                row,
                metadata.people,
            )),
            row.rect,
        );
    }

    if !metadata.activities.is_empty()
        && let Some(row) = layout.activities
    {
        frame.render_widget(
            Paragraph::new(metadata_value_lines_for_row(
                "Activities: ",
                row,
                metadata.activities,
            )),
            row.rect,
        );
    }

    if !metadata.tags.is_empty()
        && let Some(row) = layout.tags
    {
        frame.render_widget(
            Paragraph::new(metadata_value_lines_for_row("Tags: ", row, metadata.tags)),
            row.rect,
        );
    }
}

fn metadata_section_lines(width: u16, metadata: EntryMetadata<'_>) -> Vec<Line<'static>> {
    if metadata.mood.is_none()
        && metadata.feelings.is_empty()
        && metadata.people.is_empty()
        && metadata.activities.is_empty()
        && metadata.tags.is_empty()
    {
        return Vec::new();
    }

    let mut lines = vec![Line::from(Span::styled(
        "─".repeat(width.saturating_sub(1) as usize),
        Style::default().add_modifier(Modifier::DIM),
    ))];

    if let Some(score) = metadata.mood {
        lines.push(mood_line(width, score));
    }
    if !metadata.feelings.is_empty() {
        lines.extend(metadata_value_lines_for_width(
            "Feelings: ",
            "Feelings: ".len() as u16,
            width,
            metadata.feelings,
        ));
    }
    if !metadata.people.is_empty() {
        lines.extend(metadata_value_lines_for_width(
            "People: ",
            "People: ".len() as u16,
            width,
            metadata.people,
        ));
    }
    if !metadata.activities.is_empty() {
        lines.extend(metadata_value_lines_for_width(
            "Activities: ",
            "Activities: ".len() as u16,
            width,
            metadata.activities,
        ));
    }
    if !metadata.tags.is_empty() {
        lines.extend(metadata_value_lines_for_width(
            "Tags: ",
            "Tags: ".len() as u16,
            width,
            metadata.tags,
        ));
    }

    lines
}

fn metadata_value_lines_for_row(
    prefix: &'static str,
    row: MetadataRowLayout,
    values: &[String],
) -> Vec<Line<'static>> {
    metadata_value_lines_for_width(prefix, row.prefix_width, row.rect.width, values)
}

fn metadata_value_lines_for_width(
    prefix: &'static str,
    prefix_width: u16,
    width: u16,
    values: &[String],
) -> Vec<Line<'static>> {
    metadata_value_rows(prefix_width, width, values)
        .into_iter()
        .enumerate()
        .map(|(row_index, value_indices)| {
            let mut spans = Vec::new();
            if row_index == 0 {
                spans.push(Span::styled(
                    prefix,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
            for (index, value_index) in value_indices.into_iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" | "));
                }
                spans.push(Span::raw(values[value_index].clone()));
            }
            Line::from(spans)
        })
        .collect()
}

fn mood_line(width: u16, score: i8) -> Line<'static> {
    let mut spans = Vec::new();
    let label_width = "Miserable ".len() as u16 + " Blissful".len() as u16;
    let bar_width = if width > label_width.saturating_add(3) {
        spans.push(Span::raw("Miserable "));
        width.saturating_sub(label_width)
    } else {
        width
    };

    spans.extend(mood_bar_spans(bar_width, score));

    if width > label_width.saturating_add(3) {
        spans.push(Span::raw(" Blissful"));
    }

    Line::from(spans)
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
        if area.width < 3 {
            return;
        }

        for (i, (symbol, style)) in mood_bar_cells(area.width, self.score)
            .into_iter()
            .enumerate()
        {
            let x = area.x + i as u16;
            let Some(cell) = buf.cell_mut((x, area.y)) else {
                continue;
            };
            cell.set_symbol(symbol);
            cell.set_style(style);
        }
    }
}

fn mood_bar_spans(width: u16, score: i8) -> Vec<Span<'static>> {
    mood_bar_cells(width, score)
        .into_iter()
        .map(|(symbol, style)| Span::styled(symbol, style))
        .collect()
}

fn mood_bar_cells(width: u16, score: i8) -> Vec<(&'static str, Style)> {
    let width = width as usize;
    if width < 3 {
        return Vec::new();
    }

    let center = width / 2;
    let lw = center;
    let rw = width - center - 1;

    let neg = score.min(0).unsigned_abs() as usize;
    let pos = score.max(0) as usize;

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

    let mut cells = Vec::with_capacity(width);
    for i in 0..width {
        if i == center {
            cells.push((if score == 0 { "┃" } else { "│" }, Style::default()));
        } else if i < center {
            let dist = center - i;
            cells.push(if dist <= filled_left {
                ("━", bold)
            } else {
                ("─", dim)
            });
        } else {
            let dist = i - center;
            cells.push(if dist <= filled_right {
                ("━", bold)
            } else {
                ("─", dim)
            });
        }
    }

    cells
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

fn prepare_markdown_blocks(
    blocks: Vec<MarkdownBlock>,
    renderer: &MarkdownRenderer,
    theme: &ThemeConfig,
) -> Vec<MarkdownBlock> {
    blocks
        .into_iter()
        .map(|block| prepare_markdown_block(block, renderer, theme))
        .collect()
}

fn prepare_markdown_block(
    block: MarkdownBlock,
    renderer: &MarkdownRenderer,
    theme: &ThemeConfig,
) -> MarkdownBlock {
    match block {
        MarkdownBlock::CodeBlock {
            lang,
            code,
            header_override,
            footer_override,
            prefix_override,
        } if lang.trim().eq_ignore_ascii_case("mermaid") => {
            let block = MarkdownBlock::CodeBlock {
                lang: "mermaid".to_string(),
                code: normalize_mermaid_code(&code),
                header_override,
                footer_override,
                prefix_override,
            };

            if renderer
                .render(std::slice::from_ref(&block), theme)
                .is_empty()
            {
                fallback_mermaid_block(block)
            } else {
                block
            }
        }
        MarkdownBlock::Blockquote {
            level,
            children,
            header_override,
            footer_override,
        } => MarkdownBlock::Blockquote {
            level,
            children: prepare_markdown_blocks(children, renderer, theme),
            header_override,
            footer_override,
        },
        block => block,
    }
}

fn normalize_mermaid_code(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let Some(start) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(start, |index| index + 1);
    let body = &lines[start..end];
    let indent = body
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count()
        })
        .min()
        .unwrap_or(0);

    body.iter()
        .map(|line| strip_leading_whitespace(line, indent))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_leading_whitespace(line: &str, count: usize) -> &str {
    let mut chars_to_strip = count;
    let mut byte_index = 0;
    for (index, ch) in line.char_indices() {
        if chars_to_strip == 0 || (ch != ' ' && ch != '\t') {
            byte_index = index;
            break;
        }
        chars_to_strip -= 1;
        byte_index = index + ch.len_utf8();
    }
    &line[byte_index..]
}

fn fallback_mermaid_block(block: MarkdownBlock) -> MarkdownBlock {
    if let MarkdownBlock::CodeBlock { code, .. } = block {
        MarkdownBlock::CodeBlock {
            lang: "text".to_string(),
            code,
            header_override: Some("```mermaid".to_string()),
            footer_override: Some("```".to_string()),
            prefix_override: Some(String::new()),
        }
    } else {
        block
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

#[cfg(test)]
mod image_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn entry_path_with_asset() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let assets = dir.path().join("2026-07-05T14-30-00-abc123.assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("x9k2.png"), b"img").unwrap();
        fs::write(assets.join("aa11.png"), b"img").unwrap();
        let entry_path = dir.path().join("2026-07-05T14-30-00-abc123.md");
        fs::write(&entry_path, b"entry").unwrap();
        (dir, entry_path)
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn image_label_includes_alt_and_press_hint_and_is_one_based() {
        assert_eq!(
            line_text(&image_label_line(0, "sunset")),
            "[Image 1: sunset - click here or press 1]"
        );
        assert_eq!(
            line_text(&image_label_line(3, "")),
            "[Image 4 - click here or press 4]"
        );
    }

    #[test]
    fn tenth_image_binds_to_zero_key() {
        assert_eq!(
            line_text(&image_label_line(9, "")),
            "[Image 10 - click here or press 0]"
        );
    }

    #[test]
    fn image_label_past_ten_drops_press_hint() {
        assert_eq!(
            line_text(&image_label_line(10, "late")),
            "[Image 11: late - click here]"
        );
    }

    /// Each lone in-folder image becomes a numbered clickable label, and its body
    /// line index is recorded so clicks map back to the right image.
    #[test]
    fn replaces_images_with_numbered_labels_and_records_positions() {
        let (_guard, entry_path) = entry_path_with_asset();
        let renderer = MarkdownRenderer::new(40);
        let theme = markdown_theme();
        let content = concat!(
            "Text above\n",
            "\n",
            "![a shot](2026-07-05T14-30-00-abc123.assets/x9k2.png)\n",
            "\n",
            "![](2026-07-05T14-30-00-abc123.assets/aa11.png)\n",
            "\n",
            "Text below",
        );

        let (lines, labels) = build_body_lines(content, &renderer, &theme, Some(&entry_path));

        let rendered: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(
            rendered,
            vec![
                String::new(),
                "Text above".to_string(),
                String::new(),
                "[Image 1: a shot - click here or press 1]".to_string(),
                String::new(),
                "[Image 2 - click here or press 2]".to_string(),
                String::new(),
                "Text below".to_string(),
            ],
        );
        assert_eq!(labels, vec![(3, 0), (5, 1)]);
    }

    /// Without an entry path (no selected entry) the body renders untouched.
    #[test]
    fn no_labels_without_entry_path() {
        let renderer = MarkdownRenderer::new(40);
        let theme = markdown_theme();
        let (_lines, labels) = build_body_lines("just text", &renderer, &theme, None);
        assert!(labels.is_empty());
    }
}
