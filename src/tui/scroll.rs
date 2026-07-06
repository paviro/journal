use ratatui::layout::Rect;

pub(crate) fn viewer_scroll(requested: u16, line_count: usize, height: u16) -> u16 {
    let max_scroll = line_count
        .saturating_sub(height as usize)
        .min(u16::MAX as usize) as u16;
    requested.min(max_scroll)
}

pub(crate) fn scrollbar_position(scroll: usize, line_count: usize, height: u16) -> usize {
    let max_scroll = line_count.saturating_sub(height as usize);
    if max_scroll == 0 {
        return 0;
    }

    scroll
        .saturating_mul(line_count.saturating_sub(1))
        .checked_div(max_scroll)
        .unwrap_or(0)
}

/// The interactive vertical-scrollbar track for a panel `area`, matching where
/// `render_vertical_scrollbar` draws it: the panel's rightmost column, inset by one
/// row top and bottom (`Margin { vertical: 1 }`). A zero-height rect means there is
/// no draggable track.
pub(crate) fn scrollbar_bar_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(area.width.saturating_sub(1)),
        y: area.y.saturating_add(1),
        width: 1,
        height: area.height.saturating_sub(2),
    }
}

/// Map a mouse row on a scrollbar track to a scroll offset — the inverse of
/// `scrollbar_position`. The row is clamped into the track, so pressing at the very
/// top yields `0` and the very bottom yields `max_scroll`.
pub(crate) fn scroll_from_bar_row(
    row: u16,
    bar_top: u16,
    bar_height: u16,
    max_scroll: usize,
) -> usize {
    if max_scroll == 0 || bar_height <= 1 {
        return 0;
    }
    let span = (bar_height - 1) as usize;
    let bottom = bar_top.saturating_add(bar_height - 1);
    let rel = row.clamp(bar_top, bottom).saturating_sub(bar_top) as usize;
    // Round to nearest so the midpoint of the track lands near the middle of the range.
    (rel * max_scroll + span / 2) / span
}

/// Clamp an entry-list pixel scroll offset. Offsets are `usize` here (not `u16`)
/// so tall lists — thousands of multi-row entry boxes exceeding 65535 rows — can
/// still scroll all the way to the bottom.
pub(crate) fn clamp_scroll(requested: usize, total_height: usize, viewport_height: u16) -> usize {
    let max_scroll = total_height.saturating_sub(viewport_height as usize);
    requested.min(max_scroll)
}
