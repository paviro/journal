use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::Line,
    widgets::{HighlightSpacing, List},
};

use crate::tui::{
    app::{App, Focus, Mode},
    entry_rows::{entry_list_rows, entry_month_sections, visible_entry_items},
    render::{
        EntryListGeometry, clamp_scroll, count_label, entry_row_metadata, list_state_for_render,
        panel_block, render_scrollbar_if_needed, total_entry_row_height,
    },
};

pub(crate) fn draw_entry_list(frame: &mut Frame<'_>, geometry: EntryListGeometry, app: &mut App) {
    let focused = app.focus == Focus::Entries;
    let mut block = panel_block(
        match app.mode {
            Mode::Search => "Search",
            Mode::Browse => "Entries",
        },
        focused,
        Some(count_label(
            app.current_entry_list_len(),
            "entry",
            "entries",
        )),
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

    // iOS-style sticky section header: once a month's divider scrolls above the
    // viewport, pin that month's label to the panel's top-right border so the
    // current month stays visible while browsing.
    if let Some(month) = sticky_month_label(app, text_width, pixel_offset) {
        block = block.title(Line::from(format!(" {month} ")).right_aligned());
    }

    let highlight_active = app.entries_highlighted();
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

/// The month label to pin on the panel border. The first month rides the border
/// from the start (its divider is replaced by a leading blank line); each later
/// month takes over only once its `Month Year` divider has scrolled strictly
/// above the viewport, so the in-list divider and the border label are never
/// shown at once. `None` outside browse mode or when there are no entries.
fn sticky_month_label(app: &App, text_width: u16, pixel_offset: u16) -> Option<String> {
    if app.mode != Mode::Browse {
        return None;
    }

    let offset = pixel_offset as usize;
    let sections = entry_month_sections(app, text_width);
    // The latest month whose divider has scrolled above the top, falling back to
    // the first month (which owns the border before anything scrolls past).
    sections
        .iter()
        .rev()
        .find(|(start, _)| *start < offset)
        .or_else(|| sections.first())
        .map(|(_, label)| label.clone())
}
