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

/// Clamp an entry-list pixel scroll offset. Offsets are `usize` here (not `u16`)
/// so tall lists — thousands of multi-row entry boxes exceeding 65535 rows — can
/// still scroll all the way to the bottom.
pub(crate) fn clamp_scroll(requested: usize, total_height: usize, viewport_height: u16) -> usize {
    let max_scroll = total_height.saturating_sub(viewport_height as usize);
    requested.min(max_scroll)
}
