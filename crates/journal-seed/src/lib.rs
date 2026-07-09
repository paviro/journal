//! Development-only sample-data generator. Fills a [`JournalStore`] with
//! backdated, richly tagged fake entries so the TUI, journal timeline, and
//! stats/analytics views have realistic data to render. Every generated entry
//! carries an `[import]` block of `source = "seed"`, so the fakes are
//! self-identifying and never mistaken for hand-written history.
//!
//! The data is *emotionally coherent*: a fictional author's mood follows a
//! smooth curve over the window (good stretches and rough patches, not per-entry
//! coin-flips), and each entry's feelings, body text, and starring line up with
//! that mood. The feeling-group → valence mapping used here lives only in this
//! generator to mimic what a real user would log; the product itself never
//! attaches good/bad judgment to a feeling (see `journal-core/src/feelings.rs`).

use chrono::{Duration, Local};
use journal_core::feelings::{self, FEELING_GROUPS};
use journal_core::{AppResult, ImportSource, MOOD_RANGE, Metadata};
use journal_storage::JournalStore;
use rand::{Rng, SeedableRng, rngs::StdRng};

/// The `source` value stamped into every generated entry's `[import]` block.
pub const SEED_SOURCE: &str = "seed";

/// Knobs for a generation run.
pub struct GenConfig {
    /// Journal to fill; created if it doesn't exist yet.
    pub journal: String,
    /// Number of entries to create.
    pub count: usize,
    /// Spread creation dates across the last `days` days.
    pub days: i64,
    /// Seed for reproducible datasets; entropy-seeded when `None`.
    pub seed: Option<u64>,
}

/// Which way an entry leans, so its mood, feelings, and text stay coherent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Valence {
    Positive,
    Neutral,
    Negative,
}

/// Classify a mood score into a valence using the same bands the insights view
/// uses (`journal-analytics/src/mood.rs`): `>= 2` positive, `<= -2` negative,
/// the `-1..=1` middle neutral. Kept in lockstep so seeded data lands where the
/// Balance/sentiment buckets expect it.
fn valence_from_mood(mood: i8) -> Valence {
    if mood >= 2 {
        Valence::Positive
    } else if mood <= -2 {
        Valence::Negative
    } else {
        Valence::Neutral
    }
}

const TAGS: &[&str] = &[
    "work",
    "home",
    "travel",
    "health",
    "reading",
    "coding",
    "family",
    "ideas",
    "gratitude",
    "fitness",
    "music",
    "cooking",
    "garden",
    "finance",
    "learning",
];

const PEOPLE: &[&str] = &[
    "Alice", "Bob", "Carla", "David", "Emma", "Frank", "Grace", "Hana", "Ivan", "Julia",
];

const ACTIVITIES: &[&str] = &[
    "running",
    "reading",
    "coding",
    "cooking",
    "a meeting",
    "a long walk",
    "cycling",
    "writing",
    "meditation",
    "gaming",
];

const POSITIVE_TITLES: &[&str] = &[
    "Small wins",
    "A good conversation",
    "Deep work",
    "Morning pages",
];
const NEUTRAL_TITLES: &[&str] = &["A quiet day", "Notes to self", "Weekend recap"];
const NEGATIVE_TITLES: &[&str] = &["Late night thoughts", "Rainy afternoon", "On the road"];

/// The original, mostly-reflective lines. Used for neutral entries and, rarely,
/// borrowed by positive/negative entries for texture.
const NEUTRAL_SENTENCES: &[&str] = &[
    "Woke up before the alarm and actually felt rested for once.",
    "Spent most of the afternoon untangling a problem that turned out to be simpler than I feared.",
    "The coffee shop on the corner had the window seat free, so I took it and lost an hour to reading.",
    "Made real progress on the thing I've been putting off all week.",
    "A walk cleared my head more than another hour at the desk would have.",
    "Cooked something new tonight and it almost worked.",
    "Kept getting pulled into small distractions, but the important part still got done.",
    "Had one of those conversations that reframes how you see a whole situation.",
    "The weather turned and everything smelled like rain and wet leaves.",
    "Wrote down three things I'm grateful for and meant all of them.",
    "Slept badly, so today ran on momentum more than energy.",
    "Finally closed the loop on a task that had been nagging at me for days.",
    "Sat outside long enough to notice the light change.",
    "Nothing dramatic happened, and that was exactly what I needed.",
];

const POSITIVE_SENTENCES: &[&str] = &[
    "Everything seemed to click today, and I rode that momentum for hours.",
    "Laughed until my cheeks hurt over something that wasn't even that funny.",
    "Felt genuinely proud of how far this has come.",
    "One of those days where the good news just kept arriving.",
    "Woke up light, like the week ahead was full of possibility.",
    "Got exactly the message I'd been hoping for and grinned at my phone like an idiot.",
    "Ended the day with the rare sense that everything is, for now, enough.",
];

const NEGATIVE_SENTENCES: &[&str] = &[
    "Couldn't shake a low, heavy feeling no matter what I tried.",
    "Everything took twice the effort and half of it fell apart anyway.",
    "Snapped at someone I care about and regretted it the moment it left my mouth.",
    "The same worry kept circling back long after I'd told myself to let it go.",
    "Felt worn thin, like there was nothing left to give by early afternoon.",
    "Lay awake replaying the parts of the day I wish had gone differently.",
    "Some days the small setbacks stack up until they don't feel small at all.",
];

/// Ensure the target journal exists, then create `config.count` backdated
/// entries. Returns the number of entries created.
pub fn generate(store: &JournalStore, config: &GenConfig) -> AppResult<usize> {
    let mut rng = match config.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_os_rng(),
    };

    ensure_journal(store, &config.journal)?;

    // Draw the mood curve and feeling pools once, from `rng`, so a run stays
    // reproducible from its seed and independent of the wall clock.
    let curve = MoodCurve::draw(&mut rng);
    let feelings = FeelingPools::build();

    let anchor = Local::now().fixed_offset();
    let window_secs = config.days.max(0) * 86_400;

    for _ in 0..config.count {
        let offset = if window_secs > 0 {
            rng.random_range(0..window_secs)
        } else {
            0
        };
        let created_at = anchor - Duration::seconds(offset);

        // Position within the window drives the mood curve. Derived from the
        // rng-drawn `offset`, never the timestamp, to keep bodies reproducible.
        let phase = if window_secs > 0 {
            offset as f32 / window_secs as f32
        } else {
            0.0
        };
        let target = curve.value(phase) + entry_noise(&mut rng);
        let mood_score = clamp_mood(target);
        let valence = valence_from_mood(mood_score);

        let metadata = random_metadata(&mut rng, valence, target, mood_score, &feelings);
        let body = random_body(&mut rng, valence);
        let import = ImportSource {
            source: SEED_SOURCE.to_string(),
            id: random_id(&mut rng),
        };

        store.create_imported_entry(
            &config.journal,
            &body,
            &metadata,
            created_at,
            created_at,
            None,
            None,
            None,
            None,
            None,
            &import,
        )?;
    }

    Ok(config.count)
}

/// Create the journal if no active or archived journal already carries its name.
fn ensure_journal(store: &JournalStore, name: &str) -> AppResult<()> {
    let exists = store
        .list_journals()?
        .iter()
        .any(|journal| journal_storage::journal_display_name(&journal.name) == name);
    if !exists {
        store.create_journal(name)?;
    }
    Ok(())
}

/// A smooth mood-over-time curve: a low-frequency sum of sines plus a gentle
/// linear trend, so the fictional author drifts through sustained good and rough
/// stretches rather than flipping mood every entry.
struct MoodCurve {
    offset: f32,
    trend: f32,
    waves: [Wave; 3],
}

struct Wave {
    amp: f32,
    /// Cycles across the whole window; kept small so stretches last.
    freq: f32,
    phase: f32,
}

impl MoodCurve {
    fn draw(rng: &mut StdRng) -> Self {
        let waves = std::array::from_fn(|_| Wave {
            amp: rng.random_range(0.6..=1.5),
            freq: rng.random_range(0.5..=3.0),
            phase: rng.random_range(0.0..std::f32::consts::TAU),
        });
        MoodCurve {
            offset: rng.random_range(-1.0..=1.0),
            trend: rng.random_range(-1.5..=1.5),
            waves,
        }
    }

    /// Baseline mood at `x` in `0.0..=1.0` (fraction into the window).
    fn value(&self, x: f32) -> f32 {
        let mut v = self.offset + self.trend * (x - 0.5);
        for wave in &self.waves {
            v += wave.amp * (wave.freq * std::f32::consts::TAU * x + wave.phase).sin();
        }
        v
    }
}

/// Per-entry jitter so two entries on the same stretch still differ. Sum of two
/// uniforms → a triangular, gaussian-ish spread centred on zero.
fn entry_noise(rng: &mut StdRng) -> f32 {
    (rng.random_range(-1.0f32..=1.0) + rng.random_range(-1.0f32..=1.0)) * 0.9
}

/// Round a curve value to a mood score within [`MOOD_RANGE`].
fn clamp_mood(target: f32) -> i8 {
    let lo = f32::from(*MOOD_RANGE.start());
    let hi = f32::from(*MOOD_RANGE.end());
    target.round().clamp(lo, hi) as i8
}

/// Canonical feelings split into valence buckets by their group. This mapping is
/// a generation convenience only — see the module docs.
struct FeelingPools {
    positive: Vec<&'static str>,
    neutral: Vec<&'static str>,
    negative: Vec<&'static str>,
    all: Vec<&'static str>,
}

impl FeelingPools {
    fn build() -> Self {
        let mut positive = Vec::new();
        let mut neutral = Vec::new();
        let mut negative = Vec::new();
        for group in FEELING_GROUPS {
            let bucket = match group.name {
                "Joy & Delight"
                | "Gratitude & Appreciation"
                | "Interest, Focus & Energy"
                | "Love & Connection"
                | "Peace & Ease"
                | "Safety, Trust & Hope"
                | "Confidence & Agency" => &mut positive,
                // Surprise is genuinely ambiguous, so it rides with the steady set.
                "Neutral & Steady" | "Surprise & Startle" => &mut neutral,
                _ => &mut negative,
            };
            bucket.extend(group.feelings.iter().map(|feeling| feeling.name));
        }
        let all = feelings::feelings().collect();
        FeelingPools {
            positive,
            neutral,
            negative,
            all,
        }
    }

    fn primary(&self, valence: Valence) -> &[&'static str] {
        match valence {
            Valence::Positive => &self.positive,
            Valence::Neutral => &self.neutral,
            Valence::Negative => &self.negative,
        }
    }
}

fn random_metadata(
    rng: &mut StdRng,
    valence: Valence,
    target: f32,
    mood_score: i8,
    feelings: &FeelingPools,
) -> Metadata {
    let count = rng.random_range(0..=3);
    let feelings = pick_distinct(rng, feelings.primary(valence), &feelings.all, 0.12, count)
        .into_iter()
        .map(str::to_string)
        .collect();
    Metadata {
        tags: sample(rng, TAGS, 0, 3),
        people: sample(rng, PEOPLE, 0, 2),
        activities: sample(rng, ACTIVITIES, 0, 2),
        feelings,
        mood: Some(mood_score),
        starred: rng.random_bool(star_probability(target)),
    }
}

/// Memorable days get starred more: strong highs most, strong lows a little,
/// ordinary days rarely. Averages out near the old flat ~15%.
fn star_probability(target: f32) -> f64 {
    if target >= 3.5 {
        0.45
    } else if target >= 2.0 {
        0.30
    } else if target <= -3.5 {
        0.28
    } else if target <= -2.0 {
        0.18
    } else {
        0.10
    }
}

fn random_body(rng: &mut StdRng, valence: Valence) -> String {
    let title = pick_distinct(rng, title_pool(valence), NEUTRAL_TITLES, 0.15, 1)
        .first()
        .copied()
        .unwrap_or("Notes to self");
    let paragraphs = rng.random_range(1..=3);
    let mut body = format!("# {title}\n");
    for _ in 0..paragraphs {
        let count = rng.random_range(2..=4);
        let sentences = pick_distinct(rng, sentence_pool(valence), NEUTRAL_SENTENCES, 0.15, count);
        body.push('\n');
        body.push_str(&sentences.join(" "));
        body.push('\n');
    }
    body
}

fn title_pool(valence: Valence) -> &'static [&'static str] {
    match valence {
        Valence::Positive => POSITIVE_TITLES,
        Valence::Neutral => NEUTRAL_TITLES,
        Valence::Negative => NEGATIVE_TITLES,
    }
}

fn sentence_pool(valence: Valence) -> &'static [&'static str] {
    match valence {
        Valence::Positive => POSITIVE_SENTENCES,
        Valence::Neutral => NEUTRAL_SENTENCES,
        Valence::Negative => NEGATIVE_SENTENCES,
    }
}

/// A short random id for the `[import]` provenance block.
fn random_id(rng: &mut StdRng) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..12)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Pick `count` distinct items, mostly from `primary` but with `secondary_prob`
/// chance per pick of reaching into `secondary` — the cross-valence "noise" that
/// keeps the data from looking too clean. Returns fewer than `count` only if the
/// pools can't supply enough distinct items.
fn pick_distinct<'a>(
    rng: &mut StdRng,
    primary: &[&'a str],
    secondary: &[&'a str],
    secondary_prob: f64,
    count: usize,
) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    let max_attempts = count * 8 + 8;
    for _ in 0..max_attempts {
        if out.len() >= count {
            break;
        }
        let pool = if !secondary.is_empty() && rng.random_bool(secondary_prob) {
            secondary
        } else {
            primary
        };
        if pool.is_empty() {
            break;
        }
        let pick = pool[rng.random_range(0..pool.len())];
        if !out.contains(&pick) {
            out.push(pick);
        }
    }
    out
}

/// Pick between `min` and `max` distinct items from `pool` (capped at the pool
/// size), preserving no particular order.
fn sample(rng: &mut StdRng, pool: &[&str], min: usize, max: usize) -> Vec<String> {
    let max = max.min(pool.len());
    let min = min.min(max);
    let take = rng.random_range(min..=max);
    let mut indices: Vec<usize> = (0..pool.len()).collect();
    // Partial Fisher–Yates: shuffle just the first `take` slots.
    for i in 0..take {
        let j = rng.random_range(i..indices.len());
        indices.swap(i, j);
    }
    indices[..take]
        .iter()
        .map(|&i| pool[i].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generates_backdated_seed_entries() {
        let dir = TempDir::new().unwrap();
        let store = JournalStore::new(dir.path().join("journals"), dir.path());
        store.ensure().unwrap();

        let config = GenConfig {
            journal: "Sample".to_string(),
            count: 25,
            days: 180,
            seed: Some(1),
        };
        let created = generate(&store, &config).unwrap();
        assert_eq!(created, 25);

        let entries = store.scan_entries().unwrap();
        assert_eq!(entries.len(), 25);

        let now = Local::now().fixed_offset();
        let window_start = now - Duration::days(180);
        for entry in &entries {
            assert_eq!(entry.import.as_ref().unwrap().source, SEED_SOURCE);
            let created = entry
                .created_at
                .as_ref()
                .and_then(|ts| ts.parsed)
                .expect("created_at parses");
            assert!(created >= window_start && created <= now);
        }
    }

    #[test]
    fn same_seed_is_reproducible() {
        let render = |seed| {
            let dir = TempDir::new().unwrap();
            let store = JournalStore::new(dir.path().join("journals"), dir.path());
            store.ensure().unwrap();
            generate(
                &store,
                &GenConfig {
                    journal: "Sample".to_string(),
                    count: 10,
                    days: 90,
                    seed: Some(seed),
                },
            )
            .unwrap();
            let mut bodies: Vec<String> = store
                .scan_entries()
                .unwrap()
                .iter()
                .map(|entry| entry.content.clone())
                .collect();
            bodies.sort();
            bodies
        };
        assert_eq!(render(42), render(42));
    }

    #[test]
    fn valence_tracks_the_analytics_bands() {
        assert_eq!(valence_from_mood(5), Valence::Positive);
        assert_eq!(valence_from_mood(2), Valence::Positive);
        assert_eq!(valence_from_mood(1), Valence::Neutral);
        assert_eq!(valence_from_mood(-1), Valence::Neutral);
        assert_eq!(valence_from_mood(-2), Valence::Negative);
        assert_eq!(valence_from_mood(-5), Valence::Negative);
    }

    #[test]
    fn feeling_pools_classify_every_feeling_once() {
        let pools = FeelingPools::build();
        let total = pools.positive.len() + pools.neutral.len() + pools.negative.len();
        assert_eq!(total, feelings::feelings().count());
        assert!(pools.positive.contains(&"happy"));
        assert!(pools.positive.contains(&"grateful"));
        assert!(pools.negative.contains(&"angry"));
        assert!(pools.negative.contains(&"sad"));
        assert!(pools.neutral.contains(&"neutral"));
        assert!(!pools.positive.contains(&"angry"));
    }

    #[test]
    fn mood_curve_drifts_over_the_window() {
        // The whole point: the baseline is not flat, so the insights chart shows
        // good and rough stretches. Deterministic under a fixed seed.
        let mut rng = StdRng::seed_from_u64(7);
        let curve = MoodCurve::draw(&mut rng);
        let values: Vec<f32> = (0..=100).map(|i| curve.value(i as f32 / 100.0)).collect();
        let max = values.iter().copied().fold(f32::MIN, f32::max);
        let min = values.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            max - min > 1.5,
            "baseline should vary over time: {min}..{max}"
        );
    }

    #[test]
    fn positive_entries_draw_mostly_positive_feelings() {
        let pools = FeelingPools::build();
        let mut rng = StdRng::seed_from_u64(3);
        let (mut on_valence, mut total) = (0usize, 0usize);
        for _ in 0..500 {
            let picks = pick_distinct(&mut rng, &pools.positive, &pools.all, 0.12, 3);
            for pick in picks {
                total += 1;
                if pools.positive.contains(&pick) {
                    on_valence += 1;
                }
            }
        }
        assert!(total > 0);
        let ratio = on_valence as f32 / total as f32;
        assert!(
            ratio > 0.8,
            "expected mostly positive feelings, got {ratio}"
        );
    }
}
