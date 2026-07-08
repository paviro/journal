//! The Feelings tab (feelings + mood merged): the mood balance
//! (positive/neutral/negative by mood score) across rolling windows, a row of
//! signed mood breakdowns by year / weekday / month, then a scrollable table of
//! every feeling — its frequency, mood association, and the feelings it most often
//! shows up together with (the last column). Balance and the breakdowns sit fixed
//! on top; the table fills the rest and scrolls.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use journal_analytics::{Analytics, MoodAnalytics, Sentiment};

use super::correlate::{self, InsightsListMetrics};
use super::widgets::{Section, draw_signed_columns, grid, heading, sentiment_segments, stack};
use crate::tui::render::render_centered_notice;
use crate::tui::theme::theme;

/// Balance rows, newest window last: all-time, then the trailing year / month /
/// week from `MoodAnalytics::sentiment_windows`.
const BALANCE_LABELS: [&str; 4] = ["All", "Year", "Month", "Week"];

/// Column labels for the breakdown charts; `draw_signed_columns` truncates each to
/// its column width, so a wide column shows `Mon`/`Jan` and a narrow one an initial.
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(super) fn draw(
    frame: &mut Frame<'_>,
    area: Rect,
    analytics: &Analytics,
    scroll: &mut u16,
) -> InsightsListMetrics {
    let mood = &analytics.mood;
    if mood.mean.is_none() && mood.feelings.is_empty() {
        *scroll = 0;
        render_centered_notice(frame, area, "No mood or feelings logged yet");
        return InsightsListMetrics {
            total: 0,
            viewport: 0,
        };
    }

    let sections = stack(
        area,
        &[
            Section::new(6, 0), // balance: heading + blank + 4 window rows
            Section::new(8, 0), // by year / weekday / month, side by side
            Section::new(5, 3), // feelings table (heading + blank + scrolling table)
        ],
    );

    if let Some(area) = sections[0] {
        let body = heading(frame, area, "Balance");
        draw_balance(frame, body, &mood.sentiment, &mood.sentiment_windows);
    }

    if let Some(area) = sections[1] {
        draw_breakdowns(frame, area, mood);
    }

    match sections[2] {
        Some(area) => {
            let body = heading(frame, area, "Feelings");
            // The trailing column here is which feelings co-occur with each row's feeling.
            correlate::draw(
                frame,
                body,
                &analytics.correlations.feelings,
                "—",
                "Together",
                scroll,
            )
        }
        None => {
            *scroll = 0;
            InsightsListMetrics {
                total: 0,
                viewport: 0,
            }
        }
    }
}

/// One mood-balance bar per window, `label ▓▓▓···`, so the trend across all-time
/// / year / month / week reads down the column. Windows with no logged moods show
/// a dim track.
fn draw_balance(frame: &mut Frame<'_>, area: Rect, all: &Sentiment, windows: &[Sentiment; 3]) {
    const LABEL_W: usize = 5;
    let rows = [all, &windows[0], &windows[1], &windows[2]];
    let seg_width = (area.width as usize).saturating_sub(LABEL_W + 1);
    for (index, (label, sentiment)) in BALANCE_LABELS.iter().zip(rows).enumerate() {
        if index as u16 >= area.height {
            break;
        }
        let row = Rect {
            y: area.y + index as u16,
            height: 1,
            ..area
        };
        let mut spans = vec![Span::styled(format!("{label:<LABEL_W$} "), theme().muted())];
        spans.extend(
            sentiment_segments(
                sentiment.positive,
                sentiment.neutral,
                sentiment.negative,
                seg_width,
            )
            .spans,
        );
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }
}

/// The three signed mood breakdowns on one row — by year, by weekday, by month —
/// each a zero-baseline column chart of fixed height. Splits the area into three
/// gutter-separated cells the way the Writing tab splits its weekday/hour pair.
fn draw_breakdowns(frame: &mut Frame<'_>, area: Rect, mood: &MoodAnalytics) {
    const GUTTER: u16 = 2;
    let mut cells = grid(area, 3, 1);
    for cell in cells.iter_mut().skip(1) {
        cell.x += GUTTER;
        cell.width = cell.width.saturating_sub(GUTTER);
    }

    let year_values: Vec<Option<f32>> =
        mood.by_year.iter().map(|bucket| Some(bucket.avg)).collect();
    let year_labels: Vec<&str> = mood
        .by_year
        .iter()
        .map(|bucket| bucket.label.as_str())
        .collect();

    let body = heading(frame, cells[0], "By year");
    draw_signed_columns(frame, body, &year_values, &year_labels);
    let body = heading(frame, cells[1], "By weekday");
    draw_signed_columns(frame, body, &mood.by_weekday, &WEEKDAYS);
    let body = heading(frame, cells[2], "By month");
    draw_signed_columns(frame, body, &mood.by_month, &MONTHS);
}
