//! TUI hot-path benchmarks: in-memory word search and a full-frame render over
//! deterministic 1k/10k/25k corpora. Plain `Instant` timing (`harness = false`),
//! matching the analytics and storage scan benches. Needs `--features bench`,
//! which the `[[bench]]` entry requires automatically.

use std::{hint::black_box, time::Instant};

use notema::bench::{app_with_entries, draw_frame, search};

fn main() {
    for size in [1_000, 10_000, 25_000] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_entries(dir.path(), size);
        let iterations = if size < 10_000 { 20 } else { 5 };

        // Full-frame render (layout + journal/entry columns + markdown reader).
        draw_frame(&mut app, 120, 40);
        let started = Instant::now();
        for _ in 0..iterations {
            draw_frame(black_box(&mut app), 120, 40);
        }
        println!("render_frame/{size}: {:?}", started.elapsed() / iterations);

        // In-memory word search across every loaded entry.
        let _ = black_box(search(&app, "representative"));
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(search(black_box(&app), black_box("representative bold")));
        }
        println!("search/{size}: {:?}", started.elapsed() / iterations);
    }
}
