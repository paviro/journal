//! Shared fixtures for the TUI test modules, so `App` construction and the
//! throwaway on-disk journals don't get re-implemented in every `mod tests`.

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use super::app::App;
use crate::config::{Config, State};
use notema_storage::JournalStore;

/// Build an `App` over the given config's journal root (no entries loaded yet).
pub(crate) fn new_app(config: Config) -> App {
    let config_path = config.journal.path.join("config.toml");
    let store = JournalStore::for_config(&config_path, &config.journal.path).unwrap();
    App::new(config_path, config, store).unwrap()
}

/// Like [`new_app`], but persists `state` to `state.toml` first so `App::new`
/// picks it up through the normal load path (e.g. to launch with journals hidden).
pub(crate) fn new_app_with_state(config: Config, state: State) -> App {
    let config_path = config.journal.path.join("config.toml");
    crate::config::save_state(&config_path, &state).unwrap();
    let store = JournalStore::for_config(&config_path, &config.journal.path).unwrap();
    App::new(config_path, config, store).unwrap()
}

/// Build an `App` over a fresh temp root, running `setup` to populate it first.
/// The temp dir is leaked so it outlives the returned `App`.
fn app_in_temp(setup: impl FnOnce(&Path)) -> App {
    let dir = tempdir().unwrap();
    setup(dir.path());
    let config = Config::new(dir.path().to_path_buf());
    let app = new_app(config);
    std::mem::forget(dir);
    app
}

/// An `App` over a temp root containing empty journals with the given names.
pub(crate) fn app_with_journals(names: &[&str]) -> App {
    app_in_temp(|root| {
        for name in names {
            fs::create_dir_all(root.join(name)).unwrap();
        }
    })
}

/// An `App` with a single `work` journal holding one entry (`a.md`), with the
/// `work` journal selected.
pub(crate) fn app_with_entry() -> App {
    let mut app = app_in_temp(|root| {
        let entry_dir = root.join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\nschema_version = 1\n[time]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();
    });
    app.select_journal_by_name("work");
    app
}

/// An `App` with a `work` journal holding `count` entries (`0.md`..), one minute
/// apart, with the `work` journal selected.
pub(crate) fn app_with_entries(count: usize) -> App {
    let mut app = app_in_temp(|root| {
        let entry_dir = root.join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        for index in 0..count {
            fs::write(
                entry_dir.join(format!("{index}.md")),
                format!(
                    "+++\nschema_version = 1\n[time]\ncreated_at = \"2026-07-01T10:{index:02}:00+02:00\"\n+++\n\n# Entry {index}\nPreview {index}\n"
                ),
            )
            .unwrap();
        }
    });
    app.select_journal_by_name("work");
    app
}
