//! Writing-habit aggregates: how much, how often, and how consistently entries
//! are written.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, NaiveDate, Timelike};
use notema_core::Entry;

use crate::{Tally, period_key, period_label};

/// Cadence and volume of the writing itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Cadence {
    pub total_entries: usize,
    pub total_words: usize,
    pub words_per_entry_avg: f32,
    pub words_per_entry_median: usize,
    /// Distinct calendar days that carry at least one entry.
    pub active_days: usize,
    /// Earliest and latest entry date, or `None` when nothing is dated.
    pub date_span: Option<(NaiveDate, NaiveDate)>,
    /// Length of the consecutive run of active days ending on `today` or
    /// yesterday; `0` when the most recent entry is older than that.
    pub current_streak: usize,
    /// Longest run of consecutive active days ever.
    pub longest_streak: usize,
    /// Most empty days ever sat between two consecutive active days.
    pub longest_gap_days: usize,
    /// Entry counts by weekday, Monday (`0`) through Sunday (`6`).
    pub by_weekday: [usize; 7],
    /// Entry counts by hour of day (`0`..`24`), wall-clock in each entry's own
    /// offset. Only entries with a parsed timestamp contribute.
    pub by_hour: [usize; 24],
    /// Entries per calendar period, chronological (the period label is the
    /// [`Tally`] name). Year or month buckets to match the mood series.
    pub per_period: Vec<Tally>,
}

pub(crate) fn build(
    entries: &[&Entry],
    dates: &[Option<NaiveDate>],
    by_year: bool,
    today: NaiveDate,
) -> Cadence {
    let total_entries = entries.len();

    let mut word_counts: Vec<usize> = Vec::with_capacity(total_entries);
    let mut by_weekday = [0usize; 7];
    let mut by_hour = [0usize; 24];
    let mut days: BTreeSet<NaiveDate> = BTreeSet::new();
    let mut periods: BTreeMap<(i32, u32), usize> = BTreeMap::new();

    for (entry, date) in entries.iter().zip(dates) {
        word_counts.push(entry.word_count);

        if let Some(date) = date {
            days.insert(*date);
            by_weekday[date.weekday().num_days_from_monday() as usize] += 1;
            *periods.entry(period_key(*date, by_year)).or_insert(0) += 1;
        }
        if let Some(time) = entry.created_time() {
            by_hour[time.hour() as usize] += 1;
        }
    }

    let total_words: usize = word_counts.iter().sum();
    let words_per_entry_avg = if total_entries > 0 {
        total_words as f32 / total_entries as f32
    } else {
        0.0
    };

    let day_list: Vec<NaiveDate> = days.iter().copied().collect();
    let (longest_streak, longest_gap_days) = streak_and_gap(&day_list);
    let current_streak = current_streak(&day_list, today);

    Cadence {
        total_entries,
        total_words,
        words_per_entry_avg,
        words_per_entry_median: median(&mut word_counts),
        active_days: day_list.len(),
        date_span: day_list
            .first()
            .map(|first| (*first, *day_list.last().unwrap())),
        current_streak,
        longest_streak,
        longest_gap_days,
        by_weekday,
        by_hour,
        per_period: periods
            .into_iter()
            .map(|((year, month), count)| Tally {
                name: period_label(year, month, by_year),
                count,
            })
            .collect(),
    }
}

/// Median of a set of word counts. Even-length sets average the two middle
/// values (integer division). Empty input is `0`.
fn median(values: &mut [usize]) -> usize {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2
    } else {
        values[mid]
    }
}

/// The longest run of consecutive days and the largest empty gap between two
/// active days, in one pass over the sorted distinct days.
fn streak_and_gap(days: &[NaiveDate]) -> (usize, usize) {
    if days.is_empty() {
        return (0, 0);
    }
    let mut longest_streak = 1usize;
    let mut run = 1usize;
    let mut longest_gap = 0usize;
    for window in days.windows(2) {
        let delta = (window[1] - window[0]).num_days();
        if delta == 1 {
            run += 1;
            longest_streak = longest_streak.max(run);
        } else {
            run = 1;
            // `delta - 1` empty days sit between the two active days.
            longest_gap = longest_gap.max((delta - 1).max(0) as usize);
        }
    }
    (longest_streak, longest_gap)
}

/// Length of the consecutive run of active days ending at `today` or yesterday.
/// A journal last written two or more days ago has no current streak.
fn current_streak(days: &[NaiveDate], today: NaiveDate) -> usize {
    let Some(&last) = days.last() else {
        return 0;
    };
    if (today - last).num_days() > 1 {
        return 0;
    }
    let mut streak = 1usize;
    let mut prev = last;
    for &day in days.iter().rev().skip(1) {
        if (prev - day).num_days() == 1 {
            streak += 1;
            prev = day;
        } else {
            break;
        }
    }
    streak
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze;
    use crate::test_support::{date, entry_with, mood_entry, refs};
    use notema_core::Timestamp;

    /// A dated entry with a given word count and a distinct id.
    fn entry(id: &str, created: &str, words: usize) -> Entry {
        entry_with(|entry| {
            entry.id = id.to_string();
            entry.created_at = Some(Timestamp::parse(created));
            entry.word_count = words;
        })
    }

    #[test]
    fn totals_and_word_stats() {
        let entries = [
            entry("a", "2024-01-01T09:00:00Z", 10),
            entry("b", "2024-01-02T09:00:00Z", 20),
            entry("c", "2024-01-03T09:00:00Z", 60),
        ];
        let cadence = analyze(&refs(&entries), date(2024, 1, 3)).cadence;
        assert_eq!(cadence.total_entries, 3);
        assert_eq!(cadence.total_words, 90);
        assert_eq!(cadence.words_per_entry_avg, 30.0);
        assert_eq!(cadence.words_per_entry_median, 20);
    }

    #[test]
    fn current_streak_counts_run_ending_today() {
        let entries = [
            mood_entry("2024-03-10T09:00:00Z", None, &[]),
            mood_entry("2024-03-11T09:00:00Z", None, &[]),
            mood_entry("2024-03-12T09:00:00Z", None, &[]),
        ];
        let cadence = analyze(&refs(&entries), date(2024, 3, 12)).cadence;
        assert_eq!(cadence.current_streak, 3);
        assert_eq!(cadence.longest_streak, 3);
    }

    #[test]
    fn current_streak_is_zero_when_last_entry_is_stale() {
        let entries = [
            mood_entry("2024-03-10T09:00:00Z", None, &[]),
            mood_entry("2024-03-11T09:00:00Z", None, &[]),
        ];
        // Two full days since the last entry.
        let cadence = analyze(&refs(&entries), date(2024, 3, 13)).cadence;
        assert_eq!(cadence.current_streak, 0);
        assert_eq!(cadence.longest_streak, 2);
    }

    #[test]
    fn longest_gap_counts_empty_days_between_entries() {
        let entries = [
            mood_entry("2024-01-01T09:00:00Z", None, &[]),
            // Four empty days (2,3,4,5) before the next entry on the 6th.
            mood_entry("2024-01-06T09:00:00Z", None, &[]),
        ];
        let cadence = analyze(&refs(&entries), date(2024, 1, 6)).cadence;
        assert_eq!(cadence.longest_gap_days, 4);
    }

    #[test]
    fn weekday_and_hour_distributions() {
        // 2024-01-01 is a Monday; the timestamp is 09:00Z.
        let entries = [entry("a", "2024-01-01T09:00:00Z", 5)];
        let cadence = analyze(&refs(&entries), date(2024, 1, 1)).cadence;
        assert_eq!(cadence.by_weekday[0], 1);
        assert_eq!(cadence.by_weekday.iter().sum::<usize>(), 1);
        assert_eq!(cadence.by_hour[9], 1);
    }

    #[test]
    fn active_days_dedupe_same_day_entries() {
        let entries = [
            entry("a", "2024-01-01T09:00:00Z", 5),
            entry("b", "2024-01-01T18:00:00Z", 5),
        ];
        let cadence = analyze(&refs(&entries), date(2024, 1, 1)).cadence;
        assert_eq!(cadence.active_days, 1);
        assert_eq!(
            cadence.date_span,
            Some((date(2024, 1, 1), date(2024, 1, 1)))
        );
    }
}
