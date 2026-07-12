//! The Drivers tab: people, activities, and tags merged into one ranking of
//! what lifts vs. drains your mood. The strongest lifts come first (mood well
//! above your average), then the strongest drains (well below) — the single
//! screen that answers "what moves my mood". Rendered through the shared
//! [`super::correlate`] table, so it inherits the diverging lift/drain bar and
//! scrolling.

use notema_analytics::{Correlation, Correlations, by_mood_delta_asc, by_mood_delta_desc};

/// A value needs at least this many entries before it earns a place in the
/// ranking — one lucky good day shouldn't crown an activity as a lifter.
const MIN_COUNT: usize = 3;

/// Merge the correlation dimensions into a single lift-then-drain ranking. Values
/// with too few entries, or no mood signal, are dropped.
pub(super) fn rows(correlations: &Correlations) -> Vec<Correlation> {
    let merged: Vec<Correlation> = correlations
        .people
        .iter()
        .chain(&correlations.activities)
        .chain(&correlations.tags)
        .filter(|correlate| correlate.count >= MIN_COUNT)
        .cloned()
        .collect();

    let lifts = by_mood_delta_desc(&merged)
        .into_iter()
        .filter(|correlate| correlate.mood_delta.is_some_and(|delta| delta > 0.0));
    let drains = by_mood_delta_asc(&merged)
        .into_iter()
        .filter(|correlate| correlate.mood_delta.is_some_and(|delta| delta < 0.0));
    lifts.chain(drains).collect()
}
