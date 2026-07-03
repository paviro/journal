use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::Path,
    sync::mpsc::{self, TryRecvError},
};

pub(crate) struct FileWatcher {
    rx: mpsc::Receiver<()>,
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    pub(crate) fn start(root: &Path) -> Self {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res
                    && event.paths.iter().any(|p| is_relevant(p))
                {
                    let _ = tx.send(());
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

    pub(crate) fn poll_change(&self) -> bool {
        match self.rx.try_recv() {
            Ok(()) => {
                while self.rx.try_recv().is_ok() {}
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => false,
        }
    }
}

fn is_relevant(path: &Path) -> bool {
    !path
        .components()
        .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.')))
}
