use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use notema_domain::Metadata;

use crate::tui::{
    surface::{
        EntryMetadataLayout, EntryMetadataValues, LOCATION_PREFIX, MetadataRowLayout,
        location_wrapped_lines, metadata_value_rows,
    },
    theme::theme,
};

#[derive(Clone)]
pub(super) struct EntryMetadata<'a> {
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
    pub(super) fn from_metadata(metadata: &'a Metadata) -> Self {
        Self {
            tags: &metadata.tags,
            people: &metadata.people,
            activities: &metadata.activities,
            feelings: &metadata.feelings,
            mood: metadata.mood,
            location: metadata.location_label(),
        }
    }

    pub(super) fn values(&self) -> EntryMetadataValues<'_> {
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

pub(super) fn draw_metadata_section(
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

pub(super) fn metadata_section_lines(
    width: u16,
    metadata: &EntryMetadata<'_>,
) -> Vec<Line<'static>> {
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
