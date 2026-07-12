use std::{hint::black_box, path::PathBuf, time::Instant};

use chrono::{NaiveDate, TimeDelta, TimeZone, Utc};
use notema_analytics::analyze;
use notema_domain::{Entry, EntryEncryptionState, Timestamp};

fn main() {
    for size in [1_000, 10_000, 25_000] {
        let entries = corpus(size);
        let refs = entries.iter().collect::<Vec<_>>();
        let iterations = if size < 10_000 { 20 } else { 5 };
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(analyze(
                black_box(&refs),
                NaiveDate::from_ymd_opt(2026, 7, 12).unwrap(),
            ));
        }
        let elapsed = started.elapsed() / iterations;
        println!("analytics/{size}: {elapsed:?}");
    }
}

fn corpus(size: usize) -> Vec<Entry> {
    let start = Utc.with_ymd_and_hms(2020, 1, 1, 8, 0, 0).unwrap();
    (0..size)
        .map(|index| {
            let created = start + TimeDelta::hours(i64::try_from(index).unwrap() * 7);
            Entry {
                id: format!("entry-{index}"),
                journal: format!("journal-{}", index % 4),
                path: PathBuf::from(format!("journal/entry-{index}.md")),
                encryption_state: EntryEncryptionState::Plain,
                created_at: Some(Timestamp::parse(created.to_rfc3339())),
                edited_at: None,
                preview: "Representative journal entry".to_string(),
                activities: vec![format!("activity-{}", index % 12)],
                feelings: vec![format!("feeling-{}", index % 8)],
                people: vec![format!("person-{}", index % 20)],
                tags: vec![format!("tag-{}", index % 30)],
                mood: Some(i8::try_from(index % 11).unwrap() - 5),
                starred: index % 17 == 0,
                location: None,
                weather: None,
                celestial: None,
                air_quality: None,
                import: None,
                body: "# Representative\n\nA body used by the analytics benchmark.".to_string(),
                word_count: 9,
                search_haystack: String::new(),
                warning: None,
            }
        })
        .collect()
}
