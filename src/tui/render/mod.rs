mod chrome;
mod dialogs;
mod entries;
mod image_viewer;
pub(crate) mod insights;
mod journals;
mod layout;
mod markdown_panel;
mod pending;
mod table;
mod unlock;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    widgets::{ListState, Paragraph},
};

use super::app::{App, EntryViewImageHits, single_panel_is_active};
use super::editor_state::EditorPrompt;
#[cfg(test)]
pub(crate) use super::entry_rows::entry_row_metadata;
#[cfg(test)]
pub(crate) use super::entry_rows::{
    RowMeta, entry_box_lines, entry_day_label, entry_list_lines, entry_month_label,
};
pub(crate) use super::hit_test::{
    MetadataChip, entry_index_at, journal_index_at, metadata_at_point,
};
#[cfg(test)]
pub(crate) use super::scroll::scrollbar_position;
pub(crate) use super::scroll::{clamp_scroll, scroll_pixels, viewer_scroll};
#[cfg(test)]
use super::scroll::{scroll_from_thumb_top, scrollbar_bar_rect, scrollbar_thumb};
pub(crate) use super::surface::{
    EntryListGeometry, EntryMetadataValues, PanelGeometry, entry_metadata_layout, panel_inner,
    point_in_rect,
};
pub(crate) use chrome::{
    Hint, HintId, MetadataChoice, MetadataMenuMode, centered_rect_fixed_size, confirm_button_at,
    count_label, draw_editor_discard_confirm, draw_editor_shortcuts, draw_metadata_menu,
    draw_modal_frame, editor_discard_choice_at_point, editor_shortcut_close_at_point,
    editor_shortcut_hint_at_point, expanded_footer_height, expanded_footer_hint_id_at_point,
    expanded_footer_lines, footer_hint_id_at_point, footer_lines, hint_id_at_wrapped,
    metadata_menu_choice_at_point, metadata_menu_close_at_point, panel_block,
    render_centered_notice, render_scrollbar_if_needed,
};
#[cfg(test)]
pub(crate) use chrome::{
    expanded_footer_text, footer_height, footer_hint_id_at, footer_text, hint_grid_text,
    hint_height,
};
pub(crate) use dialogs::{
    confirm_delete_inner, feelings_dialog_hints, feelings_dialog_layout,
    feelings_selected_line_count, location_dialog_hints, location_dialog_layout,
    location_list_row_at, location_list_rows, metadata_dialog_hints, metadata_dialog_layout,
    mood_dialog_hints, mood_dialog_layout,
};
use dialogs::{
    draw_confirm_delete, draw_edit_feelings_dialog, draw_edit_location_dialog,
    draw_edit_metadata_dialog, draw_edit_mood_dialog, draw_new_journal_input,
};
use entries::draw_entry_list;
use image_viewer::draw_image_viewer;
use insights::draw_journal_insights;
pub(crate) use insights::insights_tab_at;
use journals::draw_journals;
pub(crate) use journals::{JOURNAL_BOX_HEIGHT, journal_list_rect};
pub(crate) use layout::{TuiLayout, tui_layout};
use markdown_panel::{draw_entry_editor, draw_selected_entry_view};
pub(crate) use pending::{
    AccessNotice, draw_disable_notice, draw_pending_notice, draw_pending_request,
};
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
    // A field that isn't re-drawn this frame (its panel hidden by a fullscreen
    // pane) must not keep swallowing clicks at its stale coordinates; drawing
    // it below re-registers the rect.
    app.search.query.forget_area();
    let area = frame.area();

    // Cleared each frame; the entry-view render repopulates it when an entry is
    // shown, so a stale hit-map can't leak onto insights or empty views.
    app.entry_view_image_hits = EntryViewImageHits::default();

    if app.entry_view_is_fullscreen(area.width) {
        let footer_height = expanded_footer_height(app, area.width).min(area.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
            .split(area);
        if let Some(editor) = app.editor.as_mut() {
            // Single-column (the one-col viewer breakpoint) gets a tighter margin
            // than an expanded fullscreen editor on a wide terminal.
            let (side, top) = if single_panel_is_active(area.width) {
                (3, 1)
            } else {
                (5, 3)
            };
            draw_entry_editor(frame, chunks[0], editor, side, top);
        } else {
            draw_selected_entry_view(frame, chunks[0], app);
        }
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
    if let Some(area) = layout.insights {
        draw_journal_insights(frame, area.area, app);
    } else if let Some(area) = layout.entry_view {
        if let Some(editor) = app.editor.as_mut() {
            draw_entry_editor(frame, area.area, editor, 5, 3);
        } else if app.show_journal_insights_preview() {
            // With no entry selected, the preview pane shows the journal insights.
            draw_journal_insights(frame, area.area, app);
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

    if matches!(app.overlay, crate::tui::state::Overlay::MetadataMenu) {
        draw_metadata_menu(frame, MetadataMenuMode::Viewer);
    }

    if let Some(input) = app.new_journal_input_mut() {
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

    if let Some(state) = app.edit_location_state_mut() {
        draw_edit_location_dialog(frame, state);
    }

    if let Some(state) = app.image_viewer_state() {
        draw_image_viewer(frame, state, &app.image.runtime);
    }

    if let Some(editor) = app.editor.as_mut() {
        match &mut editor.prompt {
            EditorPrompt::MetadataMenu => draw_metadata_menu(frame, MetadataMenuMode::Editor),
            EditorPrompt::Help { scroll } => draw_editor_shortcuts(frame, scroll),
            EditorPrompt::ConfirmDiscard => draw_editor_discard_confirm(frame),
            EditorPrompt::None => {}
        }
    }
}

#[cfg(test)]
mod tests;
