use ratatui::layout::Rect;

use super::entry_rows::EntryRowMeta;
#[cfg(test)]
use super::surface::PanelGeometry;
use super::surface::{
    EntryListGeometry, EntryMetadataValues, entry_metadata_layout, metadata_item_at, point_in_rect,
};

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

    let list = super::render::journal_list_rect(geometry.content);
    let relative = y.checked_sub(list.y)?;
    if relative >= list.height {
        return None;
    }
    let index = scroll as usize + (relative / super::render::JOURNAL_BOX_HEIGHT) as usize;
    (index < journal_count).then_some(index)
}

pub(crate) fn entry_index_at(
    geometry: EntryListGeometry,
    x: u16,
    y: u16,
    scroll: usize,
    rows: &[EntryRowMeta],
) -> Option<usize> {
    if !point_in_rect(geometry.panel.content, x, y) {
        return None;
    }

    let target_y = scroll + y.saturating_sub(geometry.panel.content.y) as usize;
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

/// A clickable metadata chip in the entry view. Feelings is included alongside
/// the free-form kinds because both are clickable, even though only tags/people/
/// activities share the editing machinery (`MetadataKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataChip {
    Feelings,
    People,
    Activities,
    Tags,
}

/// Which metadata chip (if any) sits under the given point, and its value.
///
/// Chips are tested in row order (feelings first) so overlapping rows resolve
/// deterministically; each occupies a distinct row in practice.
pub(crate) fn metadata_at_point(
    entry_view_area: Rect,
    x: u16,
    y: u16,
    values: EntryMetadataValues<'_>,
) -> Option<(MetadataChip, String)> {
    let layout = entry_metadata_layout(entry_view_area, values);
    [
        (MetadataChip::Feelings, layout.feelings, values.feelings),
        (MetadataChip::People, layout.people, values.people),
        (MetadataChip::Activities, layout.activities, values.activities),
        (MetadataChip::Tags, layout.tags, values.tags),
    ]
    .into_iter()
    .find_map(|(chip, row, items)| {
        metadata_item_at(row?, x, y, items).map(|value| (chip, value))
    })
}
