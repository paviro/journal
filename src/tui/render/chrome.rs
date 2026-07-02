use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders},
};

use crate::tui::app::{App, Focus, Mode};

pub(crate) fn footer_text(app: &App, _entry_view_visible: bool) -> String {
    if !app.status().is_empty() {
        return app.status().to_string();
    }

    match app.mode {
        Mode::Search => search_footer_text(app),
        Mode::Browse => browse_footer_text(app),
    }
}

fn search_footer_text(app: &App) -> String {
    let query = format!("Search {}: {}", app.search_scope_label(), app.search.query);
    match app.focus {
        Focus::EntryView if app.has_selected_entry_target() => {
            format!("{query} | view (enter) | edit (e) | delete (d) | exit search (esc) | quit (q)")
        }
        Focus::EntryView => format!("{query} | exit search (esc) | quit (q)"),
        _ => {
            let mut parts = vec![format!(
                "Search {}: {}",
                app.search_scope_label(),
                app.search.query
            )];
            if app.has_selected_entry_target() {
                parts.push("view (enter)".to_string());
            }
            parts.push("exit search (esc)".to_string());
            parts.join(" | ")
        }
    }
}

fn browse_footer_text(app: &App) -> String {
    let parts = match app.focus {
        Focus::Journals => vec![
            "new journal (n)".to_string(),
            "refresh (r)".to_string(),
            "search (/)".to_string(),
            "quit (q)".to_string(),
        ],
        Focus::Entries => {
            let mut parts = vec![];
            parts.push("new entry (n)".to_string());
            if app.has_selected_entry_target() {
                parts.push("edit (e)".to_string());
                parts.push("view (enter)".to_string());
                parts.push("delete (d)".to_string());
                parts.push("edit tags (t)".to_string());
            }
            parts.push("search (/)".to_string());
            parts.push("quit (q)".to_string());
            parts
        }
        Focus::EntryView => {
            let mut parts = vec![];
            parts.push("new entry (n)".to_string());
            if app.has_selected_entry_target() {
                parts.push("edit (e)".to_string());
                parts.push("view (enter)".to_string());
                parts.push("delete (d)".to_string());
                parts.push("edit tags (t)".to_string());
            }
            parts.push("search (/)".to_string());
            parts.push("quit (q)".to_string());
            parts
        }
    };

    parts.join(" | ")
}

pub(crate) fn selected_style(selected: bool) -> Style {
    if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    }
}

pub(crate) fn panel_block(title: &str, focused: bool) -> Block<'static> {
    let mut block = Block::default()
        .title(panel_title(title, focused))
        .borders(Borders::ALL);

    if focused {
        block = block
            .border_type(BorderType::Thick)
            .border_style(Style::default().add_modifier(Modifier::BOLD));
    }

    block
}

pub(crate) fn panel_title(title: &str, focused: bool) -> String {
    if focused {
        format!(" >> {title} ")
    } else {
        format!(" {title} ")
    }
}

pub(crate) fn panel_content_inner(area: Rect) -> Rect {
    let pad = 1;
    Rect {
        x: area.x.saturating_add(pad),
        width: area.width.saturating_sub(pad * 2).max(1),
        ..area
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
