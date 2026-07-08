//! Mood and emotion aggregates: the mood time series plus its central tendency,
//! seasonal breakdowns, and the feeling frequency / co-occurrence / sentiment
//! views.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};
use journal_storage::Entry;

use crate::{MoodBucket, Tally, period_key, period_label, sort_by_count_desc};

/// Everything the mood/feelings tabs need.
#[derive(Debug, Clone, PartialEq)]
pub struct MoodAnalytics {
    /// Average mood per calendar period (year or month buckets).
    pub series: Vec<MoodBucket>,
    /// Mean mood, or `None` when no entry logged a mood. Doubles as the "any mood
    /// data" guard for the mood tab.
    pub mean: Option<f32>,
    /// Average mood by weekday, Monday (`0`) through Sunday (`6`); `None` where
    /// no mood was logged.
    pub by_weekday: [Option<f32>; 7],
    /// Average mood by calendar month, January (`0`) through December (`11`).
    pub by_month: [Option<f32>; 12],
    /// Feeling frequency, most common first.
    pub feelings: Vec<Tally>,
    /// All-time mood balance, and the same over the trailing year / month / week
    /// (indices `0`/`1`/`2` of `sentiment_windows`) so Balance can show the trend.
    pub sentiment: Sentiment,
    pub sentiment_windows: [Sentiment; 3],
    /// The highest- and lowest-average periods from `series`.
    pub best_period: Option<MoodBucket>,
    pub worst_period: Option<MoodBucket>,
}

/// Entries split by mood sign, using a neutral band around the midpoint: a mood
/// of `>= 2` is positive, `-1..=1` neutral, `<= -2` negative. Each entry that
/// logged a mood counts once; entries with no mood contribute nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sentiment {
    pub positive: usize,
    pub neutral: usize,
    pub negative: usize,
}

impl Sentiment {
    /// Positive-to-negative ratio, or `None` when no negative feelings were
    /// logged (the ratio would be undefined / infinite).
    pub fn positive_negative_ratio(&self) -> Option<f32> {
        (self.negative > 0).then(|| self.positive as f32 / self.negative as f32)
    }
}

/// Trailing-window spans (in days) for the Balance sentiment trend: year, month,
/// week. Rolling rather than calendar so each window is always a full span.
const SENTIMENT_WINDOW_DAYS: [i64; 3] = [365, 30, 7];

pub(crate) fn build(
    entries: &[&Entry],
    dates: &[Option<NaiveDate>],
    by_year: bool,
    today: NaiveDate,
) -> MoodAnalytics {
    let mut moods: Vec<i8> = Vec::new();
    let mut series_acc: BTreeMap<(i32, u32), (i64, usize)> = BTreeMap::new();
    let mut weekday_acc: [(i64, usize); 7] = Default::default();
    let mut month_acc: [(i64, usize); 12] = Default::default();

    let mut feelings: HashMap<&str, usize> = HashMap::new();
    let mut sentiment = Sentiment::default();
    let mut sentiment_windows = [Sentiment::default(); 3];

    for (entry, date) in entries.iter().zip(dates) {
        if let Some(mood) = entry.metadata.mood {
            moods.push(mood);
            add_mood_valence(&mut sentiment, mood);
            if let Some(date) = date {
                let slot = series_acc
                    .entry(period_key(*date, by_year))
                    .or_insert((0, 0));
                slot.0 += i64::from(mood);
                slot.1 += 1;
                let weekday = date.weekday().num_days_from_monday() as usize;
                weekday_acc[weekday].0 += i64::from(mood);
                weekday_acc[weekday].1 += 1;
                let month = (date.month() - 1) as usize;
                month_acc[month].0 += i64::from(mood);
                month_acc[month].1 += 1;
                // The trailing-window Balance tallies: an entry counts in a window
                // when its date is within that many days of `today`.
                let age = (today - *date).num_days();
                for (window, span) in sentiment_windows.iter_mut().zip(SENTIMENT_WINDOW_DAYS) {
                    if (0..span).contains(&age) {
                        add_mood_valence(window, mood);
                    }
                }
            }
        }

        tally_feelings(entry, &mut feelings);
    }

    let series: Vec<MoodBucket> = series_acc
        .into_iter()
        .map(|((year, month), (sum, count))| MoodBucket {
            label: period_label(year, month, by_year),
            avg: sum as f32 / count as f32,
            count,
        })
        .collect();
    let (best_period, worst_period) = best_and_worst(&series);

    MoodAnalytics {
        best_period,
        worst_period,
        mean: mean(&moods),
        by_weekday: averages(&weekday_acc),
        by_month: averages(&month_acc),
        feelings: rank_feelings(feelings),
        sentiment,
        sentiment_windows,
        series,
    }
}

/// Fold one entry's feelings into the frequency accumulator.
fn tally_feelings<'a>(entry: &'a Entry, feelings: &mut HashMap<&'a str, usize>) {
    for feeling in &entry.metadata.feelings {
        *feelings.entry(feeling.as_str()).or_default() += 1;
    }
}

/// Fold one entry's mood into a Balance tally, using a neutral band around the
/// midpoint: `>= 2` positive, `-1..=1` neutral, `<= -2` negative.
fn add_mood_valence(sentiment: &mut Sentiment, mood: i8) {
    if mood >= 2 {
        sentiment.positive += 1;
    } else if mood <= -2 {
        sentiment.negative += 1;
    } else {
        sentiment.neutral += 1;
    }
}

fn rank_feelings(feelings: HashMap<&str, usize>) -> Vec<Tally> {
    let mut tallies: Vec<Tally> = feelings
        .into_iter()
        .map(|(name, count)| Tally {
            name: name.to_string(),
            count,
        })
        .collect();
    sort_by_count_desc(&mut tallies, |tally| (tally.count, tally.name.as_str()));
    tallies
}

fn best_and_worst(series: &[MoodBucket]) -> (Option<MoodBucket>, Option<MoodBucket>) {
    let best = series
        .iter()
        .max_by(|a, b| a.avg.total_cmp(&b.avg))
        .cloned();
    let worst = series
        .iter()
        .min_by(|a, b| a.avg.total_cmp(&b.avg))
        .cloned();
    (best, worst)
}

fn mean(moods: &[i8]) -> Option<f32> {
    if moods.is_empty() {
        return None;
    }
    let sum: i64 = moods.iter().map(|m| i64::from(*m)).sum();
    Some(sum as f32 / moods.len() as f32)
}

fn averages<const N: usize>(acc: &[(i64, usize); N]) -> [Option<f32>; N] {
    let mut out = [None; N];
    for (slot, (sum, count)) in out.iter_mut().zip(acc) {
        if *count > 0 {
            *slot = Some(*sum as f32 / *count as f32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze;
    use crate::test_support::{date, mood_entry, refs};

    fn mood(entries: &[Entry]) -> MoodAnalytics {
        analyze(&refs(entries), date(2024, 12, 31)).mood
    }

    #[test]
    fn mean_is_the_average_mood() {
        let entries = [
            mood_entry("2024-01-01T00:00:00Z", Some(-5), &[]),
            mood_entry("2024-01-02T00:00:00Z", Some(1), &[]),
            mood_entry("2024-01-03T00:00:00Z", Some(1), &[]),
            mood_entry("2024-01-04T00:00:00Z", Some(5), &[]),
        ];
        assert_eq!(mood(&entries).mean, Some(0.5));
    }

    #[test]
    fn sentiment_windows_track_trailing_days() {
        // `mood()` anchors today at 2024-12-31. mood>=2 positive, <=-2 negative.
        let entries = [
            mood_entry("2024-12-30T00:00:00Z", Some(4), &[]), // within week, positive
            mood_entry("2024-12-10T00:00:00Z", Some(-3), &[]), // within month, not week, negative
            mood_entry("2024-06-01T00:00:00Z", Some(4), &[]), // within year, not month, positive
            mood_entry("2020-01-01T00:00:00Z", Some(-3), &[]), // all-time only, negative
        ];
        let mood = mood(&entries);
        // All-time: 2 positive, 2 negative.
        assert_eq!((mood.sentiment.positive, mood.sentiment.negative), (2, 2));
        let [year, month, week] = mood.sentiment_windows;
        assert_eq!((year.positive, year.negative), (2, 1)); // 2 calm, 1 sad in last 365d
        assert_eq!((month.positive, month.negative), (1, 1)); // 1 calm, 1 sad in last 30d
        assert_eq!((week.positive, week.negative), (1, 0)); // 1 calm in last 7d
    }

    #[test]
    fn seasonal_and_weekday_averages() {
        let entries = [
            // 2024-01-01 is a Monday.
            mood_entry("2024-01-01T00:00:00Z", Some(4), &[]),
            // 2024-02-05 is a Monday.
            mood_entry("2024-02-05T00:00:00Z", Some(2), &[]),
        ];
        let mood = mood(&entries);
        assert_eq!(mood.by_month[0], Some(4.0));
        assert_eq!(mood.by_month[1], Some(2.0));
        assert_eq!(mood.by_month[2], None);
        assert_eq!(mood.by_weekday[0], Some(3.0)); // both Mondays: (4+2)/2
    }

    #[test]
    fn sentiment_buckets_entries_by_mood() {
        let entries = [
            mood_entry("2024-01-01T00:00:00Z", Some(4), &[]), // positive
            mood_entry("2024-01-02T00:00:00Z", Some(1), &[]), // neutral (within band)
            mood_entry("2024-01-03T00:00:00Z", Some(-1), &[]), // neutral (within band)
            mood_entry("2024-01-04T00:00:00Z", Some(-3), &[]), // negative
            mood_entry("2024-01-05T00:00:00Z", None, &["calm"]), // no mood -> excluded
        ];
        let mood = mood(&entries);
        assert_eq!(mood.sentiment.positive, 1);
        assert_eq!(mood.sentiment.neutral, 2);
        assert_eq!(mood.sentiment.negative, 1);
        assert_eq!(mood.sentiment.positive_negative_ratio(), Some(1.0));
    }

    #[test]
    fn best_and_worst_periods() {
        let entries = [
            mood_entry("2023-01-01T00:00:00Z", Some(-3), &[]),
            mood_entry("2024-01-01T00:00:00Z", Some(4), &[]),
        ];
        let mood = mood(&entries);
        assert_eq!(mood.best_period.unwrap().label, "2024");
        assert_eq!(mood.worst_period.unwrap().label, "2023");
    }

    #[test]
    fn ratio_is_none_without_negatives() {
        let entries = [mood_entry("2024-01-01T00:00:00Z", Some(4), &[])];
        assert!(mood(&entries).sentiment.positive_negative_ratio().is_none());
    }
}
