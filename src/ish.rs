//! iSH (iOS) support for the storage folder.
//!
//! iSH emulates a 32-bit Linux, so this is a `target_os = "linux"` build; iSH is
//! detected at runtime by its marker file `/proc/ish/version`. On iOS a real
//! folder is reachable only by mounting it with `mount -t ios . <path>`, which
//! pops the iOS Files picker. iSH does not re-mount on app restart, so we manage
//! it: the journal directory is a fixed mountpoint we (re)mount whenever it isn't
//! currently mounted. The picker does not remember the selected folder, so a
//! synced store id is checked against a device-local binding before any write.

use crate::AppResult;
use anyhow::{Context, bail};
use notema_storage::{JournalStore, LibraryDiscovery, LibraryLoadProgress, StoreId};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    thread,
    time::Duration,
};

const BINDING_FILE: &str = "ish-store.toml";
const BINDING_SCHEMA_VERSION: u32 = 1;

/// The journal directory on iSH: a fixed mountpoint the iOS folder is mounted
/// onto, so first-run needs no path prompt.
pub(crate) const DEFAULT_MOUNTPOINT: &str = "/mnt/Journals";

/// Whether we're running under iSH.
pub(crate) fn is_ish() -> bool {
    Path::new("/proc/ish/version").exists()
}

/// Whether `path` is currently a mount target, per `/proc/mounts`.
pub(crate) fn is_mounted(path: &Path) -> bool {
    let target = fs::canonicalize(path);
    let target = target.as_deref().unwrap_or(path);
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return false;
    };
    mounts.lines().any(|line| {
        // `/proc/mounts` fields: device mountpoint fstype …
        line.split_whitespace()
            .nth(1)
            .is_some_and(|mountpoint| Path::new(mountpoint) == target)
    })
}

/// On iSH, make sure the journal folder is mounted before the store is opened.
/// No-op off iSH or when already mounted. Otherwise create the mountpoint and run
/// `mount -t ios . <mountpoint>`, which opens the iOS Files picker and blocks
/// until the user selects a folder. Bails if the folder never appears (picker
/// cancelled), since an unmounted journal directory would silently read empty.
pub(crate) fn ensure_journal_mounted(mountpoint: &Path) -> AppResult<()> {
    if !is_ish() || is_mounted(mountpoint) {
        return Ok(());
    }

    fs::create_dir_all(mountpoint)?;
    println!(
        "Select the journal folder when the iOS picker opens (mounting at {})…",
        mountpoint.display()
    );

    let status = Command::new("mount")
        .args(["-t", "ios", "."])
        .arg(mountpoint)
        .status()?;

    // `mount` blocks until the picker resolves, but give the kernel a moment to
    // register the mount before we trust /proc/mounts.
    if !mounted_within(mountpoint, Duration::from_secs(3)) {
        if !status.success() {
            bail!(
                "mounting the journal folder failed (mount exited with {status}); relaunch and select the folder"
            );
        }
        bail!(
            "no journal folder was selected; relaunch and pick the folder your journals live in when the picker opens"
        );
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreBinding {
    schema_version: u32,
    store_id: StoreId,
}

/// Open a store only after the iSH mount has been identified. Off iSH this is
/// the normal construct-and-ensure path.
pub(crate) struct PreparedStore {
    pub(crate) store: JournalStore,
    pub(crate) discovery: Option<LibraryDiscovery>,
}

pub(crate) fn prepare_store(
    config_path: &Path,
    journal_root: &Path,
    allow_interactive_bind: bool,
) -> AppResult<PreparedStore> {
    if !is_ish() {
        let store = JournalStore::for_config(config_path, journal_root)?;
        store.ensure()?;
        return Ok(PreparedStore {
            store,
            discovery: None,
        });
    }

    ensure_journal_mounted(journal_root)?;
    let store = JournalStore::for_config(config_path, journal_root)?;
    let mounted_id = store.store_id()?;
    let binding = read_binding(config_path)?;
    let mut discovery = None;

    match (binding, mounted_id) {
        (Some(expected), Some(actual)) if expected == actual => store.ensure()?,
        (Some(expected), Some(actual)) => {
            bail!(
                "the selected iOS folder is a different Notema store (expected {expected}, found {actual}); unmount {}, relaunch, and select the configured journal folder",
                journal_root.display()
            );
        }
        (Some(expected), None) => {
            bail!(
                "the selected iOS folder has no Notema store marker (expected {expected}); nothing was changed. Unmount {}, relaunch, and select the configured journal folder",
                journal_root.display()
            );
        }
        (None, mounted_id) => {
            if !allow_interactive_bind {
                bail!(
                    "this iSH installation is not bound to a journal folder yet; launch `notema` once and confirm the selected folder"
                );
            }
            discovery = Some(confirm_binding(&store)?);
            let store_id = match mounted_id {
                Some(store_id) => {
                    store.ensure()?;
                    store_id
                }
                None => {
                    store.ensure()?;
                    store
                        .store_id()?
                        .context("journal store marker was not created")?
                }
            };
            write_binding(config_path, &store_id)?;
        }
    }

    Ok(PreparedStore { store, discovery })
}

fn binding_path(config_path: &Path) -> AppResult<PathBuf> {
    Ok(config_path
        .parent()
        .context("config path has no parent directory")?
        .join(BINDING_FILE))
}

fn read_binding(config_path: &Path) -> AppResult<Option<StoreId>> {
    let path = binding_path(config_path)?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let binding: StoreBinding =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if binding.schema_version != BINDING_SCHEMA_VERSION {
        bail!(
            "unsupported iSH store binding schema {} in {}",
            binding.schema_version,
            path.display()
        );
    }
    Ok(Some(binding.store_id))
}

fn write_binding(config_path: &Path, store_id: &StoreId) -> AppResult<()> {
    let binding = StoreBinding {
        schema_version: BINDING_SCHEMA_VERSION,
        store_id: store_id.clone(),
    };
    let encoded = toml::to_string_pretty(&binding)?;
    notema_encryption::atomic_write_private(&binding_path(config_path)?, encoded.as_bytes())?;
    Ok(())
}

fn confirm_binding(store: &JournalStore) -> AppResult<LibraryDiscovery> {
    let mut stdout = io::stdout();
    writeln!(stdout, "First setup: scanning selected folder.")?;
    writeln!(
        stdout,
        "This can take a long time on iSH; later starts are fast."
    )?;
    let progress = Mutex::new(InventoryProgress::new(&mut stdout));
    let inventory = store.discover_library_with_progress(&|update| {
        if let LibraryLoadProgress::Discovering { entries_found } = update {
            progress
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .update(entries_found);
        }
    });
    progress
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .finish()?;
    let discovery = inventory?;
    let journals = discovery.journal_names().collect::<Vec<_>>();
    let entries = discovery.entry_count();
    println!("Selected iOS folder:");
    if journals.is_empty() {
        println!("  Journals: none");
    } else {
        println!("  Journals: {}", journals.join(", "));
    }
    println!("  Entries: {entries}");
    print!("Use this folder for Notema on this iSH installation? [y/N]: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes" | "YES") {
        bail!("journal folder binding was cancelled; nothing was changed");
    }
    Ok(discovery)
}

struct InventoryProgress<'a, W: Write> {
    writer: &'a mut W,
    current: usize,
    last: usize,
    error: Option<io::Error>,
}

impl<'a, W: Write> InventoryProgress<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            current: 0,
            last: 0,
            error: None,
        }
    }

    fn update(&mut self, entries: usize) {
        self.current = entries;
        if self.error.is_some() || (entries != 0 && entries.saturating_sub(self.last) < 25) {
            return;
        }
        self.render();
    }

    fn render(&mut self) {
        self.last = self.current;
        if let Err(error) = write!(
            self.writer,
            "\rScanning selected folder… {} entries found",
            self.current
        )
        .and_then(|()| self.writer.flush())
        {
            self.error = Some(error);
        }
    }

    fn finish(mut self) -> io::Result<()> {
        if self.current != self.last {
            self.render();
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        writeln!(self.writer)
    }
}

/// Poll `/proc/mounts` until `path` shows up or `timeout` elapses.
fn mounted_within(path: &Path, timeout: Duration) -> bool {
    let deadline = Duration::from_millis(500);
    let mut waited = Duration::ZERO;
    loop {
        if is_mounted(path) {
            return true;
        }
        if waited >= timeout {
            return false;
        }
        thread::sleep(deadline);
        waited += deadline;
    }
}
