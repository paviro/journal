use ratatui::{Frame, layout::Rect, widgets::List};

use crate::tui::{
    app::{App, Focus, Mode},
    entry_rows::{entry_list_rows, visible_entry_items},
    render::{
        clamp_scroll, entry_row_metadata, panel_block, panel_content_inner, total_entry_row_height,
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
    );
    let inner = panel_content_inner(block.inner(area));
    let rows = entry_list_rows(app);
    let viewport_height = inner.height;
    app.scroll.entry = clamp_scroll(
        app.scroll.entry,
        total_entry_row_height(&entry_row_metadata(app)),
        viewport_height,
    );
    let items = visible_entry_items(&rows, app.scroll.entry, viewport_height);

    frame.render_widget(block, area);
    frame.render_widget(List::new(items), inner);
}
