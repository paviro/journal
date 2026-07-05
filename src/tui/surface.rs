use ratatui::layout::{Constraint, Direction, Layout, Rect};
use unicode_width::UnicodeWidthStr;

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
        let content = panel_content_inner(inner);
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
pub(crate) struct MetadataRowLayout {
    pub(crate) rect: Rect,
    pub(crate) prefix_width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryMetadataLayout {
    pub(crate) content: Rect,
    pub(crate) metadata: Option<Rect>,
    pub(crate) mood: Option<Rect>,
    pub(crate) feelings: Option<MetadataRowLayout>,
    pub(crate) tags: Option<MetadataRowLayout>,
    pub(crate) people: Option<MetadataRowLayout>,
    pub(crate) activities: Option<MetadataRowLayout>,
}

#[derive(Clone, Copy)]
pub(crate) struct EntryMetadataValues<'a> {
    pub(crate) tags: &'a [String],
    pub(crate) people: &'a [String],
    pub(crate) activities: &'a [String],
    pub(crate) feelings: &'a [String],
    pub(crate) mood: Option<i8>,
}

pub(crate) fn panel_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(crate) fn panel_content_inner(area: Rect) -> Rect {
    let pad = 1;
    Rect {
        x: area.x.saturating_add(pad),
        width: area.width.saturating_sub(pad * 2).max(1),
        ..area
    }
}

pub(crate) fn point_in_rect(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x
        && x < area.x.saturating_add(area.width)
        && y >= area.y
        && y < area.y.saturating_add(area.height)
}

pub(crate) fn entry_metadata_layout(
    entry_view_area: Rect,
    values: EntryMetadataValues<'_>,
) -> EntryMetadataLayout {
    let inner = PanelGeometry::new(entry_view_area).content;
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

    let mut mood_rect = None;
    let mut feelings_row = None;
    let mut tags_row = None;
    let mut people_row = None;
    let mut activities_row = None;

    if let Some(metadata_rect) = metadata {
        let mut y = metadata_rect.y.saturating_add(1);
        if values.mood.is_some() {
            mood_rect = Some(Rect {
                y,
                height: 1,
                ..metadata_rect
            });
            y = y.saturating_add(1);
        }
        if !values.feelings.is_empty() {
            let height = metadata_row_height(
                "Feelings: ".len() as u16,
                metadata_rect.width,
                values.feelings,
            );
            feelings_row = Some(MetadataRowLayout {
                rect: Rect {
                    y,
                    height,
                    ..metadata_rect
                },
                prefix_width: "Feelings: ".len() as u16,
            });
            y = y.saturating_add(height);
        }
        if !values.people.is_empty() {
            let height =
                metadata_row_height("People: ".len() as u16, metadata_rect.width, values.people);
            people_row = Some(MetadataRowLayout {
                rect: Rect {
                    y,
                    height,
                    ..metadata_rect
                },
                prefix_width: "People: ".len() as u16,
            });
            y = y.saturating_add(height);
        }
        if !values.activities.is_empty() {
            let height = metadata_row_height(
                "Activities: ".len() as u16,
                metadata_rect.width,
                values.activities,
            );
            activities_row = Some(MetadataRowLayout {
                rect: Rect {
                    y,
                    height,
                    ..metadata_rect
                },
                prefix_width: "Activities: ".len() as u16,
            });
            y = y.saturating_add(height);
        }
        if !values.tags.is_empty() {
            let height =
                metadata_row_height("Tags: ".len() as u16, metadata_rect.width, values.tags);
            tags_row = Some(MetadataRowLayout {
                rect: Rect {
                    y,
                    height,
                    ..metadata_rect
                },
                prefix_width: "Tags: ".len() as u16,
            });
        }
    }

    EntryMetadataLayout {
        content,
        metadata,
        mood: mood_rect,
        feelings: feelings_row,
        tags: tags_row,
        people: people_row,
        activities: activities_row,
    }
}

pub(crate) fn metadata_item_at(
    row: MetadataRowLayout,
    x: u16,
    y: u16,
    values: &[String],
) -> Option<String> {
    if y < row.rect.y || y >= row.rect.y.saturating_add(row.rect.height) || values.is_empty() {
        return None;
    }

    let row_index = y.saturating_sub(row.rect.y) as usize;
    let rows = metadata_value_rows(row.prefix_width, row.rect.width, values);
    let value_indices = rows.get(row_index)?;
    let mut x_pos = row.rect.x;
    if row_index == 0 {
        x_pos = x_pos.saturating_add(row.prefix_width);
    }
    if x < x_pos {
        return None;
    }

    for index in value_indices {
        let value = &values[*index];
        let width = UnicodeWidthStr::width(value.as_str()).min(u16::MAX as usize) as u16;
        if x >= x_pos && x < x_pos.saturating_add(width) {
            return Some(value.clone());
        }
        x_pos = x_pos.saturating_add(width).saturating_add(3);
    }

    None
}

pub(crate) fn metadata_value_rows(
    prefix_width: u16,
    row_width: u16,
    values: &[String],
) -> Vec<Vec<usize>> {
    let available = row_width as usize;
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut row_width = prefix_width as usize;

    for (index, value) in values.iter().enumerate() {
        let value_width = UnicodeWidthStr::width(value.as_str());
        let separator_width = if row.is_empty() { 0 } else { 3 };
        if !row.is_empty() && row_width + separator_width + value_width > available {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        if !row.is_empty() {
            row_width += 3;
        }
        row_width += value_width;
        row.push(index);
    }

    if !row.is_empty() {
        rows.push(row);
    }

    rows
}

fn metadata_row_height(prefix_width: u16, row_width: u16, values: &[String]) -> u16 {
    metadata_value_rows(prefix_width, row_width, values)
        .len()
        .max(1)
        .min(u16::MAX as usize) as u16
}

fn metadata_section_height(row_width: u16, values: EntryMetadataValues<'_>) -> u16 {
    let rows = values.mood.is_some() as u16
        + (!values.feelings.is_empty() as u16)
            * metadata_row_height("Feelings: ".len() as u16, row_width, values.feelings)
        + (!values.people.is_empty() as u16)
            * metadata_row_height("People: ".len() as u16, row_width, values.people)
        + (!values.activities.is_empty() as u16)
            * metadata_row_height("Activities: ".len() as u16, row_width, values.activities)
        + (!values.tags.is_empty() as u16)
            * metadata_row_height("Tags: ".len() as u16, row_width, values.tags);
    if rows == 0 { 0 } else { 1 + rows }
}
