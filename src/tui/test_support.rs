//! Shared fixtures for the TUI test modules, so `App` construction and the
//! throwaway on-disk journals don't get re-implemented in every `mod tests`.

use std::fs;

use tempfile::tempdir;

use super::app::App;
use crate::config::Config;
use journal_storage::JournalStore;

/// Build an `App` over the given config's journal root (no entries loaded yet).
pub(crate) fn new_app(config: Config) -> App {
    let config_path = config.journal_root.join("config.toml");
    let store = JournalStore::for_config(&config_path, &config.journal_root).unwrap();
    App::new(config_path, config, store).unwrap()
}

/// An `App` over a temp root containing empty journals with the given names.
/// The temp dir is leaked so it outlives the returned `App`.
pub(crate) fn app_with_journals(names: &[&str]) -> App {
    let dir = tempdir().unwrap();
    for name in names {
        fs::create_dir_all(dir.path().join(name)).unwrap();
    }
    let config = Config::new(dir.path().to_path_buf(), "true");
    let app = new_app(config);
    std::mem::forget(dir);
    app
}

/// An `App` with a single `work` journal holding one entry (`a.md`), with the
/// `work` journal selected.
pub(crate) fn app_with_entry() -> App {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    std::mem::forget(dir);
    app
}

/// An `App` with a `work` journal holding `count` entries (`0.md`..), one minute
/// apart, with the `work` journal selected.
pub(crate) fn app_with_entries(count: usize) -> App {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    for index in 0..count {
        fs::write(
            entry_dir.join(format!("{index}.md")),
            format!(
                "+++\ncreated_at = \"2026-07-01T10:{index:02}:00+02:00\"\n+++\n\n# Entry {index}\nPreview {index}\n"
            ),
        )
        .unwrap();
    }
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    std::mem::forget(dir);
    app
}
