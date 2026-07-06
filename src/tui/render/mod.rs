mod chrome;
mod dialogs;
mod entries;
mod image_viewer;
mod journals;
mod layout;
mod markdown_panel;
pub(crate) mod stats;
mod unlock;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    widgets::{ListState, Paragraph},
};

use super::app::{App, EntryViewImageHits, Focus, single_panel_is_active};
#[cfg(test)]
pub(crate) use super::entry_rows::entry_row_metadata;
#[cfg(test)]
pub(crate) use super::entry_rows::{
    EntryRowMeta, entry_box_lines, entry_day_label, entry_list_lines, entry_month_label,
};
#[cfg(test)]
pub(crate) use super::hit_test::journal_index_at;
pub(crate) use super::hit_test::{MetadataChip, entry_index_at, metadata_at_point};
#[cfg(test)]
pub(crate) use super::scroll::scrollbar_position;
pub(crate) use super::scroll::{clamp_scroll, viewer_scroll};
#[cfg(test)]
use super::scroll::{scroll_from_thumb_top, scrollbar_bar_rect, scrollbar_thumb};
pub(crate) use super::surface::{
    EntryListGeometry, EntryMetadataValues, PanelGeometry, entry_metadata_layout, panel_inner,
    point_in_rect,
};
pub(crate) use chrome::{
    HintId, centered_rect_fixed_size, count_label, expanded_footer_height,
    expanded_footer_hint_id_at_point, expanded_footer_lines, footer_hint_id_at_point, footer_lines,
    hint_id_at_wrapped, panel_block, render_centered_notice, render_scrollbar_if_needed,
};
#[cfg(test)]
pub(crate) use chrome::{
    expanded_footer_text, footer_height, footer_hint_id_at, footer_text, hint_height, hint_id_at,
};
use dialogs::{
    draw_confirm_delete, draw_edit_feelings_dialog, draw_edit_metadata_dialog,
    draw_edit_mood_dialog, draw_new_journal_input,
};
pub(crate) use dialogs::{
    feelings_dialog_hints, feelings_dialog_layout, metadata_dialog_hints, metadata_dialog_layout,
    mood_dialog_hints, mood_dialog_layout,
};
use entries::draw_entry_list;
use image_viewer::draw_image_viewer;
use journals::draw_journals;
pub(crate) use journals::{JOURNAL_BOX_HEIGHT, journal_list_rect, journals_per_page};
pub(crate) use layout::{TuiLayout, tui_layout};
use markdown_panel::draw_selected_entry_view;
use stats::draw_journal_stats;
#[cfg(test)]
pub(crate) use stats::{centered_stats_layout, journal_stats};
pub(crate) use unlock::draw_unlock;

pub(crate) fn list_state_for_render(
    selected: Option<usize>,
    offset: usize,
    viewport_height: u16,
    highlight_active: bool,
) -> ListState {
    let visible_end = offset.saturating_add(viewport_height as usize);
    let visible_selection =
        selected.filter(|index| highlight_active && *index >= offset && *index < visible_end);
    ListState::default()
        .with_offset(offset)
        .with_selected(visible_selection)
}

pub(crate) fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();

    // Cleared each frame; the entry-view render repopulates it when an entry is
    // shown, so a stale hit-map can't leak onto stats or empty views.
    app.entry_view_image_hits = EntryViewImageHits::default();

    if single_panel_is_active(area.width) && app.nav.focus == Focus::EntryView {
        let footer_height = expanded_footer_height(app, area.width).min(area.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
            .split(area);
        draw_selected_entry_view(frame, chunks[0], app);
        let footer_area = chunks[1];
        let footer_text_area = ratatui::layout::Rect {
            x: footer_area.x.saturating_add(1),
            width: footer_area.width.saturating_sub(1),
            ..footer_area
        };
        frame.render_widget(
            Paragraph::new(expanded_footer_lines(app, footer_area.width)),
            footer_text_area,
        );
        draw_overlays(frame, app);
        return;
    }

    let layout = tui_layout(area, app);

    if let Some(area) = layout.journals {
        draw_journals(frame, area, app);
    }
    if let Some(area) = layout.entries {
        draw_entry_list(frame, area, app);
    }
    if let Some(area) = layout.stats {
        draw_journal_stats(frame, area.area, app);
    } else if let Some(area) = layout.entry_view {
        // With no entry selected, the preview pane shows the journal stats.
        if app.show_journal_stats_preview() {
            draw_journal_stats(frame, area.area, app);
        } else {
            draw_selected_entry_view(frame, area.area, app);
        }
    }

    let footer = Paragraph::new(footer_lines(app, layout.footer.width));
    frame.render_widget(footer, layout.footer);

    draw_overlays(frame, app);
}

fn draw_overlays(frame: &mut Frame<'_>, app: &mut App) {
    if let crate::tui::state::Overlay::ConfirmDelete(ctx) = &app.overlay {
        draw_confirm_delete(frame, ctx);
    }

    if let Some(input) = app.new_journal_input() {
        draw_new_journal_input(frame, input);
    }

    if let Some(state) = app.edit_metadata_state_mut() {
        draw_edit_metadata_dialog(frame, state);
    }

    if let Some(state) = app.edit_feeling_state_mut() {
        draw_edit_feelings_dialog(frame, state);
    }

    if let Some(state) = app.edit_mood_state() {
        draw_edit_mood_dialog(frame, state);
    }

    if let Some(state) = app.image_viewer_state() {
        draw_image_viewer(frame, state, &app.image.runtime);
    }
}

#[cfg(test)]
mod tests;
