use crate::storage::{Entry, entry_group_date, parse_entry_timestamp};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};

use super::{
    app::{App, Focus, Mode},
    render::selected_style,
    scroll::clamp_scroll,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryRowMeta {
    pub(crate) entry_index: Option<usize>,
    pub(crate) height: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryListRow {
    pub(crate) entry_index: Option<usize>,
    lines: Vec<Line<'static>>,
    selected: bool,
}

impl EntryListRow {
    fn height(&self) -> u16 {
        self.lines.len().min(u16::MAX as usize) as u16
    }
}

pub(crate) fn entry_list_rows(app: &App) -> Vec<EntryListRow> {
    match app.mode {
        Mode::Search => app
            .search
            .hits
            .iter()
            .enumerate()
            .map(|(index, hit)| EntryListRow {
                entry_index: Some(index),
                lines: vec![
                    Line::from(app.search_hit_label(hit)),
                    Line::from(Span::styled(
                        hit.preview.clone(),
                        Style::default().add_modifier(Modifier::DIM),
                    )),
                ],
                selected: entry_selection_is_visible(app) && index == app.selected_entry_index,
            })
            .collect(),
        Mode::Browse => browse_entry_rows(app),
    }
}

fn browse_entry_rows(app: &App) -> Vec<EntryListRow> {
    let mut rows = Vec::new();
    let mut current_month = None;
    let mut current_day = None;

    for (index, entry) in app.selected_entries().iter().enumerate() {
        let month = entry_month_label(entry);
        if month != current_month {
            current_month = month.clone();
            current_day = None;
            if let Some(month) = month {
                rows.push(EntryListRow {
                    entry_index: None,
                    lines: vec![
                        Line::from(Span::raw("─".repeat(200))),
                        Line::from(Span::styled(
                            month,
                            Style::default().add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::raw("─".repeat(200))),
                    ],
                    selected: false,
                });
            }
        }

        let day = entry_day_label(entry);
        if day != current_day {
            current_day = day.clone();
            if let Some(day) = day {
                rows.push(EntryListRow {
                    entry_index: None,
                    lines: vec![Line::from(vec![
                        Span::raw("  "),
                        Span::styled(day, Style::default()),
                    ])],
                    selected: false,
                });
            }
        }

        rows.push(EntryListRow {
            entry_index: Some(index),
            lines: entry_list_lines(entry),
            selected: entry_selection_is_visible(app) && index == app.selected_entry_index,
        });
    }

    rows
}

fn entry_selection_is_visible(app: &App) -> bool {
    app.focus != Focus::Journals
}

pub(crate) fn entry_row_metadata(app: &App) -> Vec<EntryRowMeta> {
    entry_list_rows(app)
        .into_iter()
        .map(|row| EntryRowMeta {
            entry_index: row.entry_index,
            height: row.height(),
        })
        .collect()
}

pub(crate) fn visible_entry_items(
    rows: &[EntryListRow],
    scroll: u16,
    viewport_height: u16,
) -> Vec<ListItem<'static>> {
    let mut remaining_skip = scroll;
    let mut remaining_height = viewport_height;
    let mut items = Vec::new();

    for row in rows {
        if remaining_height == 0 {
            break;
        }

        let height = row.height();
        if remaining_skip >= height {
            remaining_skip -= height;
            continue;
        }

        let start = remaining_skip as usize;
        remaining_skip = 0;
        let visible_height = height.saturating_sub(start as u16).min(remaining_height);
        let end = start + visible_height as usize;
        let lines = row.lines[start..end].to_vec();
        remaining_height = remaining_height.saturating_sub(visible_height);
        items.push(ListItem::new(lines).style(selected_style(row.selected)));
    }

    items
}

pub(crate) fn entry_month_label(entry: &Entry) -> Option<String> {
    entry_group_date(entry).map(|date| date.format("%B %Y").to_string())
}

pub(crate) fn entry_day_label(entry: &Entry) -> Option<String> {
    entry_group_date(entry).map(|date| date.format("%A %d").to_string())
}

pub(crate) fn entry_list_lines(entry: &Entry) -> Vec<Line<'static>> {
    let timestamp = entry.created_at.as_deref().and_then(parse_entry_timestamp);
    let time = timestamp
        .as_ref()
        .map(|timestamp| timestamp.format("%H:%M").to_string())
        .unwrap_or_default();

    let muted_style = Style::default();
    let left_width = 7;

    let mut title_line = if !time.is_empty() {
        vec![
            Span::styled(format!("{time:<5}"), muted_style),
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
        second_line.push(Span::styled(entry.preview.clone(), muted_style));

        lines.push(Line::from(second_line));
    }

    lines
}

pub(crate) fn ensure_entry_visible(
    scroll: &mut u16,
    rows: &[EntryRowMeta],
    selected_entry_index: usize,
    viewport_height: u16,
) {
    let Some((row_start, row_height)) = selected_entry_row_span(rows, selected_entry_index) else {
        *scroll = clamp_scroll(*scroll, total_entry_row_height(rows), viewport_height);
        return;
    };

    if viewport_height == 0 {
        *scroll = clamp_scroll(*scroll, total_entry_row_height(rows), viewport_height);
        return;
    }

    if row_start < *scroll as usize {
        *scroll = row_start.min(u16::MAX as usize) as u16;
    } else {
        let row_end = row_start.saturating_add(row_height as usize);
        let viewport_end = (*scroll as usize).saturating_add(viewport_height as usize);
        if row_end > viewport_end {
            *scroll = row_end
                .saturating_sub(viewport_height as usize)
                .min(u16::MAX as usize) as u16;
        }
    }
    *scroll = clamp_scroll(*scroll, total_entry_row_height(rows), viewport_height);
}

pub(crate) fn selected_entry_row_span(
    rows: &[EntryRowMeta],
    selected_entry_index: usize,
) -> Option<(usize, u16)> {
    let mut y = 0usize;
    for row in rows {
        if row.entry_index == Some(selected_entry_index) {
            return Some((y, row.height));
        }
        y = y.saturating_add(row.height as usize);
    }
    None
}

pub(crate) fn total_entry_row_height(rows: &[EntryRowMeta]) -> usize {
    rows.iter().map(|row| row.height as usize).sum()
}
