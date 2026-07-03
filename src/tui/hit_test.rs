use ratatui::layout::Rect;

use super::entry_rows::EntryRowMeta;
#[cfg(test)]
use super::surface::PanelGeometry;
use super::surface::{EntryListGeometry, entry_metadata_layout, metadata_item_at, point_in_rect};

#[cfg(test)]
pub(crate) fn journal_index_at(
    geometry: PanelGeometry,
    x: u16,
    y: u16,
    scroll: u16,
    journal_count: usize,
) -> Option<usize> {
    if !point_in_rect(geometry.content, x, y) {
        return None;
    }

    let index = scroll as usize + y.saturating_sub(geometry.content.y) as usize;
    (index < journal_count).then_some(index)
}

pub(crate) fn entry_index_at(
    geometry: EntryListGeometry,
    x: u16,
    y: u16,
    scroll: u16,
    rows: &[EntryRowMeta],
) -> Option<usize> {
    if !point_in_rect(geometry.panel.content, x, y) {
        return None;
    }

    let target_y = scroll as usize + y.saturating_sub(geometry.panel.content.y) as usize;
    let mut row_y = 0usize;
    for row in rows {
        let next_y = row_y.saturating_add(row.height as usize);
        if target_y < next_y {
            return row.entry_index;
        }
        row_y = next_y;
    }
    None
}

pub(crate) fn tag_at_point(
    entry_view_area: Rect,
    x: u16,
    y: u16,
    tags: &[String],
    feelings: &[String],
    mood: Option<i8>,
) -> Option<String> {
    let layout = entry_metadata_layout(
        entry_view_area,
        !tags.is_empty(),
        !feelings.is_empty(),
        mood.is_some(),
    );
    metadata_item_at(layout.tags?, x, y, tags)
}

pub(crate) fn feeling_at_point(
    entry_view_area: Rect,
    x: u16,
    y: u16,
    tags: &[String],
    feelings: &[String],
    mood: Option<i8>,
) -> Option<String> {
    let layout = entry_metadata_layout(
        entry_view_area,
        !tags.is_empty(),
        !feelings.is_empty(),
        mood.is_some(),
    );
    metadata_item_at(layout.feelings?, x, y, feelings)
}
