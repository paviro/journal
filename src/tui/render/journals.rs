use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{List, ListItem},
};

use crate::tui::{
    app::{App, Focus},
    render::{clamp_scroll, panel_block, panel_content_inner, selected_style},
};

pub(crate) fn draw_journals(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Journals;
    let block = panel_block("Journals", focused);
    let inner = panel_content_inner(block.inner(area));
    let viewport_height = inner.height;
    app.scroll.journal = clamp_scroll(app.scroll.journal, app.journals.len(), viewport_height);
    let offset = app.scroll.journal as usize;
    let items: Vec<ListItem> = app
        .journals
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport_height as usize)
        .map(|(index, journal)| {
            let style = selected_style(index == app.selected_journal);
            ListItem::new(Line::from(Span::raw(&journal.name))).style(style)
        })
        .collect();

    frame.render_widget(block, area);
    frame.render_widget(List::new(items), inner);
}
