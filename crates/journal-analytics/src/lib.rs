//! Pure aggregation over journal entries, decoupled from the TUI. [`analyze`]
//! folds a borrowed slice of entries into an [`Analytics`] snapshot covering
//! three families — writing cadence, mood/emotion, and per-value correlations —
//! that the render layer can turn into panels. No I/O, no rendering: given the
//! same entries and `today`, the output is deterministic.

use chrono::{Datelike, NaiveDate};
use journal_core::{Entry, entry_group_date};

pub mod cadence;
pub mod correlations;
pub mod mood;

pub use cadence::Cadence;
pub use correlations::{Correlate, Correlations, build_correlations};
pub use mood::{MoodAnalytics, Sentiment};

/// The full set of aggregates for one set of entries.
#[derive(Debug, Clone, PartialEq)]
pub struct Analytics {
    pub cadence: Cadence,
    pub mood: MoodAnalytics,
    pub correlations: Correlations,
    pub highlights: Highlights,
}

/// The headline picks the Overview tab surfaces, each derived from the aggregates
/// above: who and what move your mood, and this year's dominant feeling.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Highlights {
    /// A person who rides with your better-than-average moods, and an activity or
    /// tag that does the same. Each is chosen from those within reach of the
    /// strongest lift in its group and rotated by the day, so "what lifts you"
    /// names a companion *and* a thing, and varies instead of fixing on one value.
    pub lifts_person: Option<String>,
    pub lifts_thing: Option<String>,
    /// The mirror of `lifts_*`: a person and a thing that ride with your
    /// worse-than-average moods, chosen and rotated the same way.
    pub drains_person: Option<String>,
    pub drains_thing: Option<String>,
    /// The feeling logged on the most entries dated in the current year, or `None`
    /// when this year has no feeling yet (callers can fall back to all-time).
    pub top_feeling_this_year: Option<String>,
}

/// Average of a metric over one calendar period (a year, or a month when every
/// entry falls in a single year). Shared by the mood series and any other
/// time-bucketed view. `count` is the number of contributing entries.
#[derive(Debug, Clone, PartialEq)]
pub struct MoodBucket {
    pub label: String,
    pub avg: f32,
    pub count: usize,
}

/// A value and how many entries carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    pub name: String,
    pub count: usize,
}

/// Aggregate `entries` into an [`Analytics`] snapshot. `today` anchors the
/// "current streak" calculation and is passed in (not read from the clock) so
/// the function stays pure and testable.
pub fn analyze(entries: &[&Entry], today: NaiveDate) -> Analytics {
    // Resolve each entry's grouping date once; every family reuses it.
    let dates: Vec<Option<NaiveDate>> = entries
        .iter()
        .map(|entry| entry_group_date(entry))
        .collect();
    // Decide the period granularity once from the whole span so the cadence
    // histogram and the mood series always agree on year-vs-month buckets.
    let by_year = multi_year(dates.iter().flatten().map(|date| date.year()));

    let correlations = correlations::build_correlations(entries);
    let lifts_person = pick_extreme(correlations.people.iter(), today, true);
    let lifts_thing = pick_extreme(
        correlations.activities.iter().chain(&correlations.tags),
        today,
        true,
    );
    let drains_person = pick_extreme(correlations.people.iter(), today, false);
    let drains_thing = pick_extreme(
        correlations.activities.iter().chain(&correlations.tags),
        today,
        false,
    );
    let top_feeling_this_year = top_feeling_in_year(entries, &dates, today.year());

    Analytics {
        cadence: cadence::build(entries, &dates, by_year, today),
        mood: mood::build(entries, &dates, by_year, today),
        correlations,
        highlights: Highlights {
            lifts_person,
            lifts_thing,
            drains_person,
            drains_thing,
            top_feeling_this_year,
        },
    }
}

/// Pick a value from `correlates` whose mood sits farthest from average — the
/// strongest lift when `positive`, the strongest drain otherwise. Chosen from
/// those within 15% of that extreme and rotated by `today`'s ordinal so the pick
/// changes day to day. `None` when nothing pulls mood in the requested direction.
fn pick_extreme<'a>(
    correlates: impl Iterator<Item = &'a Correlate>,
    today: NaiveDate,
    positive: bool,
) -> Option<String> {
    let want = |delta: f32| if positive { delta > 0.0 } else { delta < 0.0 };
    let mut candidates: Vec<&Correlate> = correlates
        .filter(|correlate| correlate.mood_delta.is_some_and(&want))
        .collect();
    // The extreme is the max lift or the min (most negative) drain; both stay
    // within 15% of it in magnitude.
    let extreme = candidates
        .iter()
        .filter_map(|correlate| correlate.mood_delta)
        .fold(0.0_f32, if positive { f32::max } else { f32::min });
    if !want(extreme) {
        return None;
    }
    let threshold = extreme * 0.85;
    candidates.retain(|correlate| {
        let delta = correlate.mood_delta.unwrap_or(0.0);
        if positive {
            delta >= threshold
        } else {
            delta <= threshold
        }
    });
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    let index = today.num_days_from_ce().rem_euclid(candidates.len() as i32) as usize;
    candidates
        .get(index)
        .map(|correlate| correlate.name.clone())
}

/// The feeling logged on the most entries dated in `year`; ties break
/// alphabetically. `None` when no this-year entry carries a feeling.
fn top_feeling_in_year(
    entries: &[&Entry],
    dates: &[Option<NaiveDate>],
    year: i32,
) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (entry, date) in entries.iter().zip(dates) {
        if date.map(|date| date.year()) == Some(year) {
            for feeling in &entry.feelings {
                *counts.entry(feeling.as_str()).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(name, _)| name.to_string())
}

/// Whether a set of years spans more than one distinct calendar year. Empty or
/// single-year input is `false` (month buckets).
pub(crate) fn multi_year(years: impl Iterator<Item = i32>) -> bool {
    let (min, max) = years.fold((i32::MAX, i32::MIN), |(min, max), year| {
        (min.min(year), max.max(year))
    });
    max > min
}

/// The sort key for one period bucket: `(year, month)`, with month `0` in the
/// per-year mode so the key stays chronological either way.
pub(crate) fn period_key(date: NaiveDate, by_year: bool) -> (i32, u32) {
    if by_year {
        (date.year(), 0)
    } else {
        (date.year(), date.month())
    }
}

/// The display label for a period bucket key.
pub(crate) fn period_label(year: i32, month: u32, by_year: bool) -> String {
    if by_year {
        year.to_string()
    } else {
        month_abbrev(month).to_string()
    }
}

/// Sort descending by count, breaking ties alphabetically for stable output.
pub(crate) fn sort_by_count_desc<T>(items: &mut [T], key: impl Fn(&T) -> (usize, &str)) {
    items.sort_by(|a, b| {
        let (a_count, a_name) = key(a);
        let (b_count, b_name) = key(b);
        b_count.cmp(&a_count).then_with(|| a_name.cmp(b_name))
    });
}

pub(crate) fn month_abbrev(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES
        .get((month.max(1) - 1) as usize)
        .copied()
        .unwrap_or("?")
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    use chrono::NaiveDate;
    use journal_core::{Entry, EntryEncryptionState, Timestamp};

    /// Build a plain entry from defaults, letting the caller set only the fields
    /// a test cares about (created_at, word_count, metadata, id).
    pub(crate) fn entry_with(configure: impl FnOnce(&mut Entry)) -> Entry {
        let mut entry = Entry {
            id: "id".to_string(),
            journal: "journal".to_string(),
            path: PathBuf::from("journal/entry.md"),
            encryption_state: EntryEncryptionState::Plain,
            created_at: None,
            edited_at: None,
            preview: String::new(),
            activities: Vec::new(),
            feelings: Vec::new(),
            people: Vec::new(),
            tags: Vec::new(),
            mood: None,
            starred: false,
            location: None,
            import: None,
            body: String::new(),
            word_count: 0,
            search_haystack: String::new(),
        };
        configure(&mut entry);
        entry
    }

    /// A dated entry carrying a mood and a list of feelings.
    pub(crate) fn mood_entry(created: &str, mood: Option<i8>, feelings: &[&str]) -> Entry {
        entry_with(|entry| {
            entry.created_at = Some(Timestamp::parse(created));
            entry.mood = mood;
            entry.feelings = feelings.iter().map(|s| s.to_string()).collect();
        })
    }

    pub(crate) fn refs(entries: &[Entry]) -> Vec<&Entry> {
        entries.iter().collect()
    }

    pub(crate) fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{date, mood_entry, refs};
    use super::*;

    #[test]
    fn empty_input_yields_empty_analytics() {
        let analytics = analyze(&[], date(2024, 1, 1));
        assert_eq!(analytics.cadence.total_entries, 0);
        assert_eq!(analytics.cadence.active_days, 0);
        assert!(analytics.cadence.date_span.is_none());
        assert!(analytics.mood.series.is_empty());
        assert!(analytics.mood.mean.is_none());
        assert!(analytics.correlations.people.is_empty());
        assert!(analytics.highlights.drains_person.is_none());
        assert!(analytics.highlights.drains_thing.is_none());
    }

    #[test]
    fn single_year_buckets_by_month_multi_year_by_year() {
        assert!(!multi_year([2024, 2024].into_iter()));
        assert!(multi_year([2023, 2024].into_iter()));
        assert!(!multi_year(std::iter::empty()));
    }

    #[test]
    fn cadence_and_mood_agree_on_period_granularity() {
        // Two years of data → both the cadence histogram and the mood series
        // should use year labels, not months.
        let entries = [
            mood_entry("2023-05-01T00:00:00Z", Some(1), &[]),
            mood_entry("2024-05-01T00:00:00Z", Some(3), &[]),
        ];
        let analytics = analyze(&refs(&entries), date(2024, 6, 1));
        assert_eq!(analytics.cadence.per_period[0].name, "2023");
        assert_eq!(analytics.mood.series[0].label, "2023");
    }

    #[test]
    fn top_feeling_prefers_the_current_year() {
        let entries = [
            mood_entry("2023-01-01T00:00:00Z", None, &["sad"]),
            mood_entry("2023-02-01T00:00:00Z", None, &["sad"]),
            mood_entry("2024-01-01T00:00:00Z", None, &["calm"]),
            mood_entry("2024-02-01T00:00:00Z", None, &["calm"]),
            mood_entry("2024-03-01T00:00:00Z", None, &["tired"]),
        ];
        // 2024's most common feeling, not the all-time "sad".
        assert_eq!(
            analyze(&refs(&entries), date(2024, 6, 1))
                .highlights
                .top_feeling_this_year
                .as_deref(),
            Some("calm"),
        );
        // A year with no entries yet leaves the fallback to the caller.
        assert!(
            analyze(&refs(&entries), date(2025, 6, 1))
                .highlights
                .top_feeling_this_year
                .is_none()
        );
    }

    #[test]
    fn lifts_names_a_person_and_a_thing_that_raise_mood() {
        use super::test_support::entry_with;
        use journal_core::Timestamp;

        let dated = |created: &str, mood: i8, configure: fn(&mut journal_core::Entry)| {
            entry_with(|entry| {
                entry.created_at = Some(Timestamp::parse(created));
                entry.mood = Some(mood);
                configure(entry);
            })
        };
        let entries = [
            dated("2024-01-01T00:00:00Z", -3, |_| {}),
            dated("2024-01-02T00:00:00Z", -3, |_| {}),
            dated("2024-01-03T00:00:00Z", 5, |e| {
                e.people = vec!["gym-buddy".into()]
            }),
            dated("2024-01-04T00:00:00Z", 5, |e| e.tags = vec!["sun".into()]),
        ];
        // A companion comes from people, the thing from activities/tags.
        let analytics = analyze(&refs(&entries), date(2024, 1, 5));
        assert_eq!(
            analytics.highlights.lifts_person.as_deref(),
            Some("gym-buddy")
        );
        assert_eq!(analytics.highlights.lifts_thing.as_deref(), Some("sun"));
    }

    #[test]
    fn drains_names_a_person_and_a_thing_that_lower_mood() {
        use super::test_support::entry_with;
        use journal_core::Timestamp;

        let dated = |created: &str, mood: i8, configure: fn(&mut journal_core::Entry)| {
            entry_with(|entry| {
                entry.created_at = Some(Timestamp::parse(created));
                entry.mood = Some(mood);
                configure(entry);
            })
        };
        let entries = [
            dated("2024-01-01T00:00:00Z", 4, |_| {}),
            dated("2024-01-02T00:00:00Z", 4, |_| {}),
            dated("2024-01-03T00:00:00Z", -5, |e| e.people = vec!["ex".into()]),
            dated("2024-01-04T00:00:00Z", -5, |e| e.tags = vec!["rain".into()]),
        ];
        let analytics = analyze(&refs(&entries), date(2024, 1, 5));
        assert_eq!(analytics.highlights.drains_person.as_deref(), Some("ex"));
        assert_eq!(analytics.highlights.drains_thing.as_deref(), Some("rain"));
    }

    #[test]
    fn a_lift_pick_rotates_daily_within_its_group() {
        use super::test_support::entry_with;
        use journal_core::Timestamp;

        let with_person = |created: &str, mood: i8, person: &str| {
            let person = person.to_string();
            entry_with(move |entry| {
                entry.created_at = Some(Timestamp::parse(created));
                entry.mood = Some(mood);
                entry.people = vec![person];
            })
        };
        let entries = [
            entry_with(|e| {
                e.created_at = Some(Timestamp::parse("2024-01-01T00:00:00Z"));
                e.mood = Some(-3);
            }),
            entry_with(|e| {
                e.created_at = Some(Timestamp::parse("2024-01-02T00:00:00Z"));
                e.mood = Some(-3);
            }),
            with_person("2024-01-03T00:00:00Z", 5, "aaa"),
            with_person("2024-01-04T00:00:00Z", 5, "bbb"),
        ];
        // `aaa` and `bbb` lift equally, so consecutive days name different people.
        let day1 = analyze(&refs(&entries), date(2024, 1, 1))
            .highlights
            .lifts_person;
        let day2 = analyze(&refs(&entries), date(2024, 1, 2))
            .highlights
            .lifts_person;
        assert!(matches!(day1.as_deref(), Some("aaa" | "bbb")));
        assert_ne!(day1, day2);
    }
}
