//! Development-only sample-data generator. Fills a [`JournalStore`] with
//! backdated, richly tagged fake entries so the TUI, journal timeline, and
//! stats/analytics views have realistic data to render. Every generated entry
//! carries an `[import]` block of `source = "seed"`, so the fakes are
//! self-identifying and never mistaken for hand-written history.

use chrono::{Duration, Local};
use journal_core::feelings;
use journal_storage::{AppResult, ImportSource, JournalStore, MOOD_RANGE, Metadata};
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

const TITLES: &[&str] = &[
    "Morning pages",
    "A quiet day",
    "Notes to self",
    "Weekend recap",
    "Late night thoughts",
    "On the road",
    "Small wins",
    "Rainy afternoon",
    "Deep work",
    "A good conversation",
];

const SENTENCES: &[&str] = &[
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

/// Ensure the target journal exists, then create `config.count` backdated
/// entries. Returns the number of entries created.
pub fn generate(store: &JournalStore, config: &GenConfig) -> AppResult<usize> {
    let mut rng = match config.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_os_rng(),
    };

    ensure_journal(store, &config.journal)?;

    let anchor = Local::now().fixed_offset();
    let window_secs = config.days.max(0) * 86_400;

    for _ in 0..config.count {
        let offset = if window_secs > 0 {
            rng.random_range(0..window_secs)
        } else {
            0
        };
        let created_at = anchor - Duration::seconds(offset);
        let metadata = random_metadata(&mut rng);
        let body = random_body(&mut rng);
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

fn random_metadata(rng: &mut StdRng) -> Metadata {
    let feelings: Vec<&str> = feelings::feelings().collect();
    Metadata {
        tags: sample(rng, TAGS, 0, 3),
        people: sample(rng, PEOPLE, 0, 2),
        activities: sample(rng, ACTIVITIES, 0, 2),
        feelings: sample(rng, &feelings, 0, 3),
        mood: rng
            .random_bool(0.7)
            .then(|| rng.random_range(*MOOD_RANGE.start()..=*MOOD_RANGE.end())),
        starred: rng.random_bool(0.15),
    }
}

fn random_body(rng: &mut StdRng) -> String {
    let title = TITLES[rng.random_range(0..TITLES.len())];
    let paragraphs = rng.random_range(1..=3);
    let mut body = format!("# {title}\n");
    for _ in 0..paragraphs {
        let sentences = rng.random_range(2..=4);
        let paragraph = sample(rng, SENTENCES, sentences, sentences).join(" ");
        body.push('\n');
        body.push_str(&paragraph);
        body.push('\n');
    }
    body
}

/// A short random id for the `[import]` provenance block.
fn random_id(rng: &mut StdRng) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..12)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
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
}
