use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders},
};

use crate::tui::app::{App, Focus, Mode};

pub(crate) fn footer_text(app: &App, entry_view_visible: bool) -> String {
    if !app.status.is_empty() {
        return app.status.clone();
    }

    match app.mode {
        Mode::Search => search_footer_text(app, entry_view_visible),
        Mode::Browse => browse_footer_text(app, entry_view_visible),
    }
}

fn search_footer_text(app: &App, entry_view_visible: bool) -> String {
    let query = format!("Search {}: {}", app.search_scope_label(), app.search_query);
    match app.focus {
        Focus::EntryView if app.has_selected_entry_target() => {
            format!(
                "{query} | left results | up/down/k/j scroll | PgUp/PgDn | Home/End | enter/v view | e edit | d delete | Esc search"
            )
        }
        Focus::EntryView => format!("{query} | left results | Esc search"),
        _ => {
            let mut parts = vec![
                format!("Search {}: {}", app.search_scope_label(), app.search_query),
                "type query".to_string(),
                "backspace".to_string(),
                "up/down select".to_string(),
            ];
            if app.has_selected_entry_target() {
                if entry_view_visible {
                    parts.push("enter view".to_string());
                    parts.push("right view".to_string());
                } else {
                    parts.push("right/enter view".to_string());
                }
            }
            parts.push("Esc search".to_string());
            parts.join(" | ")
        }
    }
}

fn browse_footer_text(app: &App, entry_view_visible: bool) -> String {
    let mut parts = match app.focus {
        Focus::Journals => vec![
            "q quit".to_string(),
            "up/down select journal".to_string(),
            "right entries".to_string(),
            "n new entry".to_string(),
            "j new journal".to_string(),
            "/ search".to_string(),
            "r refresh".to_string(),
        ],
        Focus::Entries => {
            let mut parts = vec![
                "left journals".to_string(),
                "up/down select entry".to_string(),
            ];
            if app.has_selected_entry_target() {
                if entry_view_visible {
                    parts.push("right view".to_string());
                    parts.push("enter/v view".to_string());
                } else {
                    parts.push("right/enter/v view".to_string());
                }
                parts.push("e edit".to_string());
                parts.push("d delete".to_string());
            }
            parts.push("n new entry".to_string());
            parts.push("/ search".to_string());
            parts.push("q quit".to_string());
            parts
        }
        Focus::EntryView => {
            let mut parts = vec![
                "left entries".to_string(),
                "up/down/k/j scroll".to_string(),
                "PgUp/PgDn".to_string(),
                "Home/End".to_string(),
            ];
            if app.has_selected_entry_target() {
                parts.push("enter/v view".to_string());
                parts.push("e edit".to_string());
                parts.push("d delete".to_string());
            }
            parts.push("n new entry".to_string());
            parts.push("/ search".to_string());
            parts.push("q quit".to_string());
            parts
        }
    };

    if !entry_view_visible {
        parts.retain(|part| !part.contains("right view"));
    }

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
