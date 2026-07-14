use ratatui::layout::{Constraint, Direction, Layout, Rect};
use unicode_width::UnicodeWidthStr;

use crate::tui::{
    env_strip::{EnvItem, env_strip_height},
    theme::{ChromeStyle, PillCategory, theme},
};

/// Per-entry box chrome consumed horizontally: a left/right border plus one
/// space of padding on each side (`│ … │`).
pub(crate) const ENTRY_BOX_H_OVERHEAD: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanelGeometry {
    pub(crate) area: Rect,
    pub(crate) content: Rect,
}

impl PanelGeometry {
    pub(crate) fn new(area: Rect) -> Self {
        let inner = panel_inner(area);
        let content = surface_content_inner(inner);
        Self { area, content }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryListGeometry {
    pub(crate) panel: PanelGeometry,
    pub(crate) text_width: u16,
    pub(crate) viewport_height: u16,
}

impl EntryListGeometry {
    pub(crate) fn new(area: Rect) -> Self {
        let panel = PanelGeometry::new(area);
        Self {
            text_width: panel.content.width.saturating_sub(ENTRY_BOX_H_OVERHEAD),
            viewport_height: panel.content.height,
            panel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryMetadataLayout {
    pub(crate) content: Rect,
    pub(crate) metadata: Option<Rect>,
    /// The environment strip's rows (weather, air, moon, sun, location),
    /// display-only — excluded from the click hit-test.
    pub(crate) environment: Option<Rect>,
    /// The full-width mood bar's row, when the entry carries a mood.
    pub(crate) mood: Option<Rect>,
    /// The chip pills — every feeling/person/activity/tag in one label-less
    /// flow, the category carried by each pill's own style.
    pub(crate) chips: Option<Rect>,
}

#[derive(Clone, Copy)]
pub(crate) struct EntryMetadataValues<'a> {
    pub(crate) tags: &'a [String],
    pub(crate) people: &'a [String],
    pub(crate) activities: &'a [String],
    pub(crate) feelings: &'a [String],
    pub(crate) mood: Option<i8>,
    /// The environment strip's items — weather, air, moon, sun, location;
    /// rendered, not clickable.
    pub(crate) environment: &'a [EnvItem],
}

pub(crate) fn panel_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Apply the shared horizontal content gutter inside a frame's inner rect.
/// Content always stays one cell off the left frame and one cell before the
/// scrollbar. Flat chrome also reserves the blank surface-edge column after
/// its inset scrollbar.
pub(crate) fn surface_content_inner(area: Rect) -> Rect {
    let pad = 1;
    let right_pad = pad + u16::from(theme().chrome() == ChromeStyle::Flat);
    Rect {
        x: area.x.saturating_add(pad),
        width: area.width.saturating_sub(pad + right_pad).max(1),
        ..area
    }
}

/// The vertical scrollbar column shared by drawing and mouse hit-testing.
/// Bordered chrome uses the right border; flat chrome leaves one blank column
/// between the scrollbar and the surface edge.
pub(crate) fn scrollbar_bar_rect(area: Rect) -> Rect {
    let right_padding = u16::from(theme().chrome() == ChromeStyle::Flat);
    Rect {
        x: area
            .x
            .saturating_add(area.width.saturating_sub(1 + right_padding)),
        y: area.y.saturating_add(1),
        width: 1,
        height: area.height.saturating_sub(2),
    }
}

/// Outer width required for `content_width` cells plus the shared gutters,
/// frame/scrollbar column, and the flat surface-edge column when present.
pub(crate) fn surface_outer_width(content_width: u16) -> u16 {
    content_width
        .saturating_add(4)
        .saturating_add(u16::from(theme().chrome() == ChromeStyle::Flat))
}

pub(crate) fn point_in_rect(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x
        && x < area.x.saturating_add(area.width)
        && y >= area.y
        && y < area.y.saturating_add(area.height)
}

/// The blank row between two adjacent metadata blocks (strip, mood, chips).
const METADATA_ROW_GAP: u16 = 1;

pub(crate) fn entry_metadata_layout(
    reader_area: Rect,
    values: EntryMetadataValues<'_>,
) -> EntryMetadataLayout {
    let inner = PanelGeometry::new(reader_area).content;
    let metadata_height = metadata_section_height(inner.width, values);
    let show_metadata = metadata_height > 0 && inner.height > metadata_height;

    let (content, metadata) = if show_metadata {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(metadata_height)])
            .split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

    let mut environment_rect = None;
    let mut mood_rect = None;
    let mut chips_rect = None;

    if let Some(metadata_rect) = metadata {
        // Strip, mood bar, and chips are three blocks; a blank row sets each off
        // from the one above it so they don't read as one wall.
        let mut y = metadata_rect.y.saturating_add(1);
        let mut emitted = false;
        if !values.environment.is_empty() {
            let height = env_strip_height(metadata_rect.width, values.environment);
            environment_rect = Some(Rect {
                y,
                height,
                ..metadata_rect
            });
            y = y.saturating_add(height);
            emitted = true;
        }
        if values.mood.is_some() {
            if emitted {
                y = y.saturating_add(METADATA_ROW_GAP);
            }
            mood_rect = Some(Rect {
                y,
                height: 1,
                ..metadata_rect
            });
            y = y.saturating_add(1);
            emitted = true;
        }
        let chip_height = chips_height(metadata_rect.width, values);
        if chip_height > 0 {
            if emitted {
                y = y.saturating_add(METADATA_ROW_GAP);
            }
            chips_rect = Some(Rect {
                y,
                height: chip_height,
                ..metadata_rect
            });
        }
    }

    EntryMetadataLayout {
        content,
        metadata,
        environment: environment_rect,
        mood: mood_rect,
        chips: chips_rect,
    }
}

/// A metadata pill occupies its value plus two padding/bracket cells in every
/// pill style, so layout and hit-testing never depend on the style drawn.
pub(crate) const PILL_PAD: u16 = 2;

/// The glyph-plus-space every pill leads with — its category marker, echoing
/// the environment strip's glyph-led items. Style-independent, so layout and
/// hit-testing count it uniformly.
pub(crate) const PILL_GLYPH_LEAD: u16 = 2;

/// The cells a pill occupies on a row: its category glyph lead, the value, and
/// the padding/bracket cells shared by every pill style.
const fn pill_cells(value_width: u16) -> u16 {
    PILL_GLYPH_LEAD + value_width + PILL_PAD
}

/// The single space between two pills on a row.
const PILL_SEPARATOR: u16 = 1;

/// Every chip in display order — feelings, people, activities, tags — as
/// `(category, value)` pairs. One label-less flow; the pill's own style is
/// what says which category a value belongs to.
pub(crate) fn chip_items<'a>(
    values: EntryMetadataValues<'a>,
) -> impl Iterator<Item = (PillCategory, &'a str)> {
    let of = |category: PillCategory, list: &'a [String]| {
        list.iter().map(move |value| (category, value.as_str()))
    };
    of(PillCategory::Feelings, values.feelings)
        .chain(of(PillCategory::People, values.people))
        .chain(of(PillCategory::Activities, values.activities))
        .chain(of(PillCategory::Tags, values.tags))
}

/// Flow the chips into display rows of `row_width` cells: greedy, never
/// splitting a pill, one space between pills. Each row is a list of indices
/// into [`chip_items`]'s order.
pub(crate) fn chip_rows(row_width: u16, values: EntryMetadataValues<'_>) -> Vec<Vec<usize>> {
    flow_by_widths(
        0,
        row_width,
        chip_items(values).map(|(_, value)| UnicodeWidthStr::width(value)),
        PILL_GLYPH_LEAD + PILL_PAD,
        PILL_SEPARATOR,
    )
}

/// The rows the chip flow occupies, zero when the entry has no chips. A blank
/// spacer row separates each pair of wrapped chip rows, so `n` chip rows take
/// `2n - 1` display rows.
fn chips_height(row_width: u16, values: EntryMetadataValues<'_>) -> u16 {
    let rows = chip_rows(row_width, values).len();
    if rows == 0 {
        return 0;
    }
    (rows * 2 - 1).min(u16::MAX as usize) as u16
}

/// The chip under the given point, with its category. The whole pill — glyph
/// lead and padding cells included — is the hit region.
pub(crate) fn chip_at(
    rect: Rect,
    x: u16,
    y: u16,
    values: EntryMetadataValues<'_>,
) -> Option<(PillCategory, String)> {
    let index = chip_index_at(rect, x, y, values)?;
    let (category, value) = chip_items(values).nth(index)?;
    Some((category, value.to_string()))
}

/// The index (into [`chip_items`]'s order) of the chip under the given point —
/// the identity a hover highlight tracks, sharing [`chip_at`]'s geometry so
/// the two can never disagree about which pill the cursor sits on.
pub(crate) fn chip_index_at(
    rect: Rect,
    x: u16,
    y: u16,
    values: EntryMetadataValues<'_>,
) -> Option<usize> {
    if !point_in_rect(rect, x, y) {
        return None;
    }
    let widths: Vec<u16> = chip_items(values)
        .map(|(_, value)| UnicodeWidthStr::width(value).min(u16::MAX as usize) as u16)
        .collect();
    let display_row = y.saturating_sub(rect.y) as usize;
    // Chip rows sit on even display offsets; the blank spacer rows between them
    // are odd and click nothing.
    if !display_row.is_multiple_of(2) {
        return None;
    }
    let rows = chip_rows(rect.width, values);
    let row = rows.get(display_row / 2)?;
    let mut x_pos = rect.x;
    for index in row {
        let cells = pill_cells(widths[*index]);
        if x >= x_pos && x < x_pos.saturating_add(cells) {
            return Some(*index);
        }
        x_pos = x_pos.saturating_add(cells).saturating_add(PILL_SEPARATOR);
    }
    None
}

/// Flow items into display rows: greedy, never splitting an item, the first
/// row shortened by `prefix_width`. Each row is a list of indices into the
/// width iterator's order; `item_pad` extra cells per item, `separator` cells
/// between two.
fn flow_by_widths(
    prefix_width: u16,
    row_width: u16,
    widths: impl Iterator<Item = usize>,
    item_pad: u16,
    separator: u16,
) -> Vec<Vec<usize>> {
    let available = row_width as usize;
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut row_width = prefix_width as usize;

    for (index, width) in widths.enumerate() {
        let item_width = width + item_pad as usize;
        let separator_width = if row.is_empty() {
            0
        } else {
            separator as usize
        };
        if !row.is_empty() && row_width + separator_width + item_width > available {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        if !row.is_empty() {
            row_width += separator as usize;
        }
        row_width += item_width;
        row.push(index);
    }

    if !row.is_empty() {
        rows.push(row);
    }

    rows
}

/// The classic `" | "`-separated row flow, kept for the feelings dialog's
/// "Selected: …" footer.
pub(crate) fn metadata_value_rows(
    prefix_width: u16,
    row_width: u16,
    values: &[String],
) -> Vec<Vec<usize>> {
    flow_by_widths(
        prefix_width,
        row_width,
        values
            .iter()
            .map(|value| UnicodeWidthStr::width(value.as_str())),
        0,
        3,
    )
}

pub(crate) fn metadata_section_height(row_width: u16, values: EntryMetadataValues<'_>) -> u16 {
    let blocks = [
        env_strip_height(row_width, values.environment),
        values.mood.is_some() as u16,
        chips_height(row_width, values),
    ];
    let present = blocks.iter().filter(|height| **height > 0).count() as u16;
    if present == 0 {
        return 0;
    }
    // The rule, the blocks, and one gap between each pair of present blocks.
    1 + blocks.iter().sum::<u16>() + METADATA_ROW_GAP * (present - 1)
}
