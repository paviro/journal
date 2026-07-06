use chrono::NaiveDate;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::{app::App, render::panel_block};
use journal_storage::{Entry, entry_group_date};

pub(crate) fn draw_journal_stats(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let panel = panel_block("Journal Stats", false, None);
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

pub(crate) struct StatsLayout {
    pub(crate) identity: Rect,
    pub(crate) entries: Rect,
    pub(crate) days: Rect,
}

pub(crate) fn centered_stats_layout(area: Rect) -> StatsLayout {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalStats {
    pub(crate) name: String,
    pub(crate) entry_count: usize,
    pub(crate) active_days: usize,
    pub(crate) year_range: String,
}

pub(crate) fn journal_stats(app: &App) -> Option<JournalStats> {
    let name = app.selected_journal()?.name.clone();
    // Memoized per (journal, data version): the active-days and year-range passes
    // scan every entry in the journal (with a date parse each), which would
    // otherwise run every frame the stats preview is shown.
    let stats = app.cached_journal_stats(&name, || {
        let entries = app.selected_entries();
        JournalStats {
            name: name.clone(),
            entry_count: entries.len(),
            active_days: active_day_count(&entries),
            year_range: journal_year_range(&entries)
                .unwrap_or_else(|| "No dated entries".to_string()),
        }
    });
    Some((*stats).clone())
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
