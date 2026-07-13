//! Bench-only entry points, compiled behind the `bench` feature and re-exported
//! from the crate root so `benches/` can reach the otherwise-private TUI paths.
//! Not part of the shipped binary.

use std::fs;
use std::path::Path;

use notema_domain::SearchScope;
use notema_storage::JournalStore;
use ratatui::{Terminal, backend::TestBackend};

use super::app::App;
use super::search::search_loaded_entries;
use crate::config::Config;

/// An opaque, fully-loaded app handle for benchmarks. Wraps the private `App` so
/// the bench API stays public without exposing the TUI's internal types.
pub struct BenchApp(App);

/// Build a [`BenchApp`] over a fresh on-disk store holding `count` plaintext
/// entries across four journals, with the first entry selected and the reader
/// focused so a draw exercises the markdown render path. `root` is a caller-owned
/// tempdir that must outlive the returned handle.
pub fn app_with_entries(root: &Path, count: usize) -> BenchApp {
    let journal_root = root.join("journals");
    let config_path = root.join("config.toml");
    let store = JournalStore::new(&journal_root, root);
    store.ensure().unwrap();

    for index in 0..count {
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

    let config = Config::new(root.to_path_buf());
    let mut app = App::new(config_path, config, store).unwrap();
    app.select_journal(0);
    app.select_entry_index(0);
    app.focus_reader_from_click();
    BenchApp(app)
}

/// Render one full frame to an in-memory [`TestBackend`] — the whole TUI draw
/// path (layout, journal/entry columns, markdown reader) with no real terminal.
pub fn draw_frame(app: &mut BenchApp, width: u16, height: u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| super::render::draw(frame, &mut app.0))
        .unwrap();
}

/// Word-search the loaded entries, returning the hit count.
pub fn search(app: &BenchApp, query: &str) -> usize {
    search_loaded_entries(&app.0.library.entries, query, &SearchScope::AllJournals).len()
}

fn entry_text(index: usize) -> String {
    format!(
        "+++\n\
         schema_version = 1\n\n\
         [entry]\n\
         tags = [\"tag-{}\", \"tag-{}\"]\n\
         people = [\"person-{}\"]\n\
         activities = [\"activity-{}\"]\n\
         mood = {}\n\n\
         [time]\n\
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
