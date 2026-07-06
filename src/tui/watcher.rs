use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

pub(crate) struct FileWatcher {
    rx: mpsc::Receiver<PathBuf>,
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    pub(crate) fn start(root: &Path) -> Self {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    // Forward each changed path so the app can reload just the
                    // affected entries instead of re-reading the whole corpus.
                    for path in event.paths.into_iter().filter(|p| is_relevant(p)) {
                        let _ = tx.send(path);
                    }
                }
            },
            Config::default(),
        )
        .expect("init file watcher");

        watcher
            .watch(root, RecursiveMode::Recursive)
            .expect("watch journal root");

        Self {
            rx,
            _watcher: watcher,
        }
    }

    /// Drain and return every changed path seen since the last poll (empty when
    /// nothing changed).
    pub(crate) fn poll_changes(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        while let Ok(path) = self.rx.try_recv() {
            paths.push(path);
        }
        paths
    }
}

fn is_relevant(path: &Path) -> bool {
    !path
        .components()
        .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.')))
}
