use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::tui::app::{
    App, ENTRY_LIST_INLINE_WIDTH, ENTRY_LIST_MIN_WIDTH, Focus, JOURNAL_LIST_WIDTH, Mode,
    inline_entry_view_is_visible, single_panel_is_active,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiLayout {
    pub(crate) content: Rect,
    pub(crate) footer: Rect,
    pub(crate) journals: Option<Rect>,
    pub(crate) entries: Option<Rect>,
    pub(crate) entry_view: Option<Rect>,
    pub(crate) stats: Option<Rect>,
    pub(crate) entry_view_visible: bool,
    pub(crate) single_panel: bool,
}

pub(crate) fn tui_layout(area: Rect, app: &App) -> TuiLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(area);
    let content = root[0];
    let footer = root[1];
    let inline_entry_view_visible = inline_entry_view_is_visible(content.width);
    let single_panel = single_panel_is_active(content.width);

    let mut layout = TuiLayout {
        content,
        footer,
        journals: None,
        entries: None,
        entry_view: None,
        stats: None,
        entry_view_visible: false,
        single_panel,
    };

    if single_panel {
        match app.focus {
            Focus::Journals if app.mode == Mode::Browse => layout.journals = Some(content),
            Focus::EntryView => layout.entry_view = Some(content),
            Focus::Journals | Focus::Entries => layout.entries = Some(content),
        }
        return layout;
    }

    if inline_entry_view_visible {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(JOURNAL_LIST_WIDTH),
                Constraint::Length(ENTRY_LIST_INLINE_WIDTH),
                Constraint::Min(ENTRY_LIST_MIN_WIDTH),
            ])
            .split(content);
        layout.journals = Some(body[0]);
        layout.entries = Some(body[1]);
        if app.mode == Mode::Browse && app.focus == Focus::Journals {
            layout.stats = Some(body[2]);
        } else {
            layout.entry_view = Some(body[2]);
            layout.entry_view_visible = true;
        }
    } else {
        if app.mode == Mode::Browse && app.focus == Focus::Journals {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(JOURNAL_LIST_WIDTH),
                    Constraint::Min(ENTRY_LIST_MIN_WIDTH),
                ])
                .split(content);
            layout.journals = Some(body[0]);
            layout.entries = Some(body[1]);
        } else {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(ENTRY_LIST_INLINE_WIDTH),
                    Constraint::Min(0),
                ])
                .split(content);
            layout.entries = Some(body[0]);
            layout.entry_view = Some(body[1]);
            layout.entry_view_visible = true;
        }
    }

    layout
}
