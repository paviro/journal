//! The Writing tab: writing habit — streaks, volume over time, and when entries
//! get written.

use ratatui::{Frame, layout::Rect};

use notema_analytics::Analytics;

use super::widgets::{
    Bar, Section, Stat, TWO_COL_MIN_WIDTH, caption, draw_bars, draw_histogram, draw_stats, grid,
    heading, stack,
};
use crate::tui::render::render_centered_notice;
use crate::tui::theme::theme;

pub(super) fn draw(frame: &mut Frame<'_>, area: Rect, analytics: &Analytics) {
    let cadence = &analytics.cadence;
    if cadence.total_entries == 0 {
        render_centered_notice(frame, area, "No entries yet");
        return;
    }

    let sections = stack(
        area,
        &[
            Section::new(4, 0), // streak cards (no heading)
            Section::new(4, 3), // entries over time (heading + blank + bars)
            Section::new(6, 2), // weekday + hour (side by side when wide)
            Section::new(4, 0), // word stats (no heading)
        ],
    );

    if let Some(cards) = sections[0] {
        draw_stats(
            frame,
            cards,
            &[
                Stat::new("Streak", format!("{}d", cadence.current_streak)),
                Stat::new("Longest", format!("{}d", cadence.longest_streak)),
                Stat::new("Longest gap", format!("{}d", cadence.longest_gap_days)),
            ],
        );
    }

    if let Some(area) = sections[1] {
        let body = heading(frame, area, "Entries over time");
        let max = cadence
            .per_period
            .iter()
            .map(|period| period.count)
            .max()
            .unwrap_or(1)
            .max(1);
        let bars: Vec<Bar> = cadence
            .per_period
            .iter()
            .map(|period| Bar {
                label: period.name.clone(),
                fill: period.count as f32 / max as f32,
                value: period.count.to_string(),
                style: theme().chart_bar().style,
            })
            .collect();
        draw_bars(frame, body, &bars);
    }

    if let Some(area) = sections[2] {
        // Side by side on a wide panel, stacked when the column is narrow.
        let (weekday_area, hour_area) = if area.width >= TWO_COL_MIN_WIDTH {
            let cells = grid(area, 2, 1);
            // Inset the right column so its divider rule doesn't touch the left
            // section's, leaving a small gutter between the two headings.
            const GUTTER: u16 = 2;
            let mut left = cells[0];
            left.width = left.width.saturating_sub(GUTTER);
            let mut right = cells[1];
            right.x += GUTTER;
            right.width = right.width.saturating_sub(GUTTER);
            (left, right)
        } else {
            let cells = grid(area, 1, 2);
            (cells[0], cells[1])
        };
        let body = heading(frame, weekday_area, "By weekday");
        draw_axis_histogram(frame, body, &cadence.by_weekday, "Mon → Sun");
        let body = heading(frame, hour_area, "By hour");
        let bins = hour_bins(body.width);
        draw_axis_histogram(frame, body, &bin(&cadence.by_hour, bins), "0h → 24h");
    }

    if let Some(cards) = sections[3] {
        draw_stats(
            frame,
            cards,
            &[
                Stat::new("Total words", cadence.total_words.to_string()),
                Stat::new("Avg / entry", format!("{:.0}", cadence.words_per_entry_avg)),
                Stat::new("Median", cadence.words_per_entry_median.to_string()),
            ],
        );
    }
}

/// A histogram with a dim axis caption pinned to its bottom row.
fn draw_axis_histogram(frame: &mut Frame<'_>, area: Rect, values: &[usize], axis: &str) {
    if area.height == 0 {
        return;
    }
    let bars = Rect {
        height: area.height - 1,
        ..area
    };
    let axis_row = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };
    draw_histogram(frame, bars, values);
    caption(frame, axis_row, axis);
}

/// Group `values` into `groups` contiguous bins, summing each.
fn bin(values: &[usize], groups: usize) -> Vec<usize> {
    if groups == 0 || groups >= values.len() {
        return values.to_vec();
    }
    let per = values.len().div_ceil(groups);
    values.chunks(per).map(|chunk| chunk.iter().sum()).collect()
}

/// How many hour bins fit `width` (one cell + a gap per bin): all 24, else 12
/// two-hour bins, else 8 three-hour bins.
fn hour_bins(width: u16) -> usize {
    if width >= 47 {
        24
    } else if width >= 23 {
        12
    } else {
        8
    }
}
