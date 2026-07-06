use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, List},
};

use crate::tui::{
    app::{App, Focus, Mode},
    entry_rows::visible_entry_items,
    render::{
        EntryListGeometry, clamp_scroll, count_label, list_state_for_render, panel_block,
        render_centered_notice, render_scrollbar_if_needed,
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
    let cache = app.entry_rows(text_width);
    let viewport_height = geometry.viewport_height;
    let total_height = cache.total_height;
    let pixel_offset = clamp_scroll(app.entry_list.offset(), total_height, viewport_height);
    *app.entry_list.offset_mut() = pixel_offset;

    // In search mode, show the live query on the panel's top-right border so it
    // reads as the search field, rather than tucking it into the footer.
    if app.mode == Mode::Search {
        block = block.title(search_field_title(app).right_aligned());
    }

    // iOS-style sticky section header: once a month's divider scrolls above the
    // viewport, pin that month's label to the panel's top-right border so the
    // current month stays visible while browsing.
    if let Some(month) = sticky_month_label(
        &cache.month_sections,
        app.mode == Mode::Browse,
        pixel_offset,
    ) {
        block = block.title(Line::from(format!(" {month} ")).right_aligned());
    }

    let highlight_active = app.entries_highlighted();
    let (items, selected_visible) = visible_entry_items(
        &cache.rows,
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

    // An empty list in search mode — whether the query is blank or simply
    // matches nothing — gets a centered notice so the column doesn't read as a
    // rendering glitch.
    if app.mode == Mode::Search && cache.rows.is_empty() {
        render_centered_notice(frame, geometry.panel.content, "No results");
    }
}

/// The search query drawn on the panel's top-right border. While the field is
/// the active focus it carries a blinking block caret (`search_cursor_visible`)
/// at the edit position; once focus moves off the field the caret is hidden and
/// only the query text remains.
fn search_field_title(app: &App) -> Line<'static> {
    let show_caret = app.is_search_input_active();
    let caret_style = if app.search_cursor_visible {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    if app.search.query.is_empty() {
        let mut spans = vec![Span::raw(" ")];
        if show_caret {
            spans.push(Span::styled(" ", caret_style));
        }
        spans.push(Span::styled(
            "type to search ",
            Style::default().add_modifier(Modifier::DIM),
        ));
        return Line::from(spans);
    }

    if !show_caret {
        return Line::from(format!(" {} ", app.search.query));
    }

    let chars: Vec<char> = app.search.query.chars().collect();
    let cursor = app.search.cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let mut spans = vec![Span::raw(" "), Span::raw(before)];
    if cursor < chars.len() {
        spans.push(Span::styled(chars[cursor].to_string(), caret_style));
        let after: String = chars[cursor + 1..].iter().collect();
        spans.push(Span::raw(after));
    } else {
        // Caret sits past the last char: draw it as an inverted trailing block.
        spans.push(Span::styled(" ", caret_style));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// The month label to pin on the panel border. The first month rides the border
/// from the start (its divider is replaced by a leading blank line); each later
/// month takes over only once its `Month Year` divider has scrolled strictly
/// above the viewport, so the in-list divider and the border label are never
/// shown at once. `None` outside browse mode or when there are no entries.
fn sticky_month_label(
    sections: &[(usize, String)],
    is_browse: bool,
    offset: usize,
) -> Option<String> {
    if !is_browse {
        return None;
    }

    // The latest month whose divider has scrolled above the top, falling back to
    // the first month (which owns the border before anything scrolls past).
    sections
        .iter()
        .rev()
        .find(|(start, _)| *start < offset)
        .or_else(|| sections.first())
        .map(|(_, label)| label.clone())
}
