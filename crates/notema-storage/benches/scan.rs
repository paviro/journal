//! Compare source parsing, immediate cache decode, and cache validation over
//! deterministic 1k/10k/25k corpora. Plain `Instant` timing, no framework.

use std::{fs, hint::black_box, time::Instant};

use notema_storage::{CachePolicy, JournalStore};

fn main() {
    for size in [1_000, 10_000, 25_000] {
        let dir = tempfile::tempdir().unwrap();
        let store = build_store(dir.path(), size);
        let iterations = if size < 10_000 { 10 } else { 3 };

        // Materialize both filesystem pages and the derived cache before timing.
        black_box(store.load_library(CachePolicy::Rebuild).unwrap());

        let started = Instant::now();
        for _ in 0..iterations {
            black_box(store.load_library(CachePolicy::Off).unwrap());
        }
        let elapsed = started.elapsed() / iterations;
        println!("source/{size}: {elapsed:?}");

        let started = Instant::now();
        for _ in 0..iterations {
            let cache = store.read_cached_library(CachePolicy::Normal).unwrap();
            black_box(cache.cached.as_ref().unwrap().snapshot());
        }
        let elapsed = started.elapsed() / iterations;
        println!("cache-decode/{size}: {elapsed:?}");

        let started = Instant::now();
        for _ in 0..iterations {
            let cache = store.read_cached_library(CachePolicy::Normal).unwrap();
            black_box(
                store
                    .validate_library(cache.cached, CachePolicy::Normal)
                    .unwrap(),
            );
        }
        let elapsed = started.elapsed() / iterations;
        println!("validate/{size}: {elapsed:?}");
    }
}

/// A store on disk holding `size` plaintext entries spread across four journals,
/// each with representative metadata and a short markdown body.
fn build_store(root: &std::path::Path, size: usize) -> JournalStore {
    let journal_root = root.join("journals");
    let store = JournalStore::new(&journal_root, root);
    store.ensure().unwrap();

    for index in 0..4 {
        fs::create_dir_all(journal_root.join(format!("journal-{index}"))).unwrap();
    }

    for index in 0..size {
        let journal = index % 4;
        let dir = journal_root
            .join(format!("journal-{journal}"))
            .join(format!(
                "2020-{:02}-{:02}",
                1 + (index % 12),
                1 + (index % 28)
            ));
        fs::create_dir_all(&dir).unwrap();
        let stamp = format!(
            "2020-{:02}-{:02}T{:02}-00-00",
            1 + (index % 12),
            1 + (index % 28),
            index % 24
        );
        fs::write(
            dir.join(format!("{stamp}-{index:05}.md")),
            entry_text(index),
        )
        .unwrap();
    }

    store
}

fn entry_text(index: usize) -> String {
    format!(
        "+++\n\
         schema_version = 1\n\
         tags = [\"tag-{}\", \"tag-{}\"]\n\
         people = [\"person-{}\"]\n\
         activities = [\"activity-{}\"]\n\
         mood = {}\n\
         [datetime]\n\
         created_at = \"2020-01-01T08:00:00+00:00\"\n\
         +++\n\n\
         # Entry {index}\n\n\
         A representative journal body with some **bold** text, a [link](https://example.com),\n\
         and a short list:\n\n\
         - first point\n\
         - second point\n\n\
         Closing line for entry {index}.\n",
        index % 30,
        index % 15,
        index % 20,
        index % 12,
        (index % 11) as i8 - 5,
    )
}
