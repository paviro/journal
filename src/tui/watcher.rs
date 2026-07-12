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
    pub(crate) fn start(root: &Path) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();

        let filter_root = root.to_path_buf();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    // Forward each changed path so the app can reload just the
                    // affected entries instead of re-reading the whole corpus.
                    for path in event
                        .paths
                        .into_iter()
                        .filter(|p| is_relevant(&filter_root, p))
                    {
                        let _ = tx.send(path);
                    }
                }
            },
            Config::default(),
        )?;

        watcher.watch(root, RecursiveMode::Recursive)?;

        Ok(Self {
            rx,
            _watcher: watcher,
        })
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

/// Hidden-file filter, applied only to the path *below* the watch root — a
/// root that itself lives under a dot directory (`~/.config/notema/themes`)
/// must still report its children. A path outside the root (e.g. notify
/// reporting a canonicalized form) falls back to the whole-path check rather
/// than being dropped.
fn is_relevant(root: &Path, path: &Path) -> bool {
    let below = path.strip_prefix(root).unwrap_or(path);
    !below
        .components()
        .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_filter_applies_below_the_watch_root_only() {
        let root = Path::new("/home/u/.config/notema/themes");
        assert!(is_relevant(root, &root.join("journal.toml")));
        assert!(!is_relevant(root, &root.join(".journal.toml.swp")));
        assert!(!is_relevant(
            root,
            &root.join(".hidden").join("journal.toml")
        ));
        // A path that doesn't strip against the root keeps the whole-path check.
        assert!(!is_relevant(root, Path::new("/elsewhere/.hidden/x.toml")));
        assert!(is_relevant(root, Path::new("/elsewhere/visible/x.toml")));
    }
}
