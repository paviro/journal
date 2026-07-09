//! Per-value correlations: for each person, activity, and tag, how often it
//! appears, the mood logged alongside it, its dominant feeling, and how far its
//! mood sits from the journal's overall average ("what lifts / drains you").

use std::collections::HashMap;

use journal_core::Entry;

use crate::sort_by_count_desc;

/// A person, activity, or tag with its co-occurring mood and feeling.
#[derive(Debug, Clone, PartialEq)]
pub struct Correlate {
    pub name: String,
    pub count: usize,
    /// Average mood across the co-occurring entries that carry a mood, or `None`
    /// when none do.
    pub avg_mood: Option<f32>,
    /// `avg_mood` minus the journal's overall mean mood: positive means entries
    /// with this value skew happier than average, negative sadder. `None` when
    /// `avg_mood` is `None` or the journal has no mood data.
    pub mood_delta: Option<f32>,
    /// The feelings most often logged alongside this value as `(feeling, count)`,
    /// most common first (capped at a few), so a row can show which feelings ride
    /// with it and how often.
    pub top_feelings: Vec<(String, usize)>,
}

/// Correlations for each metadata dimension. Each list is sorted most-frequent
/// first; use [`by_mood_delta_desc`] / [`by_mood_delta_asc`] for a mood ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct Correlations {
    pub people: Vec<Correlate>,
    pub activities: Vec<Correlate>,
    pub tags: Vec<Correlate>,
    /// Feelings treated as a correlated dimension: each feeling's co-occurring
    /// mood, so a mood ranking answers "which feeling rides with my best/worst
    /// mood". `top_feelings` here is usually the feeling itself and is unused.
    pub feelings: Vec<Correlate>,
}

/// Per-value accumulator for the correlation pass.
#[derive(Default)]
struct Acc {
    count: usize,
    mood_sum: i64,
    mood_count: usize,
    feelings: HashMap<String, usize>,
}

/// Build correlations over an arbitrary slice of entries. `mood_delta` is always
/// relative to *this slice's* mean mood, so a windowed slice (e.g. the last 30
/// days) yields deltas against that window's baseline rather than all-time.
pub fn build_correlations(entries: &[&Entry]) -> Correlations {
    let mut people: HashMap<String, Acc> = HashMap::new();
    let mut activities: HashMap<String, Acc> = HashMap::new();
    let mut tags: HashMap<String, Acc> = HashMap::new();
    let mut feelings: HashMap<String, Acc> = HashMap::new();
    let mut mood_sum: i64 = 0;
    let mut mood_count: usize = 0;

    for entry in entries {
        accumulate(&mut people, &entry.metadata.people, entry, false);
        accumulate(&mut activities, &entry.metadata.activities, entry, false);
        accumulate(&mut tags, &entry.metadata.tags, entry, false);
        // The feelings dimension excludes each feeling from its own associated
        // list, so `top_feelings` reads as "often logged *together* with this one".
        accumulate(&mut feelings, &entry.metadata.feelings, entry, true);
        if let Some(mood) = entry.metadata.mood {
            mood_sum += i64::from(mood);
            mood_count += 1;
        }
    }

    let overall_mean = (mood_count > 0).then(|| mood_sum as f32 / mood_count as f32);
    Correlations {
        people: finish(people, overall_mean),
        activities: finish(activities, overall_mean),
        tags: finish(tags, overall_mean),
        feelings: finish(feelings, overall_mean),
    }
}

fn accumulate(
    map: &mut HashMap<String, Acc>,
    values: &[String],
    entry: &Entry,
    exclude_self: bool,
) {
    for value in values {
        let acc = map.entry(value.clone()).or_default();
        acc.count += 1;
        if let Some(mood) = entry.metadata.mood {
            acc.mood_sum += i64::from(mood);
            acc.mood_count += 1;
        }
        for feeling in &entry.metadata.feelings {
            if exclude_self && feeling == value {
                continue;
            }
            *acc.feelings.entry(feeling.clone()).or_default() += 1;
        }
    }
}

fn finish(map: HashMap<String, Acc>, overall_mean: Option<f32>) -> Vec<Correlate> {
    let mut correlates: Vec<Correlate> = map
        .into_iter()
        .map(|(name, acc)| {
            let avg_mood =
                (acc.mood_count > 0).then(|| acc.mood_sum as f32 / acc.mood_count as f32);
            Correlate {
                name,
                count: acc.count,
                avg_mood,
                mood_delta: match (avg_mood, overall_mean) {
                    (Some(avg), Some(mean)) => Some(avg - mean),
                    _ => None,
                },
                top_feelings: top_feelings(acc.feelings),
            }
        })
        .collect();
    sort_by_count_desc(&mut correlates, |c| (c.count, c.name.as_str()));
    correlates
}

/// The feelings that co-occur with a value as `(feeling, count)`, most common
/// first, capped so a row shows a handful rather than an unbounded list.
fn top_feelings(feelings: HashMap<String, usize>) -> Vec<(String, usize)> {
    const MAX: usize = 3;
    let mut tallies: Vec<(String, usize)> = feelings.into_iter().collect();
    sort_by_count_desc(&mut tallies, |(name, count)| (*count, name.as_str()));
    tallies.truncate(MAX);
    tallies
}

/// Clone `correlates` ranked by `mood_delta` descending — the values that most
/// lift the mood first. Entries without a `mood_delta` sort to the end.
pub fn by_mood_delta_desc(correlates: &[Correlate]) -> Vec<Correlate> {
    let mut ranked = correlates.to_vec();
    ranked.sort_by(|a, b| cmp_delta(a, b, true));
    ranked
}

/// Clone `correlates` ranked by `mood_delta` ascending — the values that most
/// drain the mood first. Entries without a `mood_delta` sort to the end.
pub fn by_mood_delta_asc(correlates: &[Correlate]) -> Vec<Correlate> {
    let mut ranked = correlates.to_vec();
    ranked.sort_by(|a, b| cmp_delta(a, b, false));
    ranked
}

/// Order two correlates by `mood_delta`, keeping `None` deltas last in either
/// direction and breaking ties by name.
fn cmp_delta(a: &Correlate, b: &Correlate, desc: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.mood_delta, b.mood_delta) {
        (Some(x), Some(y)) => {
            let by_value = x.total_cmp(&y);
            let by_value = if desc { by_value.reverse() } else { by_value };
            by_value.then_with(|| a.name.cmp(&b.name))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze;
    use crate::test_support::{date, entry_with, refs};
    use journal_core::Timestamp;

    /// An entry with a mood, people, and feelings.
    fn entry(created: &str, mood: Option<i8>, people: &[&str], feelings: &[&str]) -> Entry {
        entry_with(|entry| {
            entry.created_at = Some(Timestamp::parse(created));
            entry.metadata.mood = mood;
            entry.metadata.people = people.iter().map(|s| s.to_string()).collect();
            entry.metadata.feelings = feelings.iter().map(|s| s.to_string()).collect();
        })
    }

    #[test]
    fn count_avg_mood_and_top_feeling() {
        let entries = [
            entry(
                "2024-01-01T00:00:00Z",
                Some(4),
                &["alex"],
                &["calm", "happy"],
            ),
            entry("2024-01-02T00:00:00Z", Some(2), &["alex"], &["calm"]),
            entry("2024-01-03T00:00:00Z", None, &["sam"], &["sad"]),
        ];
        let people = analyze(&refs(&entries), date(2024, 1, 3))
            .correlations
            .people;
        let alex = &people[0];
        assert_eq!(alex.name, "alex");
        assert_eq!(alex.count, 2);
        assert_eq!(alex.avg_mood, Some(3.0));
        // calm logged twice, happy once → most-common first, with counts.
        assert_eq!(
            alex.top_feelings,
            [("calm".to_string(), 2), ("happy".to_string(), 1)]
        );

        let sam = &people[1];
        assert_eq!(sam.avg_mood, None);
        assert_eq!(sam.mood_delta, None);
    }

    #[test]
    fn mood_delta_is_relative_to_overall_mean() {
        // Overall mean mood = (4 + 2 + 0) / 3 = 2.0.
        let entries = [
            entry("2024-01-01T00:00:00Z", Some(4), &["lift"], &[]),
            entry("2024-01-02T00:00:00Z", Some(2), &["mid"], &[]),
            entry("2024-01-03T00:00:00Z", Some(0), &["drain"], &[]),
        ];
        let people = analyze(&refs(&entries), date(2024, 1, 3))
            .correlations
            .people;

        let desc = by_mood_delta_desc(&people);
        assert_eq!(desc[0].name, "lift");
        assert_eq!(desc[0].mood_delta, Some(2.0));
        assert_eq!(desc.last().unwrap().name, "drain");
        assert_eq!(desc.last().unwrap().mood_delta, Some(-2.0));

        let asc = by_mood_delta_asc(&people);
        assert_eq!(asc[0].name, "drain");
    }

    #[test]
    fn feelings_are_correlated_to_mood() {
        // Overall mean mood = (5 + 1) / 2 = 3.0.
        let entries = [
            entry("2024-01-01T00:00:00Z", Some(5), &[], &["grateful"]),
            entry("2024-01-02T00:00:00Z", Some(1), &[], &["anxious"]),
        ];
        let feelings = analyze(&refs(&entries), date(2024, 1, 2))
            .correlations
            .feelings;

        let lifts = by_mood_delta_desc(&feelings);
        assert_eq!(lifts[0].name, "grateful");
        assert_eq!(lifts[0].avg_mood, Some(5.0));
        assert_eq!(lifts[0].mood_delta, Some(2.0));
        assert_eq!(lifts.last().unwrap().name, "anxious");
        assert_eq!(lifts.last().unwrap().mood_delta, Some(-2.0));
    }

    #[test]
    fn moodless_correlates_sort_last_in_both_directions() {
        let entries = [
            entry("2024-01-01T00:00:00Z", Some(3), &["known"], &[]),
            entry("2024-01-02T00:00:00Z", None, &["unknown"], &[]),
        ];
        let people = analyze(&refs(&entries), date(2024, 1, 2))
            .correlations
            .people;
        assert_eq!(by_mood_delta_desc(&people).last().unwrap().name, "unknown");
        assert_eq!(by_mood_delta_asc(&people).last().unwrap().name, "unknown");
    }
}
