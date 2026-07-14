use ratatui::{
    Frame,
    layout::Rect,
    widgets::{HighlightSpacing, List},
};

use crate::tui::{
    app::{App, Focus, Mode, SearchScope},
    entry_rows::{total_row_height, visible_box_items},
    render::{
        PanelGeometry, clamp_scroll, count_label, flat_chrome, list_state_for_render, panel_block,
        render_centered_notice, render_scrollbar_if_needed,
    },
    theme::theme,
};

/// Rows occupied by one journal's bordered box (top border, name, bottom border).
pub(crate) const JOURNAL_BOX_HEIGHT: u16 = 3;

/// Rows per journal row in the current chrome. Flat cards keep the box's three
/// rows (all background-filled, name centered) and add a blank separator row
/// so adjacent cards read as distinct blocks. Uniform per chrome, so
/// `journal_row_top` stays a plain multiply.
pub(crate) fn journal_row_height() -> u16 {
    if flat_chrome() {
        JOURNAL_BOX_HEIGHT + 1
    } else {
        JOURNAL_BOX_HEIGHT
    }
}

/// A blank row leads the journal boxes so the first one lines up with the first
/// entry box, which sits one row below the entry list's month divider.
pub(crate) const JOURNAL_LIST_TOP_OFFSET: u16 = 1;

/// The rect the journal boxes are drawn into: the panel content shifted down by
/// the leading offset. Shared by rendering and hit-testing so they stay in sync.
pub(crate) fn journal_list_rect(content: Rect) -> Rect {
    Rect {
        y: content.y.saturating_add(JOURNAL_LIST_TOP_OFFSET),
        height: content.height.saturating_sub(JOURNAL_LIST_TOP_OFFSET),
        ..content
    }
}

pub(crate) fn draw_journals(frame: &mut Frame<'_>, geometry: PanelGeometry, app: &mut App) {
    let focused = app.nav.focus == Focus::Journals;
    // An all-journals search covers everything, so highlight every journal
    // rather than implying it's scoped to the selected one. A journal-scoped
    // search keeps the single highlight.
    let select_all = app.nav.mode == Mode::Search && app.search.scope == SearchScope::AllJournals;
    // Flat chrome bakes selection (and the all-journals flood) into the chip
    // lines themselves; only bordered mode drives the List highlight.
    let styles_baked = flat_chrome();
    let highlight_active = !select_all && !styles_baked;
    // Archived journals are still journals, so the panel count includes them; the
    // "Archived" divider marks the split within the list.
    let block = panel_block(
        "Journals",
        focused,
        Some(count_label(
            app.library.journals.len(),
            "journal",
            "journals",
        )),
    );
    app.normalize_journal_selection();

    let (rows, meta, list_area) = app.journal_rows(geometry.content);
    let viewport_height = list_area.height;
    let total_height = total_row_height(&meta);
    let pixel_offset = clamp_scroll(app.nav.journal_list.offset(), total_height, viewport_height);
    *app.nav.journal_list.offset_mut() = pixel_offset;

    let highlight_style = theme().selection();
    let (items, selected_visible, item_indices) = visible_box_items(
        &rows,
        pixel_offset,
        viewport_height,
        app.nav.journal_list.selected(),
        highlight_active,
    );
    // An all-journals search highlights every journal box to signal the wide
    // scope (the single-selection highlight is suppressed via `highlight_active`).
    // The "Archived" divider isn't a journal, so it's left unhighlighted.
    let items: Vec<_> = if select_all && !styles_baked {
        items
            .into_iter()
            .zip(&item_indices)
            .map(|(item, index)| {
                if index.is_some() {
                    item.style(highlight_style)
                } else {
                    item
                }
            })
            .collect()
    } else {
        items
    };

    let list = List::new(items)
        .highlight_style(highlight_style)
        .highlight_spacing(HighlightSpacing::Never);

    let mut render_state =
        list_state_for_render(selected_visible, 0, viewport_height, highlight_active);

    frame.render_widget(block, geometry.area);
    super::panel_focus_stripe(frame, geometry.area, focused);
    frame.render_stateful_widget(list, list_area, &mut render_state);
    render_scrollbar_if_needed(
        frame,
        geometry.area,
        total_height,
        viewport_height,
        pixel_offset,
        focused,
    );

    // With no journals the column would otherwise be blank; a centered notice
    // matches the overview and entry list so it reads as intentional.
    if app.library.journals.is_empty() {
        render_centered_notice(frame, geometry.content, "No journals");
    }
}
