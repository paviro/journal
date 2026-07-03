use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, List, ListItem},
};

use crate::tui::{
    app::{App, Focus},
    render::{PanelGeometry, list_state_for_render, panel_block, render_scrollbar_if_needed},
    state::normalize_list_state,
};

pub(crate) fn draw_journals(frame: &mut Frame<'_>, geometry: PanelGeometry, app: &mut App) {
    let focused = app.focus == Focus::Journals;
    let highlight_active = app.focus != Focus::Entries;
    let block = panel_block("Journals", focused, None);
    let viewport_height = geometry.content.height;

    normalize_list_state(&mut app.journal_list, app.journals.len());
    let max_offset = app.journals.len().saturating_sub(viewport_height as usize);
    let offset = app.journal_list.offset().min(max_offset);
    *app.journal_list.offset_mut() = offset;

    let items: Vec<ListItem> = app
        .journals
        .iter()
        .map(|journal| ListItem::new(Line::from(Span::raw(journal.name.clone()))))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_spacing(HighlightSpacing::Never);

    let mut render_state = list_state_for_render(
        app.journal_list.selected(),
        offset,
        viewport_height,
        highlight_active,
    );

    frame.render_widget(block, geometry.area);
    frame.render_stateful_widget(list, geometry.content, &mut render_state);
    render_scrollbar_if_needed(
        frame,
        geometry.area,
        app.journals.len(),
        viewport_height,
        offset as u16,
    );
}
