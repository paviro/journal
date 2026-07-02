use ratatui::{
    Frame,
    layout::Rect,
    widgets::{List, ScrollbarState},
};

use crate::tui::{
    app::{App, Focus, Mode},
    entry_rows::{entry_list_rows, visible_entry_items},
    render::{
        clamp_scroll, entry_row_metadata, panel_block, panel_content_inner,
        render_vertical_scrollbar, scrollbar_position, total_entry_row_height,
    },
};

pub(crate) fn draw_entry_list(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Entries;
    let block = panel_block(
        match app.mode {
            Mode::Search => "Search",
            Mode::Browse => "Entries",
        },
        focused,
        None,
    );
    let inner = panel_content_inner(block.inner(area));
    let text_width = inner.width.saturating_sub(7);
    let rows = entry_list_rows(app, text_width);
    let viewport_height = inner.height;
    let meta = entry_row_metadata(app, text_width);
    let total_height = total_entry_row_height(&meta);
    app.scroll.entry = clamp_scroll(app.scroll.entry, total_height, viewport_height);
    let items = visible_entry_items(&rows, app.scroll.entry, viewport_height);

    frame.render_widget(block, area);
    frame.render_widget(List::new(items), inner);

    if total_height > viewport_height as usize {
        let mut state = ScrollbarState::default()
            .content_length(total_height)
            .viewport_content_length(viewport_height as usize)
            .position(scrollbar_position(
                app.scroll.entry,
                total_height,
                viewport_height,
            ));
        render_vertical_scrollbar(frame, area, &mut state);
    }
}
