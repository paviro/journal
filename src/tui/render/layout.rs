use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::tui::app::{
    App, ENTRY_LIST_INLINE_WIDTH, ENTRY_LIST_MIN_WIDTH, Focus, JOURNAL_LIST_WIDTH, Mode,
    inline_reader_is_visible, single_panel_is_active,
};
use crate::tui::surface::{EntryListGeometry, PanelGeometry};

use super::footer::footer_height;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiLayout {
    pub(crate) content: Rect,
    pub(crate) footer: Rect,
    pub(crate) journals: Option<PanelGeometry>,
    pub(crate) entries: Option<EntryListGeometry>,
    pub(crate) reader: Option<PanelGeometry>,
    pub(crate) insights: Option<PanelGeometry>,
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
    let inline_reader_visible = inline_reader_is_visible(content.width);
    let single_panel = single_panel_is_active(content.width);
    let show_journals = app.state.ui.show_journals;

    let mut layout = TuiLayout {
        content,
        footer,
        journals: None,
        entries: None,
        reader: None,
        insights: None,
        single_panel,
    };

    // A full-screen viewer owns the whole content area at any width, so mouse
    // hit-testing lines up with what `draw` paints.
    if app.reader_is_fullscreen(content.width) {
        layout.reader = Some(PanelGeometry::new(content));
        return layout;
    }

    // Likewise for an expanded insights panel: hand it the whole content area and
    // let its responsive renderer pick a larger, multi-column layout from the
    // bigger `Rect` — no fullscreen flag reaches the render code.
    if app.insights_is_fullscreen(content.width) {
        layout.insights = Some(PanelGeometry::new(content));
        return layout;
    }

    if single_panel {
        match app.nav.focus {
            Focus::Journals if app.nav.mode == Mode::Browse && show_journals => {
                layout.journals = Some(PanelGeometry::new(content))
            }
            Focus::Reader => layout.reader = Some(PanelGeometry::new(content)),
            // Reached by pressing Right from the entries column (or stranded here by a
            // resize) — show the panel full-width, the only pane at this width.
            Focus::Insights => layout.insights = Some(PanelGeometry::new(content)),
            Focus::Journals | Focus::Entries => {
                layout.entries = Some(EntryListGeometry::new(content))
            }
        }
        return layout;
    }

    if inline_reader_visible {
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
            // The right column is the insights panel whenever no entry is
            // shown (Journals/Entries/Insights focus with nothing selected), and
            // the reader once an entry is selected.
            if app.show_journal_insights() {
                layout.insights = Some(PanelGeometry::new(body[2]));
            } else {
                layout.reader = Some(PanelGeometry::new(body[2]));
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
            layout.reader = Some(PanelGeometry::new(body[1]));
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
            layout.reader = Some(PanelGeometry::new(body[1]));
        }
    }

    layout
}
