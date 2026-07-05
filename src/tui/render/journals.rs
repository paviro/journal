use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, List, ListItem},
};

use crate::tui::{
    app::{App, Focus},
    render::{
        PanelGeometry, count_label, list_state_for_render, panel_block, render_scrollbar_if_needed,
    },
    state::normalize_list_state,
};

/// Rows occupied by one journal's bordered box (top border, name, bottom border).
pub(crate) const JOURNAL_BOX_HEIGHT: u16 = 3;

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

/// How many journal boxes fit in a list of the given content height (at least one,
/// so navigation never stalls in a very short viewport).
pub(crate) fn journals_per_page(content_height: u16) -> u16 {
    (content_height / JOURNAL_BOX_HEIGHT).max(1)
}

pub(crate) fn draw_journals(frame: &mut Frame<'_>, geometry: PanelGeometry, app: &mut App) {
    let focused = app.focus == Focus::Journals;
    let highlight_active = true;
    let block = panel_block(
        "Journals",
        focused,
        Some(count_label(app.journals.len(), "journal", "journals")),
    );
    let list_area = journal_list_rect(geometry.content);
    let per_page = journals_per_page(list_area.height);

    normalize_list_state(&mut app.journal_list, app.journals.len());
    let max_offset = app.journals.len().saturating_sub(per_page as usize);
    let offset = app.journal_list.offset().min(max_offset);
    *app.journal_list.offset_mut() = offset;

    let inner_width = list_area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .journals
        .iter()
        .map(|journal| ListItem::new(journal_box_lines(&journal.name, inner_width)))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_spacing(HighlightSpacing::Never);

    let mut render_state = list_state_for_render(
        app.journal_list.selected(),
        offset,
        per_page,
        highlight_active,
    );

    frame.render_widget(block, geometry.area);
    frame.render_stateful_widget(list, list_area, &mut render_state);
    render_scrollbar_if_needed(
        frame,
        geometry.area,
        app.journals.len(),
        per_page,
        offset as u16,
    );
}

/// One journal rendered as a bordered box with the name inside, mirroring the
/// entry list. The name is truncated to fit the inner width.
fn journal_box_lines(name: &str, inner_width: usize) -> Vec<Line<'static>> {
    let border = Style::default().add_modifier(Modifier::DIM);
    let box_width = inner_width + 4;
    let rule = "─".repeat(box_width.saturating_sub(2));

    let content: String = name.chars().take(inner_width).collect();
    let pad = inner_width.saturating_sub(content.chars().count());

    vec![
        Line::from(Span::styled(format!("┌{rule}┐"), border)),
        Line::from(vec![
            Span::styled("│ ".to_string(), border),
            Span::raw(content),
            Span::styled(format!("{} │", " ".repeat(pad)), border),
        ]),
        Line::from(Span::styled(format!("└{rule}┘"), border)),
    ]
}
