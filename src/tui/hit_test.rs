use ratatui::layout::Rect;

use super::entry_rows::RowMeta;
use super::surface::{
    EntryListGeometry, EntryMetadataValues, chip_at, chip_index_at, entry_metadata_layout,
    point_in_rect,
};
use super::theme::PillCategory;

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
/// All categories share one label-less pill flow; the chip's position in it
/// carries the category.
pub(crate) fn metadata_at_point(
    reader_area: Rect,
    x: u16,
    y: u16,
    values: EntryMetadataValues<'_>,
) -> Option<(MetadataChip, String)> {
    let layout = entry_metadata_layout(reader_area, values);
    let (category, value) = chip_at(layout.chips?, x, y, values)?;
    let chip = match category {
        PillCategory::Feelings => MetadataChip::Feelings,
        PillCategory::People => MetadataChip::People,
        PillCategory::Activities => MetadataChip::Activities,
        PillCategory::Tags => MetadataChip::Tags,
    };
    Some((chip, value))
}

/// The flat chip index (into the pinned metadata's pill flow) under the given
/// point — the identity a hover highlight tracks. Shares
/// [`metadata_at_point`]'s layout so hover and click land on the same pill.
pub(crate) fn metadata_chip_index_at(
    reader_area: Rect,
    x: u16,
    y: u16,
    values: EntryMetadataValues<'_>,
) -> Option<usize> {
    let layout = entry_metadata_layout(reader_area, values);
    chip_index_at(layout.chips?, x, y, values)
}
