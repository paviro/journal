use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::tui::app::{
    App, ENTRY_LIST_INLINE_WIDTH, ENTRY_LIST_MIN_WIDTH, Focus, JOURNAL_LIST_WIDTH, Mode,
    inline_entry_view_is_visible, single_panel_is_active,
};
use crate::tui::surface::{EntryListGeometry, PanelGeometry};

use super::chrome::footer_height;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiLayout {
    pub(crate) content: Rect,
    pub(crate) footer: Rect,
    pub(crate) journals: Option<PanelGeometry>,
    pub(crate) entries: Option<EntryListGeometry>,
    pub(crate) entry_view: Option<PanelGeometry>,
    pub(crate) stats: Option<PanelGeometry>,
    pub(crate) single_panel: bool,
}

pub(crate) fn tui_layout(area: Rect, app: &App) -> TuiLayout {
    let footer_height = footer_height(app, area.width).min(area.height);
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
        .split(area);
    let content = root[0];
    let footer = root[1];
    let inline_entry_view_visible = inline_entry_view_is_visible(content.width);
    let single_panel = single_panel_is_active(content.width);
    let show_journals = app.config.show_journals;

    let mut layout = TuiLayout {
        content,
        footer,
        journals: None,
        entries: None,
        entry_view: None,
        stats: None,
        single_panel,
    };

    if single_panel {
        match app.nav.focus {
            Focus::Journals if app.nav.mode == Mode::Browse && show_journals => {
                layout.journals = Some(PanelGeometry::new(content))
            }
            Focus::EntryView => layout.entry_view = Some(PanelGeometry::new(content)),
            Focus::Journals | Focus::Entries => {
                layout.entries = Some(EntryListGeometry::new(content))
            }
        }
        return layout;
    }

    if inline_entry_view_visible {
        if show_journals {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(JOURNAL_LIST_WIDTH),
                    Constraint::Length(ENTRY_LIST_INLINE_WIDTH),
                    Constraint::Min(ENTRY_LIST_MIN_WIDTH),
                ])
                .split(content);
            layout.journals = Some(PanelGeometry::new(body[0]));
            layout.entries = Some(EntryListGeometry::new(body[1]));
            if app.nav.mode == Mode::Browse && app.nav.focus == Focus::Journals {
                layout.stats = Some(PanelGeometry::new(body[2]));
            } else {
                layout.entry_view = Some(PanelGeometry::new(body[2]));
            }
        } else {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(ENTRY_LIST_INLINE_WIDTH),
                    Constraint::Min(ENTRY_LIST_MIN_WIDTH),
                ])
                .split(content);
            layout.entries = Some(EntryListGeometry::new(body[0]));
            layout.entry_view = Some(PanelGeometry::new(body[1]));
        }
    } else {
        if show_journals && app.nav.mode == Mode::Browse && app.nav.focus == Focus::Journals {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(JOURNAL_LIST_WIDTH),
                    Constraint::Min(ENTRY_LIST_MIN_WIDTH),
                ])
                .split(content);
            layout.journals = Some(PanelGeometry::new(body[0]));
            layout.entries = Some(EntryListGeometry::new(body[1]));
        } else {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(ENTRY_LIST_INLINE_WIDTH),
                    Constraint::Min(0),
                ])
                .split(content);
            layout.entries = Some(EntryListGeometry::new(body[0]));
            layout.entry_view = Some(PanelGeometry::new(body[1]));
        }
    }

    layout
}
