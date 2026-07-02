use ratatui::layout::{Constraint, Direction, Layout, Rect};

use super::entry_rows::EntryRowMeta;
use super::render::panel_content_inner;

pub(crate) fn panel_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(crate) fn point_in_rect(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x
        && x < area.x.saturating_add(area.width)
        && y >= area.y
        && y < area.y.saturating_add(area.height)
}

pub(crate) fn journal_index_at(
    area: Rect,
    x: u16,
    y: u16,
    scroll: u16,
    journal_count: usize,
) -> Option<usize> {
    let inner = panel_inner(area);
    if !point_in_rect(inner, x, y) {
        return None;
    }

    let index = scroll as usize + y.saturating_sub(inner.y) as usize;
    (index < journal_count).then_some(index)
}

pub(crate) fn entry_index_at(
    area: Rect,
    x: u16,
    y: u16,
    scroll: u16,
    rows: &[EntryRowMeta],
) -> Option<usize> {
    let inner = panel_inner(area);
    if !point_in_rect(inner, x, y) {
        return None;
    }

    let target_y = scroll as usize + y.saturating_sub(inner.y) as usize;
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

fn metadata_section_height(tags: &[String], feelings: &[String]) -> u16 {
    let rows = (!feelings.is_empty()) as u16 + (!tags.is_empty()) as u16;
    if rows == 0 { 0 } else { 1 + rows }
}

pub(crate) fn tag_at_point(
    entry_view_area: Rect,
    x: u16,
    y: u16,
    tags: &[String],
    feelings: &[String],
) -> Option<String> {
    if tags.is_empty() {
        return None;
    }

    let inner = panel_content_inner(panel_inner(entry_view_area));
    let metadata_height = metadata_section_height(tags, feelings);
    if inner.height <= metadata_height {
        return None;
    }

    let metadata_rect = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(metadata_height)])
        .split(inner)[1];

    let tags_y = metadata_rect.y + 1 + (!feelings.is_empty()) as u16;
    if y != tags_y {
        return None;
    }

    let prefix = "Tags: ";
    let mut x_pos = metadata_rect.x;
    if x < x_pos + prefix.len() as u16 {
        return None;
    }
    x_pos += prefix.len() as u16;

    for tag in tags {
        let tag_width = tag.len() as u16;
        if x >= x_pos && x < x_pos + tag_width {
            return Some(tag.clone());
        }
        x_pos += tag_width + 3;
    }

    None
}

pub(crate) fn feeling_at_point(
    entry_view_area: Rect,
    x: u16,
    y: u16,
    tags: &[String],
    feelings: &[String],
) -> Option<String> {
    if feelings.is_empty() {
        return None;
    }

    let inner = panel_content_inner(panel_inner(entry_view_area));
    let metadata_height = metadata_section_height(tags, feelings);
    if inner.height <= metadata_height {
        return None;
    }

    let metadata_rect = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(metadata_height)])
        .split(inner)[1];

    let feelings_y = metadata_rect.y + 1;
    if y != feelings_y {
        return None;
    }

    let prefix = "Feelings: ";
    let mut x_pos = metadata_rect.x;
    if x < x_pos + prefix.len() as u16 {
        return None;
    }
    x_pos += prefix.len() as u16;

    for feeling in feelings {
        let width = feeling.len() as u16;
        if x >= x_pos && x < x_pos + width {
            return Some(feeling.clone());
        }
        x_pos += width + 3;
    }

    None
}
