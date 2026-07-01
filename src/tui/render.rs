use crate::storage::Entry;
use chrono::{DateTime, Local, NaiveDate};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use ratatui_markdown::{
    markdown::MarkdownRenderer,
    theme::{CodeColors, ThemeConfig},
};

use super::app::{
    App, ENTRY_LIST_MIN_WIDTH, Focus, JOURNAL_LIST_WIDTH, MarkdownView, Mode, preview_is_visible,
    single_panel_is_active,
};

pub(crate) fn draw(frame: &mut Frame<'_>, app: &mut App) {
    if let Some(viewer) = &mut app.viewer {
        draw_markdown_viewer(frame, viewer);
        return;
    }

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(frame.area());

    let preview_visible = preview_is_visible(root[0].width);
    let single_panel = single_panel_is_active(root[0].width);
    app.normalize_focus(preview_visible);

    if single_panel {
        draw_focused_panel(frame, root[0], app);
    } else if preview_visible {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(JOURNAL_LIST_WIDTH),
                Constraint::Length(42),
                Constraint::Min(ENTRY_LIST_MIN_WIDTH),
            ])
            .split(root[0]);
        draw_journals(frame, body[0], app);
        draw_entry_list(frame, body[1], app);
        if app.mode == Mode::Browse && app.focus == Focus::Journals {
            draw_journal_stats(frame, body[2], app);
        } else {
            draw_selected_preview(frame, body[2], app);
        }
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(JOURNAL_LIST_WIDTH),
                Constraint::Min(ENTRY_LIST_MIN_WIDTH),
            ])
            .split(root[0]);
        draw_journals(frame, body[0], app);
        draw_entry_list(frame, body[1], app);
    }

    let footer_text = footer_text(app, preview_visible);
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

fn draw_focused_panel(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &mut App) {
    match app.focus {
        Focus::Journals if app.mode == Mode::Browse => draw_journals(frame, area, app),
        Focus::Preview => draw_selected_preview(frame, area, app),
        Focus::Journals | Focus::Entries => draw_entry_list(frame, area, app),
    }
}

pub(crate) fn footer_text(app: &App, preview_visible: bool) -> String {
    if !app.status.is_empty() {
        return app.status.clone();
    }

    match app.mode {
        Mode::Search => search_footer_text(app, preview_visible),
        Mode::Browse => browse_footer_text(app, preview_visible),
    }
}

fn search_footer_text(app: &App, preview_visible: bool) -> String {
    let query = format!("Search {}: {}", app.search_scope_label(), app.search_query);
    match app.focus {
        Focus::Preview if app.has_selected_entry_target() => {
            format!(
                "{query} | left results | up/down/k/j scroll | PgUp/PgDn | Home/End | enter/v view | e edit | d delete | Esc search"
            )
        }
        Focus::Preview => format!("{query} | left results | Esc search"),
        _ => {
            let mut parts = vec![
                format!("Search {}: {}", app.search_scope_label(), app.search_query),
                "type query".to_string(),
                "backspace".to_string(),
                "up/down select".to_string(),
            ];
            if app.has_selected_entry_target() {
                if preview_visible {
                    parts.push("enter view".to_string());
                    parts.push("right preview".to_string());
                } else {
                    parts.push("right/enter view".to_string());
                }
            }
            parts.push("Esc search".to_string());
            parts.join(" | ")
        }
    }
}

fn browse_footer_text(app: &App, preview_visible: bool) -> String {
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
                if preview_visible {
                    parts.push("right preview".to_string());
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
        Focus::Preview => {
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

    if !preview_visible {
        parts.retain(|part| !part.contains("preview"));
    }

    parts.join(" | ")
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
        Paragraph::new(" Enter/Esc/q close | e edit | up/down/k/j scroll | PgUp/PgDn | Home/End"),
        root[1],
    );
}

fn draw_selected_preview(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_entry_preview() {
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
            .block(panel_block("Preview", app.focus == Focus::Preview));
        frame.render_widget(empty, area);
    }
}

fn draw_journal_stats(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let panel = panel_block("Journal Stats", false);
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let Some(stats) = journal_stats(app) else {
        frame.render_widget(Paragraph::new("No journal selected"), inner);
        return;
    };

    let layout = centered_stats_layout(inner);
    draw_journal_identity(frame, layout.identity, &stats);
    draw_stat_card(
        frame,
        layout.entries,
        "Entries",
        &stats.entry_count.to_string(),
    );
    draw_stat_card(frame, layout.days, "Days", &stats.active_days.to_string());
}

struct StatsLayout {
    identity: Rect,
    entries: Rect,
    days: Rect,
}

fn centered_stats_layout(area: Rect) -> StatsLayout {
    let content = centered_fixed_rect(area, 60, 14);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(0),
            Constraint::Length(6),
        ])
        .split(content);
    let metrics = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(1),
            Constraint::Percentage(50),
        ])
        .split(vertical[2]);

    StatsLayout {
        identity: vertical[0],
        entries: metrics[0],
        days: metrics[2],
    }
}

fn draw_journal_identity(frame: &mut Frame<'_>, area: Rect, stats: &JournalStats) {
    let identity = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            stats.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(stats.year_range.clone()),
    ])
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(identity, area);
}

fn draw_stat_card(frame: &mut Frame<'_>, area: Rect, label: &'static str, value: &str) {
    let card = Paragraph::new(vec![
        Line::from(""),
        Line::from(label),
        Line::from(Span::styled(
            value.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ])
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(card, area);
}

fn centered_fixed_rect(area: Rect, desired_width: u16, desired_height: u16) -> Rect {
    let width = desired_width.min(area.width);
    let height = desired_height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct JournalStats {
    name: String,
    entry_count: usize,
    active_days: usize,
    year_range: String,
}

fn journal_stats(app: &App) -> Option<JournalStats> {
    let journal = app.selected_journal()?;
    let entries = app.selected_entries();
    let entry_count = entries.len();
    let active_days = active_day_count(&entries);
    let year_range = journal_year_range(&entries).unwrap_or_else(|| "No dated entries".to_string());

    Some(JournalStats {
        name: journal.name.clone(),
        entry_count,
        active_days,
        year_range,
    })
}

fn journal_year_range(entries: &[&Entry]) -> Option<String> {
    let mut dates = entries.iter().filter_map(|entry| entry_group_date(entry));
    let first = dates.next()?;
    let (oldest, newest) = dates.fold((first, first), |(oldest, newest), date| {
        (oldest.min(date), newest.max(date))
    });

    let oldest_year = oldest.format("%Y").to_string();
    let newest_year = newest.format("%Y").to_string();
    if oldest_year == newest_year {
        Some(oldest_year)
    } else {
        Some(format!("{oldest_year}-{newest_year}"))
    }
}

fn active_day_count(entries: &[&Entry]) -> usize {
    let mut dates: Vec<NaiveDate> = entries
        .iter()
        .filter_map(|entry| entry_group_date(entry))
        .collect();
    dates.sort_unstable();
    dates.dedup();
    dates.len()
}

fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    title: &str,
    content: &str,
    requested_scroll: u16,
    focused: bool,
) -> u16 {
    let block = panel_block(title, focused);
    let inner = block.inner(area);
    let width = inner.width.saturating_sub(1).max(1) as usize;
    let theme = markdown_theme();
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
            .style(Style::default().add_modifier(Modifier::DIM))
            .thumb_style(Style::default().add_modifier(Modifier::BOLD));
        frame.render_stateful_widget(scrollbar, area, &mut state);
    }

    scroll
}

fn markdown_theme() -> ThemeConfig {
    let foreground = Color::Reset;
    ThemeConfig::builder()
        .with_text_color(foreground)
        .with_muted_text_color(foreground)
        .with_primary_color(foreground)
        .with_popup_selected_background(foreground)
        .with_border_color(foreground)
        .with_focused_border_color(foreground)
        .with_secondary_color(foreground)
        .with_info_color(foreground)
        .with_json_key_color(foreground)
        .with_json_string_color(foreground)
        .with_json_number_color(foreground)
        .with_json_bool_color(foreground)
        .with_json_null_color(foreground)
        .with_accent_yellow(foreground)
        .with_code_colors(reset_code_colors())
        .build()
}

fn reset_code_colors() -> CodeColors {
    CodeColors {
        comment: Color::Reset,
        keyword: Color::Reset,
        string: Color::Reset,
        string_escape: Color::Reset,
        number: Color::Reset,
        constant: Color::Reset,
        function: Color::Reset,
        r#type: Color::Reset,
        variable: Color::Reset,
        property: Color::Reset,
        operator: Color::Reset,
        punctuation: Color::Reset,
        attribute: Color::Reset,
        tag: Color::Reset,
        label: Color::Reset,
        error: Color::Reset,
    }
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
    let focused = app.focus == Focus::Journals;
    let items: Vec<ListItem> = app
        .journals
        .iter()
        .enumerate()
        .map(|(index, journal)| {
            let style = selected_style(index == app.selected_journal && focused);
            ListItem::new(Line::from(Span::raw(&journal.name))).style(style)
        })
        .collect();

    let list = List::new(items).block(panel_block("Journals", focused));
    frame.render_widget(list, area);
}

fn draw_entry_list(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let focused = app.focus == Focus::Entries;
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
                    Line::from(app.search_hit_label(hit)),
                    Line::from(Span::styled(
                        hit.preview.clone(),
                        Style::default().add_modifier(Modifier::DIM),
                    )),
                ])
                .style(selected_style(index == app.selected_entry_index && focused))
            })
            .collect(),
        Mode::Browse => entry_list_items(app),
    };

    let list = List::new(items).block(panel_block(title, focused));
    frame.render_widget(list, area);
}

fn entry_list_items(app: &App) -> Vec<ListItem<'static>> {
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
                        Style::default().add_modifier(Modifier::BOLD | Modifier::DIM),
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
                        Span::styled(day, Style::default().add_modifier(Modifier::DIM)),
                    ]))
                    .style(Style::default()),
                );
            }
        }

        items.push(ListItem::new(entry_list_lines(entry)).style(selected_style(
            index == app.selected_entry_index && app.focus == Focus::Entries,
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

fn panel_block(title: &str, focused: bool) -> Block<'static> {
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

fn panel_title(title: &str, focused: bool) -> String {
    if focused {
        format!(" >> {title} ")
    } else {
        format!(" {title} ")
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
    use crate::config::Config;
    use ratatui::{Terminal, backend::TestBackend};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn app_with_entry() -> App {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let entry_dir = root.join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# A\nBody\n",
        )
        .unwrap();

        let config = Config::new(root, "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app
    }

    fn render_text(mut app: App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

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
    fn markdown_theme_uses_terminal_default_foregrounds() {
        let theme = markdown_theme();

        assert_eq!(theme.text_color, Color::Reset);
        assert_eq!(theme.muted_text_color, Color::Reset);
        assert_eq!(theme.primary_color, Color::Reset);
        assert_eq!(theme.secondary_color, Color::Reset);
        assert_eq!(theme.accent_yellow, Color::Reset);
        assert_eq!(theme.code_colors.variable, Color::Reset);
    }

    #[test]
    fn focused_panel_titles_have_ascii_focus_marker() {
        assert_eq!(panel_title("Entries", true), " >> Entries ");
        assert_eq!(panel_title("Entries", false), " Entries ");
    }

    #[test]
    fn compact_render_shows_only_the_active_step() {
        let mut journals_app = app_with_entry();
        journals_app.focus = Focus::Journals;
        let journals = render_text(journals_app, 57, 16);
        assert!(journals.contains(">> Journals"));
        assert!(!journals.contains(" Entries "));
        assert!(!journals.contains("2026-07-01 10:00"));

        let mut entries_app = app_with_entry();
        entries_app.focus = Focus::Entries;
        let entries = render_text(entries_app, 57, 16);
        assert!(entries.contains(">> Entries"));
        assert!(!entries.contains(" Journals "));
        assert!(!entries.contains("2026-07-01 10:00"));

        let mut preview_focus_app = app_with_entry();
        preview_focus_app.focus = Focus::Preview;
        let preview_focus = render_text(preview_focus_app, 57, 16);
        assert!(preview_focus.contains(">> Entries"));
        assert!(!preview_focus.contains(" Journals "));
        assert!(!preview_focus.contains("2026-07-01 10:00"));
    }

    #[test]
    fn journal_stats_summarizes_selected_journal() {
        let app = app_with_entry();

        let stats = journal_stats(&app).unwrap();

        assert_eq!(stats.name, "work");
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.active_days, 1);
        assert_eq!(stats.year_range, "2026");
    }

    #[test]
    fn journal_stats_handles_empty_journals() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");

        let stats = journal_stats(&app).unwrap();

        assert_eq!(stats.name, "work");
        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.active_days, 0);
        assert_eq!(stats.year_range, "No dated entries");
    }

    #[test]
    fn centered_stats_layout_places_identity_above_metric_cards() {
        let layout = centered_stats_layout(Rect {
            x: 10,
            y: 3,
            width: 80,
            height: 24,
        });

        assert_eq!(layout.identity.y, 8);
        assert_eq!(layout.identity.height, 6);
        assert_eq!(layout.entries.y, 14);
        assert_eq!(layout.days.y, 14);
        assert!(layout.entries.x < layout.days.x);
        assert_eq!(layout.entries.height, 6);
        assert_eq!(layout.days.height, 6);
    }

    #[test]
    fn journal_footer_omits_entry_actions() {
        let mut app = app_with_entry();
        app.focus = Focus::Journals;

        let text = footer_text(&app, true);

        assert!(!text.contains("enter/v view"));
        assert!(!text.contains("e edit"));
        assert!(!text.contains("d delete"));
    }

    #[test]
    fn entries_footer_includes_entry_actions_when_an_entry_is_selected() {
        let mut app = app_with_entry();
        app.focus = Focus::Entries;

        let text = footer_text(&app, true);

        assert!(text.contains("enter/v view"));
        assert!(text.contains("e edit"));
        assert!(text.contains("d delete"));
    }

    #[test]
    fn entries_footer_omits_entry_actions_without_a_selection() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = App::new(config).unwrap();
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        let text = footer_text(&app, true);

        assert!(!text.contains("enter/v view"));
        assert!(!text.contains("e edit"));
        assert!(!text.contains("d delete"));
    }

    #[test]
    fn search_results_footer_keeps_text_input_keys_available() {
        let mut app = app_with_entry();
        app.mode = Mode::Search;
        app.focus = Focus::Entries;
        app.search_query = "body".to_string();
        app.search_hits = vec![crate::storage::SearchHit {
            path: app.entries[0].path.clone(),
            journal: "work".to_string(),
            title: "A".to_string(),
            preview: "Body".to_string(),
        }];

        let text = footer_text(&app, true);

        assert!(text.contains("type query"));
        assert!(text.contains("Search all: body"));
        assert!(text.contains("enter view"));
        assert!(!text.contains("enter/v view"));
        assert!(!text.contains("e edit"));
        assert!(!text.contains("d delete"));
    }

    #[test]
    fn scoped_search_hit_labels_omit_journal_prefix() {
        let mut app = app_with_entry();
        app.search_scope = crate::tui::app::SearchScope::CurrentJournal("work".to_string());
        let hit = crate::storage::SearchHit {
            path: app.entries[0].path.clone(),
            journal: "work".to_string(),
            title: "A".to_string(),
            preview: "Body".to_string(),
        };

        assert_eq!(app.search_hit_label(&hit), "A");
    }

    #[test]
    fn global_search_hit_labels_include_journal_prefix() {
        let app = app_with_entry();
        let hit = crate::storage::SearchHit {
            path: app.entries[0].path.clone(),
            journal: "work".to_string(),
            title: "A".to_string(),
            preview: "Body".to_string(),
        };

        assert_eq!(app.search_hit_label(&hit), "work/A");
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
