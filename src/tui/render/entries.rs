use ratatui::{
    Frame,
    style::{Modifier, Style},
    widgets::{HighlightSpacing, List},
};

use crate::tui::{
    app::{App, Focus, Mode},
    entry_rows::{entry_list_rows, visible_entry_items},
    render::{
        EntryListGeometry, clamp_scroll, entry_row_metadata, list_state_for_render, panel_block,
        render_scrollbar_if_needed, total_entry_row_height,
    },
};

pub(crate) fn draw_entry_list(frame: &mut Frame<'_>, geometry: EntryListGeometry, app: &mut App) {
    let focused = app.focus == Focus::Entries;
    let block = panel_block(
        match app.mode {
            Mode::Search => "Search",
            Mode::Browse => "Entries",
        },
        focused,
        None,
    );
    let text_width = geometry.text_width;
    let rows = entry_list_rows(app, text_width);
    let viewport_height = geometry.viewport_height;
    let meta = entry_row_metadata(app, text_width);
    let total_height = total_entry_row_height(&meta);
    let pixel_offset = clamp_scroll(
        app.entry_list.offset() as u16,
        total_height,
        viewport_height,
    );
    *app.entry_list.offset_mut() = pixel_offset as usize;

    let highlight_active = app.focus != Focus::Journals;
    let (items, selected_visible) = visible_entry_items(
        &rows,
        pixel_offset,
        viewport_height,
        app.selected_entry_index,
        highlight_active,
    );

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_spacing(HighlightSpacing::Never);

    let mut render_state =
        list_state_for_render(selected_visible, 0, viewport_height, highlight_active);

    frame.render_widget(block, geometry.panel.area);
    frame.render_stateful_widget(list, geometry.panel.content, &mut render_state);
    render_scrollbar_if_needed(
        frame,
        geometry.panel.area,
        total_height,
        viewport_height,
        pixel_offset,
    );
}
