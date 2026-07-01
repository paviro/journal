use crate::storage::Entry;
use chrono::{DateTime, Local, NaiveDate};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};
use ratatui_markdown::{markdown::MarkdownRenderer, theme::ThemeConfig};

use super::app::{App, Focus, MarkdownView, Mode};

pub(crate) fn draw(frame: &mut Frame<'_>, app: &mut App) {
    if let Some(viewer) = &mut app.viewer {
        draw_markdown_viewer(frame, viewer);
        return;
    }

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(frame.area());

    let wide = root[0].width >= 118;
    let body = if wide {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(18),
                Constraint::Length(42),
                Constraint::Min(40),
            ])
            .split(root[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(18), Constraint::Min(40)])
            .split(root[0])
    };

    draw_journals(frame, body[0], app);
    draw_items(frame, body[1], app);
    if wide {
        draw_selected_preview(frame, body[2], app);
    }

    let footer_text = if app.mode == Mode::Search {
        format!("Search: {}  Esc exits search", app.search_query)
    } else if app.status.is_empty() {
        "q quit | arrows navigate/scroll focused pane | tab/right focus preview | enter/e edit | v view | n new | j journal | d delete | / search"
            .to_string()
    } else {
        app.status.clone()
    };
    let footer = Paragraph::new(footer_text).block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, root[1]);

    if app.confirm_delete {
        let area = centered_rect(50, 20, frame.area());
        frame.render_widget(Clear, area);
        let dialog = Paragraph::new("Move selected file to trash? y/n")
            .block(
                Block::default()
                    .title("Confirm Delete")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true });
        frame.render_widget(dialog, area);
    }

    if let Some(input) = &app.new_journal_input {
        let area = centered_rect(60, 20, frame.area());
        frame.render_widget(Clear, area);
        let dialog = Paragraph::new(format!("Name: {input}\n\nEnter saves | Esc cancels"))
            .block(Block::default().title("New Journal").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        frame.render_widget(dialog, area);
    }
}

fn draw_markdown_viewer(frame: &mut Frame<'_>, viewer: &mut MarkdownView) {
    let area = frame.area();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    viewer.scroll = draw_markdown_panel(
        frame,
        root[0],
        &viewer.title,
        &viewer.content,
        viewer.scroll,
        true,
    );

    frame.render_widget(
        Paragraph::new(" Esc/q close | up/down/k/j scroll | PgUp/PgDn | Home/End"),
        root[1],
    );
}

fn draw_selected_preview(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_markdown_preview() {
        app.preview_scroll = draw_markdown_panel(
            frame,
            area,
            &title,
            &content,
            app.preview_scroll,
            app.focus == Focus::Preview,
        );
    } else {
        let empty = Paragraph::new("No entry selected")
            .block(Block::default().title("Preview").borders(Borders::ALL));
        frame.render_widget(empty, area);
    }
}

fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    title: &str,
    content: &str,
    requested_scroll: u16,
    focused: bool,
) -> u16 {
    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });
    let inner = block.inner(area);
    let width = inner.width.saturating_sub(1).max(1) as usize;
    let theme = ThemeConfig::default();
    let renderer = MarkdownRenderer::new(width);
    let blocks = renderer.parse(content);
    let lines = renderer.render(&blocks, &theme);
    let line_count = lines.len();
    let scroll = viewer_scroll(requested_scroll, line_count, inner.height);

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);

    if line_count > inner.height as usize {
        let mut state = ScrollbarState::default()
            .content_length(line_count)
            .viewport_content_length(inner.height as usize)
            .position(scrollbar_position(scroll, line_count, inner.height));
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .track_symbol(Some("|"))
            .thumb_symbol("#")
            .style(Style::default().fg(Color::DarkGray))
            .thumb_style(Style::default().fg(Color::Cyan));
        frame.render_stateful_widget(scrollbar, area, &mut state);
    }

    scroll
}

pub(crate) fn viewer_scroll(requested: u16, line_count: usize, height: u16) -> u16 {
    let max_scroll = line_count
        .saturating_sub(height as usize)
        .min(u16::MAX as usize) as u16;
    requested.min(max_scroll)
}

pub(crate) fn scrollbar_position(scroll: u16, line_count: usize, height: u16) -> usize {
    let max_scroll = line_count.saturating_sub(height as usize);
    if max_scroll == 0 {
        return 0;
    }

    (scroll as usize)
        .saturating_mul(line_count.saturating_sub(1))
        .checked_div(max_scroll)
        .unwrap_or(0)
}

fn draw_journals(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let items: Vec<ListItem> = app
        .journals
        .iter()
        .enumerate()
        .map(|(index, journal)| {
            let style =
                selected_style(index == app.selected_journal && app.focus == Focus::Journals);
            ListItem::new(Line::from(Span::raw(&journal.name))).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Journals").borders(Borders::ALL));
    frame.render_widget(list, area);
}

fn draw_items(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let title = match app.mode {
        Mode::Search => "Search",
        Mode::Browse => "Entries",
    };

    let items = match app.mode {
        Mode::Search => app
            .search_hits
            .iter()
            .enumerate()
            .map(|(index, hit)| {
                ListItem::new(vec![
                    Line::from(hit.label.clone()),
                    Line::from(Span::styled(
                        hit.preview.clone(),
                        Style::default().add_modifier(Modifier::DIM),
                    )),
                ])
                .style(selected_style(
                    index == app.selected_item && app.focus == Focus::Items,
                ))
            })
            .collect(),
        Mode::Browse => entry_items(app),
    };

    let list = List::new(items).block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(list, area);
}

fn entry_items(app: &App) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let mut current_month = None;
    let mut current_day = None;

    for (index, entry) in app.selected_entries().iter().enumerate() {
        let month = entry_month_label(entry);
        if month != current_month {
            current_month = month.clone();
            current_day = None;
            if let Some(month) = month {
                items.push(
                    ListItem::new(Line::from(Span::styled(
                        month,
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    )))
                    .style(Style::default()),
                );
            }
        }

        let day = entry_day_label(entry);
        if day != current_day {
            current_day = day.clone();
            if let Some(day) = day {
                items.push(
                    ListItem::new(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(day, Style::default().fg(Color::DarkGray)),
                    ]))
                    .style(Style::default()),
                );
            }
        }

        items.push(ListItem::new(entry_list_lines(entry)).style(selected_style(
            index == app.selected_item && app.focus == Focus::Items,
        )));
    }

    items
}

pub(crate) fn entry_month_label(entry: &Entry) -> Option<String> {
    entry_group_date(entry).map(|date| date.format("%B %Y").to_string())
}

pub(crate) fn entry_day_label(entry: &Entry) -> Option<String> {
    entry_group_date(entry).map(|date| date.format("%A %d").to_string())
}

fn entry_group_date(entry: &Entry) -> Option<NaiveDate> {
    entry
        .created_at
        .as_deref()
        .and_then(parse_entry_timestamp)
        .map(|timestamp| timestamp.date_naive())
        .or_else(|| entry_date_from_path(&entry.path))
}

fn entry_date_from_path(path: &std::path::Path) -> Option<NaiveDate> {
    let date = path.parent()?.file_name()?.to_str()?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

pub(crate) fn entry_list_lines(entry: &Entry) -> Vec<Line<'static>> {
    let timestamp = entry.created_at.as_deref().and_then(parse_entry_timestamp);
    let time = timestamp
        .as_ref()
        .map(|timestamp| timestamp.format("%H:%M").to_string())
        .unwrap_or_default();

    let dim_style = Style::default().add_modifier(Modifier::DIM);
    let left_width = 7;

    let mut title_line = if !time.is_empty() {
        vec![
            Span::styled(format!("{time:<5}"), dim_style),
            Span::raw("  "),
        ]
    } else {
        vec![Span::raw(" ".repeat(left_width))]
    };
    title_line.push(Span::styled(
        entry.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    ));

    let mut lines = vec![Line::from(title_line)];

    if !entry.preview.is_empty() {
        let mut second_line = vec![Span::raw(" ".repeat(left_width))];
        second_line.push(Span::styled(entry.preview.clone(), dim_style));

        lines.push(Line::from(second_line));
    }

    lines
}

fn parse_entry_timestamp(value: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Local))
}

fn selected_style(selected: bool) -> Style {
    if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    }
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn viewer_scroll_clamps_to_rendered_content_height() {
        assert_eq!(viewer_scroll(100, 20, 8), 12);
        assert_eq!(viewer_scroll(5, 4, 8), 0);
    }

    #[test]
    fn viewer_scroll_saturates_large_rendered_content_height() {
        assert_eq!(viewer_scroll(u16::MAX, 100_000, 8), u16::MAX);
    }

    #[test]
    fn scrollbar_position_reaches_end_at_viewer_bottom() {
        let line_count = 40;
        let height = 20;
        let scroll = viewer_scroll(u16::MAX, line_count, height);

        assert_eq!(scroll, 20);
        assert_eq!(scrollbar_position(scroll, line_count, height), 39);
    }

    #[test]
    fn scrollbar_position_stays_at_start_when_content_fits() {
        assert_eq!(scrollbar_position(0, 4, 8), 0);
    }

    #[test]
    fn entry_list_lines_use_time_gutter_and_content() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("id.md"),
            created_at: Some("2026-07-01T10:23:00+02:00".to_string()),
            updated_at: None,
            title: "Title".to_string(),
            preview: "Preview".to_string(),
            content: String::new(),
        };

        let lines = entry_list_lines(&entry);
        let rendered: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "10:23  Title");
        assert_eq!(rendered[1], "       Preview");
    }

    #[test]
    fn entry_group_labels_use_created_timestamp() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("work/2026-01-01/id.md"),
            created_at: Some("2026-07-01T10:23:00+02:00".to_string()),
            updated_at: None,
            title: "Title".to_string(),
            preview: String::new(),
            content: String::new(),
        };

        assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
        assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
    }

    #[test]
    fn entry_group_labels_fall_back_to_date_folder() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("work/2026-07-01/id.md"),
            created_at: None,
            updated_at: None,
            title: "Title".to_string(),
            preview: String::new(),
            content: String::new(),
        };

        assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
        assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
    }
}
