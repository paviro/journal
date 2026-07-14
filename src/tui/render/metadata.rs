use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use notema_domain::{AirQuality, Celestial, Metadata, Weather};

use crate::tui::{
    env_strip::{EnvItem, env_strip_rows, environment_items},
    state::HoverTarget,
    surface::{EntryMetadataLayout, EntryMetadataValues, chip_items, chip_rows},
    theme::{PillCategory, PillStyle, theme},
};

#[derive(Clone)]
pub(super) struct EntryMetadata<'a> {
    tags: &'a [String],
    people: &'a [String],
    activities: &'a [String],
    feelings: &'a [String],
    mood: Option<i8>,
    /// The formatted location label, kept so [`Self::with_environment`] can
    /// rebuild the strip around it.
    location: Option<String>,
    /// The environment strip's items: location-only from a bare bundle, the
    /// full weather/air/moon/sun set once the viewer adds the entry's context.
    env: Vec<EnvItem>,
}

impl<'a> EntryMetadata<'a> {
    /// Build the entry-view metadata section straight from a [`Metadata`] bundle
    /// — the single construction path for both the viewer and the internal
    /// editor, so no front-matter field can render in one mode and vanish in the
    /// other. The bundle carries no environment data (that lives on the entry),
    /// so the strip starts location-only; the viewer layers the rest on with
    /// [`Self::with_environment`].
    pub(super) fn from_metadata(metadata: &'a Metadata) -> Self {
        let location = metadata.location_label();
        let env = environment_items(location.as_deref(), None, None, None);
        Self {
            tags: &metadata.tags,
            people: &metadata.people,
            activities: &metadata.activities,
            feelings: &metadata.feelings,
            mood: metadata.mood,
            location,
            env,
        }
    }

    /// Rebuild the environment strip with the entry's context tables — the
    /// viewer-only step; editor drafts carry no environment data.
    pub(super) fn with_environment(
        mut self,
        weather: Option<&Weather>,
        celestial: Option<&Celestial>,
        air: Option<&AirQuality>,
    ) -> Self {
        self.env = environment_items(self.location.as_deref(), weather, celestial, air);
        self
    }

    pub(super) fn values(&self) -> EntryMetadataValues<'_> {
        EntryMetadataValues {
            tags: self.tags,
            people: self.people,
            activities: self.activities,
            feelings: self.feelings,
            mood: self.mood,
            environment: &self.env,
        }
    }
}

pub(super) fn draw_metadata_section(
    frame: &mut Frame<'_>,
    layout: EntryMetadataLayout,
    metadata: &EntryMetadata<'_>,
    hover: HoverTarget,
) {
    let Some(area) = layout.metadata else {
        return;
    };
    let hovered_chip = match hover {
        HoverTarget::MetadataChip(index) => Some(index),
        _ => None,
    };
    let sep = theme()
        .env_glyphs()
        .rule
        .to_string()
        .repeat(area.width.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(sep).style(theme().muted()),
        Rect { height: 1, ..area },
    );

    if !metadata.env.is_empty()
        && let Some(rect) = layout.environment
    {
        frame.render_widget(
            Paragraph::new(env_strip_lines(rect.width, &metadata.env)),
            rect,
        );
    }

    if let Some(score) = metadata.mood
        && let Some(mood_rect) = layout.mood
    {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(MOOD_LOW_LABEL.len() as u16),
                Constraint::Min(4),
                Constraint::Length(MOOD_HIGH_LABEL.len() as u16),
            ])
            .split(mood_rect);
        frame.render_widget(
            Paragraph::new(MOOD_LOW_LABEL).style(theme().muted()),
            chunks[0],
        );
        frame.render_widget(MoodBar::new(score), chunks[1]);
        frame.render_widget(
            Paragraph::new(MOOD_HIGH_LABEL).style(theme().muted()),
            chunks[2],
        );
    }

    if let Some(rect) = layout.chips {
        frame.render_widget(
            Paragraph::new(chip_lines(rect.width, metadata.values(), hovered_chip)),
            rect,
        );
    }
}

/// The environment strip as display lines, shared by the pinned and scrolling
/// layouts: glyph-led items with muted separator dots, continuation rows of a
/// wrapped item indented under its glyph.
fn env_strip_lines(width: u16, items: &[EnvItem]) -> Vec<Line<'static>> {
    let separator = format!(" {} ", theme().env_glyphs().separator);
    env_strip_rows(width, items)
        .into_iter()
        .map(|row| {
            let mut spans = Vec::new();
            for (index, item) in row.into_iter().enumerate() {
                if index > 0 {
                    spans.push(Span::styled(separator.clone(), theme().muted()));
                }
                if item.continuation {
                    spans.push(Span::raw("  "));
                } else if let Some(glyph) = item.glyph {
                    spans.push(Span::styled(format!("{glyph} "), item.glyph_style));
                }
                spans.extend(
                    item.segments
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style)),
                );
            }
            Line::from(spans)
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
        && metadata.env.is_empty()
    {
        return Vec::new();
    }

    let mut lines = vec![Line::from(Span::styled(
        theme()
            .env_glyphs()
            .rule
            .to_string()
            .repeat(width.saturating_sub(1) as usize),
        theme().muted(),
    ))];

    // A blank row sets each present block off from the one above it, matching
    // the pinned layout's gaps so the two modes stay row-for-row identical.
    let mut emitted = false;
    let env = env_strip_lines(width, &metadata.env);
    if !env.is_empty() {
        lines.extend(env);
        emitted = true;
    }
    if let Some(score) = metadata.mood {
        if emitted {
            lines.push(Line::from(""));
        }
        lines.push(mood_line(width, score));
        emitted = true;
    }
    let chips = chip_lines(width, metadata.values(), None);
    if !chips.is_empty() {
        if emitted {
            lines.push(Line::from(""));
        }
        lines.extend(chips);
    }

    lines
}

/// The chip pills as display lines: one label-less flow across every category,
/// each pill led by its category glyph and styled by its category. `hovered`
/// is the flat index of the pill under the cursor, highlighted whole.
fn chip_lines(
    width: u16,
    values: EntryMetadataValues<'_>,
    hovered: Option<usize>,
) -> Vec<Line<'static>> {
    let items: Vec<(PillCategory, &str)> = chip_items(values).collect();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for row in chip_rows(width, values) {
        // A blank spacer sets each wrapped chip row off from the one above it.
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        let mut spans = Vec::new();
        for (position, index) in row.into_iter().enumerate() {
            if position > 0 {
                spans.push(Span::raw(" "));
            }
            let (category, value) = items[index];
            spans.push(pill_span(value, category, hovered == Some(index)));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// One metadata value as a chip: its category glyph then the value, inside the
/// pill's own fill (or `[glyph value]` for brackets) so the marker echoes the
/// strip's glyph-led items while riding the pill. Always exactly the glyph lead
/// plus two cells wider than the value, so every style shares the row flow and
/// hit-test math. When `hovered`, the whole chip takes the theme's hover
/// highlight, matching its click region.
fn pill_span(value: &str, category: PillCategory, hovered: bool) -> Span<'static> {
    let glyph = theme().pill_glyph(category);
    let (content, mut style) = match theme().pill_style() {
        PillStyle::Bracket => (format!("[{glyph} {value}]"), Style::default()),
        PillStyle::Reversed | PillStyle::Bg => {
            (format!(" {glyph} {value} "), theme().pill(category))
        }
    };
    if hovered {
        style = style.patch(theme().hover());
    }
    Span::styled(content, style)
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

/// The pole labels flanking the full-width mood bar.
const MOOD_LOW_LABEL: &str = "Miserable ";
const MOOD_HIGH_LABEL: &str = " Blissful";

/// The mood row as a full-width bar: the "Miserable"/"Blissful" pole labels
/// flank the centered, valence-colored bar. Below a floor width the labels drop
/// and the bar takes the whole line.
fn mood_line(width: u16, score: i8) -> Line<'static> {
    let labels_width = (MOOD_LOW_LABEL.len() + MOOD_HIGH_LABEL.len()) as u16;
    let mut spans = Vec::new();
    let bar_width = if width > labels_width.saturating_add(3) {
        spans.push(Span::styled(MOOD_LOW_LABEL, theme().muted()));
        width.saturating_sub(labels_width)
    } else {
        width
    };

    spans.extend(mood_bar_spans(bar_width, score));

    if width > labels_width.saturating_add(3) {
        spans.push(Span::styled(MOOD_HIGH_LABEL, theme().muted()));
    }

    Line::from(spans)
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

    // Number of filled cells per side: the mood's 0..5 magnitude mapped across
    // the side's width, with any nonzero mood showing at least one cell.
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

    // The fill carries the valence hue — negative left, positive right — so
    // the filled span reads as one textured bar; the empty track stays muted,
    // keeping the row full-height without competing with the fill. The heavy
    // `┃` shown at an exact zero stays fixed: weight is the meaning.
    let negative = theme().mood_fill(false);
    let positive = theme().mood_fill(true);
    let dim = theme().muted();
    let center_glyph = theme().glyphs().diverge_center.to_string();
    let fill_glyph = theme().env_glyphs().mood_fill.to_string();
    let track_glyph = theme().env_glyphs().mood_track.to_string();

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
                (fill_glyph.clone(), negative)
            } else {
                (track_glyph.clone(), dim)
            });
        } else {
            let dist = i - center;
            cells.push(if dist <= filled_right {
                (fill_glyph.clone(), positive)
            } else {
                (track_glyph.clone(), dim)
            });
        }
    }

    cells
}
