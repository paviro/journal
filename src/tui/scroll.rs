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

/// The thumb's rows within a scrollbar `bar`, replicating ratatui's `Scrollbar`
/// layout so mouse hit-testing lines up with what is drawn. The bar's first and last
/// rows are the up/down arrows, and the thumb travels over the `bar.height - 2` track
/// rows between them. Returns `(thumb_top, thumb_len)` in absolute rows, or `None`
/// when the bar is too short to host a track.
pub(crate) fn scrollbar_thumb(
    bar: Rect,
    content_length: usize,
    viewport_length: u16,
    position: usize,
) -> Option<(u16, u16)> {
    let track_len = bar.height.checked_sub(2)?;
    if track_len == 0 || content_length == 0 {
        return None;
    }
    // Mirrors `Scrollbar::part_lengths`: the thumb spans the viewport's share of the
    // content, positioned proportionally to `position` within the track.
    let track = f64::from(track_len);
    let viewport = f64::from(viewport_length);
    let max_position = content_length.saturating_sub(1) as f64;
    let start = (position as f64).clamp(0.0, max_position);
    let max_viewport_position = max_position + viewport;
    let thumb_start = (start * track / max_viewport_position)
        .round()
        .clamp(0.0, track - 1.0) as u16;
    let thumb_end = ((start + viewport) * track / max_viewport_position)
        .round()
        .clamp(0.0, track) as u16;
    let thumb_len = thumb_end.saturating_sub(thumb_start).max(1);
    Some((bar.y.saturating_add(1).saturating_add(thumb_start), thumb_len))
}

/// Map a desired thumb-top row to a scroll offset — the inverse of the thumb
/// placement, used while dragging so the grabbed point of the thumb follows the
/// cursor. `track_top` is `bar.y + 1` (first row below the up arrow) and `track_len`
/// is `bar.height - 2`. The thumb can travel over `track_len - thumb_len` rows.
pub(crate) fn scroll_from_thumb_top(
    thumb_top: u16,
    track_top: u16,
    track_len: u16,
    thumb_len: u16,
    max_scroll: usize,
) -> usize {
    let travel = track_len.saturating_sub(thumb_len);
    if travel == 0 || max_scroll == 0 {
        return 0;
    }
    let start = thumb_top.saturating_sub(track_top).min(travel) as usize;
    // Round to nearest so the thumb's ends map cleanly to 0 and `max_scroll`.
    (start * max_scroll + travel as usize / 2) / travel as usize
}

/// Clamp an entry-list pixel scroll offset. Offsets are `usize` here (not `u16`)
/// so tall lists — thousands of multi-row entry boxes exceeding 65535 rows — can
/// still scroll all the way to the bottom.
pub(crate) fn clamp_scroll(requested: usize, total_height: usize, viewport_height: u16) -> usize {
    let max_scroll = total_height.saturating_sub(viewport_height as usize);
    requested.min(max_scroll)
}
