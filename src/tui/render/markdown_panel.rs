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
    highlight::{HighlightHooks, TreeSitterHighlighter},
    markdown::{MarkdownBlock, MarkdownRenderer, RenderHooks},
    theme::{CodeColors, ThemeConfig},
};

use journal_core::{Entry, Metadata};
use std::path::Path;

use crate::tui::{
    app::{App, EntryViewImageHits, Focus},
    editor_state::{EditorPrompt, EntryEditor},
    image::{digit_for_image, sole_image_ref},
    render::{
        count_label, entry_metadata_layout, panel_block, render_centered_notice,
        render_scrollbar_if_needed, viewer_scroll,
    },
    surface::{
        EntryMetadataLayout, EntryMetadataValues, LOCATION_PREFIX, MetadataRowLayout,
        PanelGeometry, location_wrapped_lines, metadata_section_height, metadata_value_rows,
    },
    theme::theme,
};

/// The body (writing/reading area) is kept at least this tall; the metadata block
/// only pins below the body when the pane can still afford these lines, otherwise it
/// folds into the scroll.
const MIN_ENTRY_BODY_LINES: u16 = 20;

pub(crate) fn draw_selected_entry_view(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_entry_view() {
        let metadata = app
            .resolved_selected_entry()
            .map(Entry::metadata_bundle)
            .unwrap_or_default();
        let entry_path = app.selected_entry_target().map(|target| target.path);

        let (scroll, labels, content_rect, line_count) = draw_markdown_panel(
            frame,
            area,
            app,
            PanelEntry {
                title: &title,
                content: &content,
                word_count: app.selected_entry_word_count(),
                metadata: EntryMetadata::from_metadata(&metadata),
            },
            app.nav.scroll.entry_view,
            app.nav.focus == Focus::EntryView,
            entry_path.as_deref(),
        );
        app.nav.scroll.entry_view = scroll;
        app.entry_view_image_hits = EntryViewImageHits {
            content_rect,
            scroll,
            line_count,
            labels,
        };
    } else {
        let block = panel_block("Entry", app.nav.focus == Focus::EntryView, None);
        let content = PanelGeometry::new(area).content;
        frame.render_widget(block, area);
        super::panel_focus_stripe(frame, area, app.nav.focus == Focus::EntryView);
        render_centered_notice(frame, content, "No entry selected");
    }
}

/// Draw the internal editor in the entry-view pane: the same bordered panel as
/// the viewer, with the `ratatui-textarea` buffer as the body and the buffered
/// metadata pinned below it. Honors the viewer's max-width and vertical-center
/// settings, and shows an inline discard confirmation when one is pending.
pub(crate) fn draw_entry_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &mut EntryEditor,
    side_margin: u16,
    top_margin: u16,
) {
    let block = panel_block(editor.title(), true, None);
    frame.render_widget(block, area);
    super::panel_focus_stripe(frame, area, true);

    // Same builder the viewer uses, from the buffered metadata — so location and
    // every other front-matter field show in edit mode too.
    let metadata = EntryMetadata::from_metadata(&editor.metadata);

    // The metadata section pins below the body only while the pane can still give the
    // body its minimum height; once the metadata would push it under that, it's
    // dropped and the whole pane goes to the textarea. (The viewer instead folds
    // metadata into its scroll there, but the editor's scroll is cursor-driven and
    // can't reach a read-only block past the text.) Nothing is lost: the Ctrl+G
    // dialogs show the current values as you edit them, and the viewer shows them in
    // full on save.
    let (body_area, layout) = if metadata_scrolls_with_body(area, metadata.values()) {
        (PanelGeometry::new(area).content, None)
    } else {
        let layout = entry_metadata_layout(area, metadata.values());
        (layout.content, Some(layout))
    };

    // Inset the writing area with a fixed margin (side left/right, top, 0 bottom)
    // rather than a max-width gutter. The editor never floats vertically — typing
    // at a moving baseline is disorienting.
    let text_rect = Rect {
        x: body_area.x + side_margin,
        y: body_area.y + top_margin,
        width: body_area.width.saturating_sub(side_margin * 2),
        height: body_area.height.saturating_sub(top_margin),
    };

    // While selecting, draw the reversed-block caret so the boundary character
    // reads as part of the selection (a thin bar can't fill that cell); otherwise
    // the theme's cursor style — by default unstyled, leaving the native bar
    // cursor placed below as the only caret.
    let selecting = editor.textarea.selection_range().is_some();
    editor.textarea.set_cursor_style(if selecting {
        theme().selection()
    } else {
        theme().cursor()
    });

    editor.text_rect = text_rect;
    frame.render_widget(&editor.textarea, text_rect);

    // Native terminal bar cursor, only while typing without a selection and with
    // no modal prompt over the editor. screen_cursor().row is the absolute wrapped
    // row; subtracting the scroll top gives the viewport-relative row. Wrap mode
    // has no horizontal scroll, so col maps directly. Valid only after render.
    if !selecting && matches!(editor.prompt, EditorPrompt::None) {
        let sc = editor.textarea.screen_cursor();
        let scroll = editor.textarea.scroll_offset() as usize;
        if let Some(rel) = sc.row.checked_sub(scroll) {
            let x = text_rect.x + sc.col as u16;
            let y = text_rect.y + rel as u16;
            if x < text_rect.x + text_rect.width && y < text_rect.y + text_rect.height {
                frame.set_cursor_position((x, y));
            }
        }
    }

    // Scroll offset and wrapped-line count are only valid after the textarea has
    // rendered (it stores them during render), so read them here.
    render_scrollbar_if_needed(
        frame,
        area,
        editor.textarea.screen_line_count(),
        text_rect.height,
        editor.textarea.scroll_offset() as usize,
    );

    if let Some(layout) = layout {
        draw_metadata_section(frame, layout, &metadata);
    }
}

/// The entry content rendered by the markdown panel.
struct PanelEntry<'a> {
    title: &'a str,
    content: &'a str,
    /// Precomputed on the entry, so the panel title never re-tokenizes the body.
    word_count: usize,
    metadata: EntryMetadata<'a>,
}

/// Draw the entry body and metadata, returning the applied scroll, the clickable
/// image-label positions (`(body line index, image index)`), the body rect (for
/// mapping clicks back to labels), and the total rendered line count (for scrollbar
/// drag mapping).
fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    entry: PanelEntry<'_>,
    requested_scroll: u16,
    focused: bool,
    entry_path: Option<&Path>,
) -> (u16, Vec<(usize, usize)>, Rect, usize) {
    let PanelEntry {
        title,
        content,
        word_count,
        metadata,
    } = entry;
    let block = panel_block(
        title,
        focused,
        Some(count_label(word_count, "word", "words")),
    );
    let layout = entry_metadata_layout(area, metadata.values());
    let metadata_scrolls = metadata_scrolls_with_body(area, metadata.values());
    let content_rect = if metadata_scrolls {
        PanelGeometry::new(area).content
    } else {
        layout.content
    };
    // The metadata (when it scrolls with the body) shares the paragraph, so keep
    // the full width there; otherwise gutter the body to a readable max width.
    let body_rect = if metadata_scrolls {
        content_rect
    } else {
        centered_body_rect(
            content_rect,
            app.config.ui.layout.entry_viewer.body_max_width,
        )
    };

    let width = body_rect.width.saturating_sub(1).max(1) as usize;
    // Memoized on (entry path, width, data version): the markdown parse + syntax
    // highlight + render is the preview's dominant per-frame cost, so a frame that
    // only scrolled, blinked, or ticked images reuses the rendered lines.
    let body = app.cached_entry_body(entry_path, width, || {
        let theme = markdown_theme();
        let mut renderer = MarkdownRenderer::new(width);
        if let Some(hooks) = highlight_hooks(width) {
            renderer = renderer.with_render_hooks(hooks);
        }
        build_body_lines(content, &renderer, &theme, entry_path)
    });
    let mut lines = body.0.clone();
    let labels = body.1.clone();
    if metadata_scrolls {
        let meta_lines = metadata_section_lines(body_rect.width, &metadata);
        if !meta_lines.is_empty() {
            let height = body_rect.height as usize;
            if lines.len() + meta_lines.len() < height {
                // Fits: bottom-attach the metadata to the pane's bottom edge,
                // matching the pinned layout on taller panes.
                lines.resize(height - meta_lines.len(), Line::from(""));
            } else {
                // Overflows: one blank line sets the metadata off from the body as
                // it scrolls into view.
                lines.push(Line::from(""));
            }
            lines.extend(meta_lines);
        }
    }
    let line_count = lines.len();
    let scroll = viewer_scroll(requested_scroll, line_count, body_rect.height);
    // Float a short entry in the vertical middle — but only when the metadata is
    // pinned (taller panes). On short panes the body flows from the top with the
    // metadata bottom-attached, so centering would fight that.
    let body_rect = if app.config.ui.layout.entry_viewer.body_center_vertically && !metadata_scrolls
    {
        center_body_vertically(body_rect, line_count, scroll)
    } else {
        body_rect
    };

    frame.render_widget(block, area);
    super::panel_focus_stripe(frame, area, focused);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), body_rect);

    if !metadata_scrolls && layout.metadata.is_some() {
        draw_metadata_section(frame, layout, &metadata);
    }

    render_scrollbar_if_needed(frame, area, line_count, body_rect.height, scroll as usize);

    (scroll, labels, body_rect, line_count)
}

/// Pin the metadata below the body only when doing so still leaves the body at least
/// [`MIN_ENTRY_BODY_LINES`]; otherwise fold it into the scroll. With no metadata the
/// height is zero and this reduces to a plain minimum-body check.
pub(crate) fn metadata_scrolls_with_body(area: Rect, values: EntryMetadataValues<'_>) -> bool {
    let inner = PanelGeometry::new(area).content;
    let metadata_height = metadata_section_height(inner.width, values);
    inner.height < MIN_ENTRY_BODY_LINES.saturating_add(metadata_height)
}

/// Cap `rect` at `max_width` and center it horizontally, leaving the height and
/// narrower panels untouched. A `max_width` of 0 means no cap.
fn centered_body_rect(rect: Rect, max_width: u16) -> Rect {
    if max_width == 0 || rect.width <= max_width {
        return rect;
    }
    let x = rect.x + (rect.width - max_width) / 2;
    Rect {
        x,
        width: max_width,
        ..rect
    }
}

/// When the entry fits without scrolling, push it down so it sits in the vertical
/// middle of `rect`; otherwise render from the top so scrolling covers every line.
fn center_body_vertically(rect: Rect, line_count: usize, scroll: u16) -> Rect {
    if scroll > 0 || line_count >= rect.height as usize {
        return rect;
    }
    let pad = (rect.height - line_count as u16) / 2;
    Rect {
        y: rect.y + pad,
        height: rect.height - pad,
        ..rect
    }
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
    Line::from(Span::styled(text, theme().md_link()))
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

#[derive(Clone)]
struct EntryMetadata<'a> {
    tags: &'a [String],
    people: &'a [String],
    activities: &'a [String],
    feelings: &'a [String],
    mood: Option<i8>,
    /// The formatted location label, computed from the bundle's `Location`.
    location: Option<String>,
}

impl<'a> EntryMetadata<'a> {
    /// Build the entry-view metadata section straight from a [`Metadata`] bundle
    /// — the single construction path for both the viewer and the internal
    /// editor, so no front-matter field can render in one mode and vanish in the
    /// other.
    fn from_metadata(metadata: &'a Metadata) -> Self {
        Self {
            tags: &metadata.tags,
            people: &metadata.people,
            activities: &metadata.activities,
            feelings: &metadata.feelings,
            mood: metadata.mood,
            location: metadata.location_label(),
        }
    }

    fn values(&self) -> EntryMetadataValues<'_> {
        EntryMetadataValues {
            tags: self.tags,
            people: self.people,
            activities: self.activities,
            feelings: self.feelings,
            mood: self.mood,
            location: self.location.as_deref(),
        }
    }
}

fn draw_metadata_section(
    frame: &mut Frame<'_>,
    layout: EntryMetadataLayout,
    metadata: &EntryMetadata<'_>,
) {
    let Some(area) = layout.metadata else {
        return;
    };
    let sep = "─".repeat(area.width.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(sep).style(theme().muted()),
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

    if let Some(location) = metadata.location.as_deref()
        && let Some(row) = layout.location
    {
        frame.render_widget(
            Paragraph::new(location_lines(row.prefix_width, row.rect.width, location)),
            row.rect,
        );
    }
}

/// Build the location row as wrapped lines: the bold `Location: ` label leads
/// the first line, continuation lines run flush-left.
fn location_lines(prefix_width: u16, width: u16, value: &str) -> Vec<Line<'static>> {
    location_wrapped_lines(prefix_width, width, value)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(LOCATION_PREFIX, theme().heading()),
                    Span::raw(chunk),
                ])
            } else {
                Line::from(Span::raw(chunk))
            }
        })
        .collect()
}

fn metadata_section_lines(width: u16, metadata: &EntryMetadata<'_>) -> Vec<Line<'static>> {
    if metadata.mood.is_none()
        && metadata.feelings.is_empty()
        && metadata.people.is_empty()
        && metadata.activities.is_empty()
        && metadata.tags.is_empty()
        && metadata.location.is_none()
    {
        return Vec::new();
    }

    let mut lines = vec![Line::from(Span::styled(
        "─".repeat(width.saturating_sub(1) as usize),
        theme().muted(),
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
    if let Some(location) = metadata.location.as_deref() {
        lines.extend(location_lines(
            LOCATION_PREFIX.len() as u16,
            width,
            location,
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
                spans.push(Span::styled(prefix, theme().heading()));
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
            cell.set_symbol(&symbol);
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

fn mood_bar_cells(width: u16, score: i8) -> Vec<(String, Style)> {
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

    let bold = theme().heading();
    let dim = theme().muted();
    // The theme picks the light strokes; the heavy `┃`/`━` variants stay fixed
    // because weight is the meaning (an exact zero, the filled span).
    let center_glyph = theme().glyphs().bar_center.to_string();
    let fill_glyph = theme().glyphs().mood_fill.to_string();

    let mut cells = Vec::with_capacity(width);
    for i in 0..width {
        if i == center {
            cells.push(if score == 0 {
                ("┃".to_string(), Style::default())
            } else {
                (center_glyph.clone(), Style::default())
            });
        } else if i < center {
            let dist = center - i;
            cells.push(if dist <= filled_left {
                ("━".to_string(), bold)
            } else {
                (fill_glyph.clone(), dim)
            });
        } else {
            let dist = i - center;
            cells.push(if dist <= filled_right {
                ("━".to_string(), bold)
            } else {
                (fill_glyph.clone(), dim)
            });
        }
    }

    cells
}

/// The markdown renderer's colors, fed from the theme's markdown tokens. The
/// crate couples elements to a handful of slots: `primary` colors H1 headings
/// and inline links, `secondary` H3, `accent_yellow` inline code, `muted` the
/// rules/prefixes; the JSON-tree slots follow the closest `[markdown.syntax]`
/// categories. Tokens without a color resolve to `Reset`, so a theme with no
/// `[markdown]` colors renders exactly the plain classic output.
pub(crate) fn markdown_theme() -> ThemeConfig {
    let theme = theme();
    let fg = |style: Style| adapt_color(style.fg);
    let syntax = theme.syntax();
    let reset = MarkdownColor::Reset;
    ThemeConfig::builder()
        .with_text_color(fg(theme.text()))
        .with_muted_text_color(fg(theme.md_blockquote()))
        .with_primary_color(fg(theme.md_heading()))
        .with_popup_selected_background(reset)
        .with_border_color(reset)
        .with_focused_border_color(reset)
        .with_secondary_color(fg(theme.md_heading3()))
        .with_info_color(fg(theme.md_link()))
        .with_json_key_color(adapt_color(Some(syntax.property)))
        .with_json_string_color(adapt_color(Some(syntax.string)))
        .with_json_number_color(adapt_color(Some(syntax.number)))
        .with_json_bool_color(adapt_color(Some(syntax.constant)))
        .with_json_null_color(adapt_color(Some(syntax.constant)))
        .with_accent_yellow(fg(theme.md_code()))
        .with_code_colors(syntax_code_colors(syntax))
        .build()
}

/// ratatui `Color` → the markdown crate's color enum; the reverse of
/// [`adapt_markdown_color`]. An unset foreground becomes `Reset`.
fn adapt_color(color: Option<Color>) -> MarkdownColor {
    match color {
        None => MarkdownColor::Reset,
        Some(Color::Reset) => MarkdownColor::Reset,
        Some(Color::Black) => MarkdownColor::Black,
        Some(Color::Red) => MarkdownColor::Red,
        Some(Color::Green) => MarkdownColor::Green,
        Some(Color::Yellow) => MarkdownColor::Yellow,
        Some(Color::Blue) => MarkdownColor::Blue,
        Some(Color::Magenta) => MarkdownColor::Magenta,
        Some(Color::Cyan) => MarkdownColor::Cyan,
        Some(Color::Gray) => MarkdownColor::Gray,
        Some(Color::DarkGray) => MarkdownColor::DarkGray,
        Some(Color::LightRed) => MarkdownColor::LightRed,
        Some(Color::LightGreen) => MarkdownColor::LightGreen,
        Some(Color::LightYellow) => MarkdownColor::LightYellow,
        Some(Color::LightBlue) => MarkdownColor::LightBlue,
        Some(Color::LightMagenta) => MarkdownColor::LightMagenta,
        Some(Color::LightCyan) => MarkdownColor::LightCyan,
        Some(Color::White) => MarkdownColor::White,
        Some(Color::Rgb(r, g, b)) => MarkdownColor::Rgb(r, g, b),
        Some(Color::Indexed(index)) => MarkdownColor::Indexed(index),
    }
}

/// Tree-sitter syntax highlighting for fenced code blocks, installed only when
/// the theme colors at least one `[markdown.syntax]` category — plain themes
/// keep the renderer's classic un-highlighted code blocks. Languages without a
/// compiled grammar fall back to plain rendering per block.
fn highlight_hooks(max_width: usize) -> Option<Box<dyn RenderHooks>> {
    let syntax = theme().syntax();
    if !syntax.any_color() {
        return None;
    }
    let highlighter = TreeSitterHighlighter::new().with_code_colors(syntax_code_colors(syntax));
    Some(Box::new(
        HighlightHooks::new(std::sync::Arc::new(highlighter), max_width)
            .with_border_color(adapt_color(theme().md_blockquote().fg)),
    ))
}

/// The theme's `[markdown.syntax]` colors as the crate's code-block palette.
/// Unset categories resolved to `Reset`, so an empty table highlights nothing.
fn syntax_code_colors(syntax: crate::tui::theme::Syntax) -> CodeColors {
    let color = |color| adapt_color(Some(color));
    CodeColors {
        comment: color(syntax.comment),
        keyword: color(syntax.keyword),
        string: color(syntax.string),
        string_escape: color(syntax.string_escape),
        number: color(syntax.number),
        constant: color(syntax.constant),
        function: color(syntax.function),
        r#type: color(syntax.r#type),
        variable: color(syntax.variable),
        property: color(syntax.property),
        operator: color(syntax.operator),
        punctuation: color(syntax.punctuation),
        attribute: color(syntax.attribute),
        tag: color(syntax.tag),
        label: color(syntax.label),
        error: color(syntax.error),
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
