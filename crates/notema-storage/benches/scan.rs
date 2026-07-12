//! Time a full journal scan (walk + parse + preview + search-haystack build)
//! over deterministic 1k/10k/25k corpora. Mirrors the analytics bench: a plain
//! `Instant`-timed binary (`harness = false`), no external bench framework.

use std::{fs, hint::black_box, time::Instant};

use notema_storage::JournalStore;

fn main() {
    for size in [1_000, 10_000, 25_000] {
        let dir = tempfile::tempdir().unwrap();
        let store = build_store(dir.path(), size);
        let iterations = if size < 10_000 { 10 } else { 3 };

        // Warm the page cache so the measurement reflects parse/aggregate cost,
        // not the first cold read of the freshly written tree.
        black_box(store.scan_entries().unwrap());

        let started = Instant::now();
        for _ in 0..iterations {
            black_box(store.scan_entries().unwrap());
        }
        let elapsed = started.elapsed() / iterations;
        println!("scan/{size}: {elapsed:?}");
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
