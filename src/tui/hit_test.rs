use ratatui::layout::Rect;

use super::entry_rows::RowMeta;
use super::surface::{
    EntryListGeometry, EntryMetadataValues, entry_metadata_layout, metadata_item_at, point_in_rect,
};

/// Maps a point in the journal panel's content to the journal index under it, or
/// `None` for the leading offset, the "Archived" divider row, or empty space. The
/// journal column uses the same pixel-row model as the entry list, so `meta`
/// carries per-row heights and `scroll` is a pixel offset.
pub(crate) fn journal_index_at(
    content: Rect,
    x: u16,
    y: u16,
    scroll: usize,
    meta: &[RowMeta],
) -> Option<usize> {
    let list = super::render::journal_list_rect(content);
    if !point_in_rect(list, x, y) {
        return None;
    }

    let target_y = scroll + y.saturating_sub(list.y) as usize;
    let mut row_y = 0usize;
    for row in meta {
        let next_y = row_y.saturating_add(row.height as usize);
        if target_y < next_y {
            return row.item_index;
        }
        row_y = next_y;
    }
    None
}

pub(crate) fn entry_index_at(
    geometry: EntryListGeometry,
    x: u16,
    y: u16,
    scroll: usize,
    rows: &[RowMeta],
) -> Option<usize> {
    if !point_in_rect(geometry.panel.content, x, y) {
        return None;
    }

    let target_y = scroll + y.saturating_sub(geometry.panel.content.y) as usize;
    let mut row_y = 0usize;
    for row in rows {
        let next_y = row_y.saturating_add(row.height as usize);
        if target_y < next_y {
            return row.item_index;
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
    reader_area: Rect,
    x: u16,
    y: u16,
    values: EntryMetadataValues<'_>,
) -> Option<(MetadataChip, String)> {
    let layout = entry_metadata_layout(reader_area, values);
    [
        (MetadataChip::Feelings, layout.feelings, values.feelings),
        (MetadataChip::People, layout.people, values.people),
        (
            MetadataChip::Activities,
            layout.activities,
            values.activities,
        ),
        (MetadataChip::Tags, layout.tags, values.tags),
    ]
    .into_iter()
    .find_map(|(chip, row, items)| metadata_item_at(row?, x, y, items).map(|value| (chip, value)))
}
